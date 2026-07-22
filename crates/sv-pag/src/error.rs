//! 解析失败的原因。
//!
//! **本 crate 对任何输入都不 panic**(仓库 R4 去 panic 纪律):截断、畸形、
//! "声称的长度大于剩余字节"一律走这里返回 `Err`。索引全部先经边界检查,
//! 没有一处 `unwrap`/`expect`/切片直接下标越界的可能。
//!
//! 与 libpag 的差异要说清楚:libpag 的 `DecodeStream` 在越界时是
//! **记一条 exception、返回 0 继续往下跑**(`src/codec/utils/DecodeStream.cpp`,
//! 每个 `readXxx` 都是 `if (!checkEndOfFile(n)) { ... } return 0;`),
//! 调用方靠 `hasException()` 在循环边界补查。我们不复制这套 ——
//! 半路返回一堆零值再"事后补查"会让"解出来的东西"和"真实字节"悄悄脱钩,
//! 而这正是二进制解析器最难查的一类 bug。

/// PAG 容器解析失败的原因。
///
/// **没有 `Eq`**:`VerifyFailed` 带 `f32` 载荷(出问题的帧率),而 NaN != NaN ——
/// 给含浮点的类型实现 `Eq` 是在说谎。`PartialEq` 够用(测试里的 `assert_eq!`
/// 只要它)。
#[derive(Clone, Debug, PartialEq)]
pub enum PagError {
    /// 文件短于最小长度。
    ///
    /// 门槛 11 字节抄自 libpag `src/codec/Codec.cpp::ReadBodyBytes`
    /// (`if (stream->length() < 11)`)。9 字节是文件头本身
    /// (magic 3 + version 1 + bodyLength 4 + compression 1),
    /// 再加 2 字节是空的 body 也至少要有一个结束标签(`WriteEndTag` 写 `uint16 0`)。
    TooShort { len: usize },

    /// 头三个字节不是 `'P' 'A' 'G'`。
    BadMagic { found: [u8; 3] },

    /// 版本号 3 = 加密文件。libpag 开源版自己也解不了(企业版特性),
    /// 这里原样拒绝而不是尝试解析。
    Encrypted { version: u8 },

    /// 版本号大于 libpag 已知的最大版本(`KnownVersion = 3`)。
    UnsupportedVersion { version: u8 },

    /// 压缩标志不是 `'U'`(UNCOMPRESSED)。
    ///
    /// `CompressionAlgorithm` 里还定义了 `'Z'`(ZLIB)与 `'L'`(LZMA),
    /// 但 **libpag 自己的解码器也只接受 `'U'`** —— 另外两个是声明了没实现。
    /// 我们照样拒绝:压缩体的具体封装方式未核实,猜不得。
    UnsupportedCompression { code: u8 },

    /// 读到了缓冲区外面。
    UnexpectedEof {
        /// 出错时的字节游标
        at: usize,
        /// 本次想读多少字节
        need: usize,
        /// 实际还剩多少字节
        available: usize,
    },

    /// 标签头声称的体长度大于父流剩余字节。
    ///
    /// 这是二进制解析器最经典的越界/OOM 来源,单列一个变体方便定位。
    TagLengthOverflow {
        code: u16,
        length: u32,
        available: usize,
    },

    /// `BitmapSequence` 声称的帧数在剩余字节里放不下(见 `parse` 里的上界推导)。
    FrameCountTooLarge { count: u32, available: usize },

    /// 某一帧声称的 bitmap 块数在剩余字节里放不下。
    BitmapCountTooLarge { count: u32, available: usize },

    /// 位读的位数超出 `read_ubits` 的定义域(1..=32)。
    /// 只可能来自本 crate 内部调用错误,不由输入触发。
    BadBitCount { num_bits: u8 },

    /// 文件里一个 composition 都没有。
    ///
    /// 与 libpag 一致:`Codec::VerifyAndMake` 的第一句就是
    /// `bool success = !compositions.empty();`,空的直接返回 nullptr。
    NoCompositions,

    /// 文件通不过 libpag 的 `verify()` 门。
    ///
    /// libpag 在 `src/codec/Codec.cpp:150-176` 的 `VerifyAndMake` 里逐个
    /// composition 调 `verify()`,**任何一个不过就把整个文件拒掉**
    /// (delete 全部对象、返回 `nullptr`)。我们照做:放行 libpag 自己都拒的
    /// 文件没有任何好处 —— 那等于把负宽高 / 零帧率 / 空序列原样递给上层,
    /// 而上层拿 `seq.width` 去 `as usize` 开画布时 `-4i32` 会变成
    /// 18446744073709551612。**我们的"不 panic"不该是靠把 panic 转移出去实现的。**
    ///
    /// 具体是哪一条门槛见 [`VerifyFailure`]。
    VerifyFailed {
        /// 出问题的 composition 在文件里的下标(0 基,按文件出现顺序)
        composition: usize,
        reason: VerifyFailure,
    },
}

/// [`PagError::VerifyFailed`] 的具体原因,每一条都逐字对应 libpag 的一个
/// `verify()` 实现。
///
/// **有一条我们比 libpag 松,必须说清楚**:`VectorComposition::verify()`
/// (`src/base/VectorComposition.cpp`)还要求**每一个 `Layer` 都 verify 通过**,
/// 而本 crate 有意不解析 `LayerBlock` —— 那一层我们检不了。也就是说矢量档上
/// 我们的门比 libpag 松,可能放行 libpag 会拒的文件。这是"只解容器"这个范围
/// 裁剪的直接后果,不是遗漏。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VerifyFailure {
    /// composition 没有 `CompositionAttributes`(标签 3)。
    ///
    /// libpag 里这些字段的默认值是 0,于是 `Composition::verify()` 的
    /// `width > 0` 直接不成立 —— 效果等价于拒绝,我们单列一个变体好定位。
    MissingCompositionAttributes,

    /// `Composition::verify()`(`src/base/Composition.cpp:59`):
    /// `width > 0 && height > 0 && duration > 0 && frameRate > 0`。
    ///
    /// (libpag 同一处还有 `audioBytes != nullptr && length() == 0` 一条,
    /// 我们不解析 `AudioBytes`,检不了。)
    CompositionGeometry {
        width: i32,
        height: i32,
        duration_frames: u64,
        frame_rate: f32,
    },

    /// `BitmapComposition::verify()` / `VideoComposition::verify()`
    /// (`src/base/BitmapComposition.cpp`、`src/base/VideoComposition.cpp`)
    /// 都要求 `sequences` 非空 —— 一个不带任何序列的位图/视频 composition
    /// 在 libpag 里是无效文件。
    NoSequences,

    /// `Sequence::verify()`(`src/base/Sequence.cpp:40`):
    /// `composition != nullptr && width > 0 && height > 0 && frameRate > 0`。
    /// (`composition != nullptr` 在我们这里是结构性成立的,不存在孤儿序列。)
    SequenceGeometry {
        /// 序列在该 composition 的 `bitmap_sequences` 里的下标
        sequence: usize,
        width: i32,
        height: i32,
        frame_rate: f32,
    },

    /// `BitmapSequence::verify()`(`src/base/BitmapSequence.cpp:75`):
    /// `frames.empty()` 即失败。
    EmptyBitmapSequence { sequence: usize },

    /// `BitmapFrame::verify()`(`src/base/BitmapSequence.cpp:34-39`):
    /// 每个块都要 `fileBytes != nullptr`。
    ///
    /// 而 `DecodeStream::readByteData`(`src/codec/utils/DecodeStream.cpp:147`)
    /// 对 `length == 0` 返回的**正是** `nullptr` —— 所以**零长度块 = 整个文件
    /// 不合法**,不是什么"空帧"。写侧也印证:`WriteBitmapSequence`
    /// (`src/codec/tags/BitmapSequence.cpp:64-80`)在计数和写入两处都
    /// `if (bitmap->fileBytes->length() == 0) continue;` —— libpag **从不产出**
    /// 零长度块。
    ///
    /// (真正的"空帧"在 libpag 里长什么样见
    /// `BitmapSequence::isEmptyBitmapFrame`:导出器的一个 bug 会把它导成
    /// **1×1 的 WebP**,靠 `length() <= 150` + `WebPGetInfo` 宽高 ≤ 1 识别。)
    EmptyFrameBytes {
        sequence: usize,
        frame: usize,
        bitmap: usize,
    },
}

impl std::fmt::Display for VerifyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCompositionAttributes => {
                write!(f, "缺 CompositionAttributes(标签 3)")
            }
            Self::CompositionGeometry {
                width,
                height,
                duration_frames,
                frame_rate,
            } => write!(
                f,
                "composition 的 {width}x{height}、时长 {duration_frames} 帧、帧率 {frame_rate} 里有非正值(libpag 要求四项全 > 0)"
            ),
            Self::NoSequences => write!(f, "位图/视频 composition 里一条序列都没有"),
            Self::SequenceGeometry {
                sequence,
                width,
                height,
                frame_rate,
            } => write!(
                f,
                "第 {sequence} 条序列的 {width}x{height} @ {frame_rate}fps 里有非正值(libpag 要求三项全 > 0)"
            ),
            Self::EmptyBitmapSequence { sequence } => {
                write!(f, "第 {sequence} 条位图序列一帧都没有")
            }
            Self::EmptyFrameBytes {
                sequence,
                frame,
                bitmap,
            } => write!(
                f,
                "第 {sequence} 条序列第 {frame} 帧的第 {bitmap} 个块是零长度(libpag 的 readByteData 对此返回 nullptr,整个文件会被拒)"
            ),
        }
    }
}

impl std::fmt::Display for PagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { len } => {
                write!(f, "不是 PAG 文件:长度 {len} 字节,不足最小的 11 字节")
            }
            Self::BadMagic { found } => write!(
                f,
                "不是 PAG 文件:头三字节是 {found:02X?},应为 'P' 'A' 'G'(50 41 47)"
            ),
            Self::Encrypted { version } => {
                write!(f, "PAG 文件已加密(版本号 {version}),开源解码器无法读取")
            }
            Self::UnsupportedVersion { version } => {
                write!(
                    f,
                    "PAG 版本号 {version} 超出已知范围(libpag KnownVersion = 3)"
                )
            }
            Self::UnsupportedCompression { code } => write!(
                f,
                "PAG 压缩标志 {code:#04X} 不是 'U'(未压缩);ZLIB/LZMA 体的封装方式未核实,拒绝猜测"
            ),
            Self::UnexpectedEof {
                at,
                need,
                available,
            } => write!(
                f,
                "PAG 数据在偏移 {at} 处截断:需要 {need} 字节,只剩 {available} 字节"
            ),
            Self::TagLengthOverflow {
                code,
                length,
                available,
            } => write!(
                f,
                "PAG 标签 {code} 声称体长 {length} 字节,但父流只剩 {available} 字节"
            ),
            Self::FrameCountTooLarge { count, available } => write!(
                f,
                "BitmapSequence 声称 {count} 帧,{available} 字节装不下(每帧至少要 1 bit + 1 字节)"
            ),
            Self::BitmapCountTooLarge { count, available } => write!(
                f,
                "BitmapFrame 声称 {count} 个 bitmap 块,{available} 字节装不下(每块至少 3 字节)"
            ),
            Self::BadBitCount { num_bits } => {
                write!(f, "内部错误:位读宽度 {num_bits} 超出 1..=32")
            }
            Self::NoCompositions => {
                write!(f, "PAG 文件里没有任何 composition")
            }
            Self::VerifyFailed {
                composition,
                reason,
            } => write!(
                f,
                "PAG 文件通不过 libpag 的 verify():第 {composition} 个 composition {reason}"
            ),
        }
    }
}

impl std::error::Error for PagError {}
