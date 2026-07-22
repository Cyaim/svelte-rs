//! 字节流读取器 —— 对照移植自 libpag `src/codec/utils/DecodeStream.{h,cpp}`。
//!
//! 每一条读取语义都能指到 libpag 的源码行,出处见 `README.md` 的核实表。
//! 这里只把**为什么这么写**记下来。

use crate::error::PagError;

/// PAG 的读游标。
///
/// 三条容易踩的坑,按重要性排:
///
/// 1. **字节序恒为小端。** `DecodeStream.h` 的类注释原文:
///    "The byte order of DecodeStream is always little-endian."
///    PAG 里没有任何一处是大端。
///
/// 2. **字节游标与位游标是两个变量,而且互相同步。** libpag 用
///    `positionChanged` / `bitPositionChanged` 两个私有方法维持不变式:
///    - 字节读之后:`_bitPosition = _position * 8`(位游标**对齐回**字节边界);
///    - 位读之后:`_position = BitsToBytes(_bitPosition)`,而
///      `BitsToBytes(c) = ceil(c * 0.125)`(`src/codec/utils/StreamContext.h`),
///      也就是字节游标**向上取整**到下一个字节边界。
///
///    "位读把字节游标向上取整"这一条就是 `BitmapSequence` 能先 bit-packed
///    地写一串 `isKeyframe`、再接着按字节读的原因。**写错这一处,
///    序列帧的偏移会整体错位,而且错得很隐蔽** —— 帧数 ≤ 8 时读出来完全正常
///    (1 个字节装得下),帧数一超过 8 才开始崩,是那种在最小测试用例上
///    永远复现不出来的 bug。所以 `bit_align_rounds_byte_cursor_up` 那条测试
///    是特意用 9 帧写的。
///
/// 3. **越界一律返回 `Err`,不返回零值。** libpag 是"记 exception + 返回 0
///    继续跑",调用方在循环边界补查 `hasException()`。我们不复制,理由见
///    `error.rs` 顶部。
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    /// 字节游标,对应 `DecodeStream::_position`
    pos: usize,
    /// 位游标,对应 `DecodeStream::_bitPosition`
    bit_pos: u64,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            bit_pos: 0,
        }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn need(&self, n: usize) -> Result<(), PagError> {
        if self.remaining() < n {
            return Err(PagError::UnexpectedEof {
                at: self.pos,
                need: n,
                available: self.remaining(),
            });
        }
        Ok(())
    }

    /// 对应 `DecodeStream::positionChanged`:推进字节游标,并把位游标拉回字节边界。
    fn advance(&mut self, n: usize) {
        self.pos += n;
        self.bit_pos = self.pos as u64 * 8;
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8, PagError> {
        self.need(1)?;
        let v = self.bytes[self.pos];
        self.advance(1);
        Ok(v)
    }

    /// `readBoolean` 读**整整一个字节**(不是一个 bit),非零即真。
    /// 别和 `read_bit_bool` 搞混:`VideoCompositionBlock` 的 `hasAlpha` 走这个,
    /// `BitmapSequence` 的 `isKeyframe` 走那个。
    pub(crate) fn read_bool(&mut self) -> Result<bool, PagError> {
        Ok(self.read_u8()? != 0)
    }

    pub(crate) fn read_u16(&mut self) -> Result<u16, PagError> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.advance(2);
        Ok(v)
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32, PagError> {
        self.need(4)?;
        let b = &self.bytes[self.pos..self.pos + 4];
        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        self.advance(4);
        Ok(v)
    }

    pub(crate) fn read_f32(&mut self) -> Result<f32, PagError> {
        self.need(4)?;
        let b = &self.bytes[self.pos..self.pos + 4];
        let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        self.advance(4);
        Ok(v)
    }

    /// 借出接下来的 `n` 字节(零拷贝),并推进游标。
    pub(crate) fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], PagError> {
        self.need(n)?;
        let out = &self.bytes[self.pos..self.pos + n];
        self.advance(n);
        Ok(out)
    }

    /// LEB128 变长无符号整数:每字节低 7 位是数据(**低位在前**),
    /// 最高位是"还有下一字节"。
    ///
    /// **上限 5 组(移位 0/7/14/21/28)**,逐字对应 libpag 的
    /// `for (int i = 0; i < 32; i += 7)`。第 5 组仍带续位时 libpag
    /// **不报错**,循环自然结束、高位被丢掉,游标恰好推进 5 字节。
    /// 我们照抄这个行为:报错会让我们拒掉 libpag 能打开的文件,
    /// 而照抄能保证"消耗的字节数"与参考实现逐字节一致 —— 对容器解析器来说,
    /// **游标不跑偏比拒绝畸形值更重要**(值本身的合理性由上层的上界检查兜)。
    pub(crate) fn read_encoded_u32(&mut self) -> Result<u32, PagError> {
        let mut value: u32 = 0;
        let mut shift: u32 = 0;
        while shift < 32 {
            let byte = self.read_u8()?;
            value |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(value)
    }

    /// 变长有符号整数。**不是标准 zigzag**:libpag 的
    /// `readEncodedInt32` 是 `value = data >> 1; return (data & 1) ? -value : value;`
    /// —— 即**最低位是符号位(1 表示负)**,其余位是绝对值。
    ///
    /// 标准 zigzag 是 `(n >> 1) ^ -(n & 1)`,两者在负数上结果不同
    /// (例:data = 3,PAG 解出 -1,zigzag 解出 -2)。照 zigzag 写会静默解错。
    pub(crate) fn read_encoded_i32(&mut self) -> Result<i32, PagError> {
        let data = self.read_encoded_u32()?;
        // data >> 1 最大 0x7FFF_FFFF,转 i32 与取负都不会溢出
        let value = (data >> 1) as i32;
        Ok(if data & 1 > 0 { -value } else { value })
    }

    /// 64 位变长无符号整数,上限 10 组(移位 0/7/…/63),
    /// 对应 libpag 的 `for (int i = 0; i < 64; i += 7)`。
    /// `CompositionAttributes` 的 duration(`ReadTime`)走这个。
    pub(crate) fn read_encoded_u64(&mut self) -> Result<u64, PagError> {
        let mut value: u64 = 0;
        let mut shift: u32 = 0;
        while shift < 64 {
            let byte = self.read_u8()?;
            value |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(value)
    }

    /// 按位读,**字节内低位在前**(对应 `readUBits` 里的
    /// `byte = bytes[bytePosition] >> bitPosition`)。
    ///
    /// 边界条件抄 libpag:要求 `bit_pos + num_bits <= 总位数`。
    fn read_ubits(&mut self, num_bits: u8) -> Result<u32, PagError> {
        if num_bits == 0 || num_bits > 32 {
            return Err(PagError::BadBitCount { num_bits });
        }
        let total_bits = self.bytes.len() as u64 * 8;
        if self.bit_pos + u64::from(num_bits) > total_bits {
            return Err(PagError::UnexpectedEof {
                at: self.pos,
                need: u64::from(num_bits).div_ceil(8) as usize,
                available: self.remaining(),
            });
        }
        const BIT_MASKS: [u8; 9] = [0, 1, 3, 7, 15, 31, 63, 127, 255];
        let mut value: u32 = 0;
        let mut done: u8 = 0;
        while done < num_bits {
            let byte_pos = (self.bit_pos / 8) as usize;
            let bit_off = (self.bit_pos % 8) as u8;
            let take = (8 - bit_off).min(num_bits - done);
            let chunk = (self.bytes[byte_pos] >> bit_off) & BIT_MASKS[take as usize];
            value |= u32::from(chunk) << done;
            done += take;
            self.bit_pos += u64::from(take);
        }
        // 对应 bitPositionChanged + BitsToBytes:字节游标向上取整
        self.pos = self.bit_pos.div_ceil(8) as usize;
        Ok(value)
    }

    /// `readBitBoolean()` = `readUBits(1) != 0`(`DecodeStream.h` 内联定义)。
    pub(crate) fn read_bit_bool(&mut self) -> Result<bool, PagError> {
        Ok(self.read_ubits(1)? != 0)
    }

    /// `readByteData`:先一个变长长度,再借出那么多字节(零拷贝)。
    ///
    /// libpag 在 `length == 0` 或 `length > 可用` 时都返回 `nullptr`。
    /// 我们把两种情况分开:长度 0 返回**空切片**(`BitmapSequenceReader.cpp`
    /// 的注释说这代表"空帧",是合法数据,不是错误),
    /// 长度超出剩余才报 `UnexpectedEof`。
    pub(crate) fn read_byte_data(&mut self) -> Result<&'a [u8], PagError> {
        let len = self.read_encoded_u32()? as usize;
        self.read_bytes(len)
    }
}
