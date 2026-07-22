//! 解析驱动:文件头 → 标签流 → composition → 位图序列。
//!
//! 结构上**没有递归**:文件级标签循环里遇到 composition 块就进 composition
//! 级标签循环,composition 级不再下钻到任何带子标签列表的块(比如 `LayerBlock`)。
//! 于是嵌套深度恒为 2,不需要深度计数器也不可能爆栈 —— 这不是运气,
//! 是"只解容器、不解矢量模型"这个范围裁剪带来的直接好处。

use crate::error::{PagError, VerifyFailure};
use crate::model::{
    BitmapFrame, BitmapRect, BitmapSequence, Composition, CompositionAttributes, CompositionKind,
    PagFile,
};
use crate::reader::Reader;
use crate::tag::{self, read_tag_header};

/// 文件头魔数。
const MAGIC: [u8; 3] = *b"PAG";
/// 版本 3 = 加密档(libpag `Codec.cpp` 的 `EncryptedVersion`)
const ENCRYPTED_VERSION: u8 = 3;
/// 已知最大版本(同上的 `KnownVersion`)
const KNOWN_VERSION: u8 = 3;
/// `CompressionAlgorithm::UNCOMPRESSED`,字面量就是字符 `'U'`
const COMPRESSION_UNCOMPRESSED: u8 = b'U';
/// libpag 的最小长度门槛,见 `PagError::TooShort` 的说明
const MIN_FILE_LEN: usize = 11;

pub(crate) fn parse(bytes: &[u8]) -> Result<PagFile<'_>, PagError> {
    if bytes.len() < MIN_FILE_LEN {
        return Err(PagError::TooShort { len: bytes.len() });
    }
    let mut r = Reader::new(bytes);

    // 文件头,逐字对应 libpag `src/codec/Codec.cpp::ReadBodyBytes`:
    //   'P' 'A' 'G' | version:u8 | bodyLength:u32 小端 | compression:u8
    let magic = [r.read_u8()?, r.read_u8()?, r.read_u8()?];
    if magic != MAGIC {
        return Err(PagError::BadMagic { found: magic });
    }
    let version = r.read_u8()?;
    // 顺序不能换:version == 3 要报"加密"而不是"版本不支持",
    // libpag 也是先判 EncryptedVersion 再判 > KnownVersion。
    if version == ENCRYPTED_VERSION {
        return Err(PagError::Encrypted { version });
    }
    if version > KNOWN_VERSION {
        return Err(PagError::UnsupportedVersion { version });
    }
    let body_length = r.read_u32()?;
    let compression = r.read_u8()?;
    if compression != COMPRESSION_UNCOMPRESSED {
        return Err(PagError::UnsupportedCompression { code: compression });
    }

    // `bodyLength = std::min(bodyLength, stream->bytesAvailable());`
    // (Codec.cpp)。声称长度**大于**实际剩余时 libpag 不报错,按剩余截断。
    // 这里照抄:报错会拒掉 libpag 能打开的文件,而截断后真正读越界的地方
    // 自然会在下面的标签循环里报出来。
    let body_length = (body_length as usize).min(r.remaining());
    let body = r.read_bytes(body_length)?;

    let compositions = parse_file_tags(body)?;
    if compositions.is_empty() {
        return Err(PagError::NoCompositions);
    }
    verify(&compositions)?;
    Ok(PagFile {
        version,
        compositions,
    })
}

/// libpag 的 `verify()` 门,对应 `src/codec/Codec.cpp:150-176` 的
/// `Codec::VerifyAndMake`:逐个 composition 调 `verify()`,**任何一个不过就把
/// 整个文件拒掉**(它 delete 全部对象并返回 `nullptr`,`Codec::Decode` 于是
/// 也返回 `nullptr`)。
///
/// 为什么要有这一步:没有它,一个 `width = -4 / height = 0 / frameRate = 0 /
/// frames = []` 的序列会被我们**原样交给上层**,而 README 让上层拿
/// `seq.width × seq.height` 去开画布 —— `-4i32 as usize` 是
/// 18446744073709551612。libpag 恰恰在这一层就把文件拒了,我们把这条门省掉
/// 等于把 panic 转移给调用方,而不是消灭它。
///
/// 逐条出处(全部读过原文,不是转述):
///
/// | 规则 | 出处 |
/// |---|---|
/// | `width > 0 && height > 0 && duration > 0 && frameRate > 0` | `src/base/Composition.cpp:59` |
/// | 位图/视频 composition 的 `sequences` 非空 | `src/base/BitmapComposition.cpp`、`src/base/VideoComposition.cpp` |
/// | 序列 `width > 0 && height > 0 && frameRate > 0` | `src/base/Sequence.cpp:40` |
/// | 位图序列 `frames` 非空 | `src/base/BitmapSequence.cpp:75` |
/// | 每个块 `fileBytes != nullptr`(= 长度非零) | `src/base/BitmapSequence.cpp:34-39` + `src/codec/utils/DecodeStream.cpp:147` |
///
/// **两处我们检不了,如实记在这里**:
/// 1. `VectorComposition::verify()` 还要求每个 `Layer` 都 verify 通过 ——
///    我们不解析 `LayerBlock`,矢量档上这道门比 libpag 松;
/// 2. `Composition::verify()` 的 `audioBytes` 非空长度检查 —— 我们不解析
///    `AudioBytes`。
///
/// 反过来没有"比 libpag 严"的地方:这里每一条都是 libpag 原样的子集,
/// 不会拒掉 libpag 打得开的文件。
fn verify(compositions: &[Composition<'_>]) -> Result<(), PagError> {
    for (index, comp) in compositions.iter().enumerate() {
        let fail = |reason| {
            Err(PagError::VerifyFailed {
                composition: index,
                reason,
            })
        };

        // Composition::verify()
        let Some(a) = comp.attributes else {
            return fail(VerifyFailure::MissingCompositionAttributes);
        };
        // 注意 NaN:`NaN > 0.0` 是 false,于是 NaN 帧率会被拒 —— 与 C++ 的
        // `frameRate > 0` 行为一致,这是想要的。
        if !(a.width > 0 && a.height > 0 && a.duration_frames > 0 && a.frame_rate > 0.0) {
            return fail(VerifyFailure::CompositionGeometry {
                width: a.width,
                height: a.height,
                duration_frames: a.duration_frames,
                frame_rate: a.frame_rate,
            });
        }

        match comp.kind {
            // VectorComposition::verify():除 Composition::verify() 外只查
            // 每个 Layer —— 而 Layer 我们不解析。
            CompositionKind::Vector => {}
            // VideoComposition::verify():sequences 非空 + 每条序列 verify。
            // VideoSequence 的字段布局我们未核实(只计数),所以只能检非空这一半。
            CompositionKind::Video => {
                if comp.video_sequence_count == 0 {
                    return fail(VerifyFailure::NoSequences);
                }
            }
            CompositionKind::Bitmap => {
                if comp.bitmap_sequences.is_empty() {
                    return fail(VerifyFailure::NoSequences);
                }
                for (si, seq) in comp.bitmap_sequences.iter().enumerate() {
                    // Sequence::verify()
                    if !(seq.width > 0 && seq.height > 0 && seq.frame_rate > 0.0) {
                        return fail(VerifyFailure::SequenceGeometry {
                            sequence: si,
                            width: seq.width,
                            height: seq.height,
                            frame_rate: seq.frame_rate,
                        });
                    }
                    // BitmapSequence::verify()
                    if seq.frames.is_empty() {
                        return fail(VerifyFailure::EmptyBitmapSequence { sequence: si });
                    }
                    // BitmapFrame::verify()
                    for (fi, frame) in seq.frames.iter().enumerate() {
                        for (bi, bitmap) in frame.bitmaps.iter().enumerate() {
                            if bitmap.bytes.is_empty() {
                                return fail(VerifyFailure::EmptyFrameBytes {
                                    sequence: si,
                                    frame: fi,
                                    bitmap: bi,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// 通用标签循环,对应 libpag `src/codec/TagHeader.h` 的模板函数 `ReadTags`:
/// 读头 → 切出 `length` 字节的子流 → 交给 `on_tag` → 直到 `End`。
///
/// **子流是隔离的**:每个标签体在自己的 `Reader` 里解析,位游标也从 0 开始。
/// 这一点对 `BitmapSequence` 的 bit-packed 字段是必要的。
fn read_tags<'a, F>(r: &mut Reader<'a>, mut on_tag: F) -> Result<(), PagError>
where
    // 标签体的生命周期必须是**输入缓冲区**的 `'a`,不能是闭包体的匿名生命周期 ——
    // 否则借出去的帧字节活不过这个闭包,零拷贝就无从谈起。
    F: FnMut(u16, &'a [u8]) -> Result<(), PagError>,
{
    loop {
        let header = read_tag_header(r)?;
        if header.code == tag::END {
            return Ok(());
        }
        let len = header.length as usize;
        // **越界/OOM 的头号入口**:标签头是攻击者可控的 u32,
        // 单独报一个变体,别让它掉进泛化的 EOF 里。
        if len > r.remaining() {
            return Err(PagError::TagLengthOverflow {
                code: header.code,
                length: header.length,
                available: r.remaining(),
            });
        }
        let body = r.read_bytes(len)?;
        on_tag(header.code, body)?;
    }
}

fn parse_file_tags<'a>(body: &'a [u8]) -> Result<Vec<Composition<'a>>, PagError> {
    let mut r = Reader::new(body);
    let mut out = Vec::new();
    read_tags(&mut r, |code, tag_body| {
        // 只认三种 composition 块。其余(FontTables / ImageBytes* / FileAttributes /
        // EditableIndices / …)一律**按长度跳过** —— 它们的体布局本 crate 未核实,
        // 猜着解等于编格式。
        let kind = match code {
            tag::VECTOR_COMPOSITION_BLOCK => CompositionKind::Vector,
            tag::BITMAP_COMPOSITION_BLOCK => CompositionKind::Bitmap,
            tag::VIDEO_COMPOSITION_BLOCK => CompositionKind::Video,
            _ => return Ok(()),
        };
        out.push(parse_composition(tag_body, kind)?);
        Ok(())
    })?;
    Ok(out)
}

fn parse_composition<'a>(
    body: &'a [u8],
    kind: CompositionKind,
) -> Result<Composition<'a>, PagError> {
    let mut r = Reader::new(body);
    // 三种块体都以 `id: EncodedUint32` 开头
    // (Vector/Bitmap/VideoCompositionTag.cpp 的 `composition->id = stream->readEncodedUint32();`)
    let id = r.read_encoded_u32()?;
    // **只有** VideoCompositionBlock 在 id 之后多一个整字节的 Boolean:
    // `auto hasAlpha = stream->readBoolean();`(VideoCompositionTag.cpp)。
    // 漏掉它会让视频档的整个标签流错位一个字节。
    let has_alpha = match kind {
        CompositionKind::Video => Some(r.read_bool()?),
        _ => None,
    };

    let mut attributes = None;
    let mut bitmap_sequences = Vec::new();
    let mut video_sequence_count = 0usize;

    read_tags(&mut r, |code, tag_body| {
        match code {
            tag::COMPOSITION_ATTRIBUTES => {
                attributes = Some(parse_composition_attributes(tag_body)?);
            }
            tag::BITMAP_SEQUENCE => {
                bitmap_sequences.push(parse_bitmap_sequence(tag_body)?);
            }
            // 视频序列内部是 H.264 NAL,字段布局未核实 —— 只计数,不解析。
            tag::VIDEO_SEQUENCE => video_sequence_count += 1,
            // LayerBlock / AudioBytes / MarkerList / Mp4Header … 一律跳过
            _ => {}
        }
        Ok(())
    })?;

    Ok(Composition {
        id,
        kind,
        attributes,
        bitmap_sequences,
        video_sequence_count,
        has_alpha,
    })
}

/// `CompositionAttributes`(标签 3),对应
/// `src/codec/tags/CompositionAttributes.cpp::ReadCompositionAttributes`:
///
/// ```text
/// width           : EncodedInt32
/// height          : EncodedInt32
/// duration        : EncodedUint64   (ReadTime,单位是帧)
/// frameRate       : float32 小端
/// backgroundColor : uint8 R, uint8 G, uint8 B   (ReadColor,没有 alpha)
/// ```
fn parse_composition_attributes(body: &[u8]) -> Result<CompositionAttributes, PagError> {
    let mut r = Reader::new(body);
    let width = r.read_encoded_i32()?;
    let height = r.read_encoded_i32()?;
    let duration_frames = r.read_encoded_u64()?;
    let frame_rate = r.read_f32()?;
    let background_color = [r.read_u8()?, r.read_u8()?, r.read_u8()?];
    Ok(CompositionAttributes {
        width,
        height,
        duration_frames,
        frame_rate,
        background_color,
    })
}

/// 预留容量的上限。**声称的**元素个数不能直接换成一次大分配 ——
/// 超过这个数就让 `Vec` 自己按需增长(均摊 O(1),且规模跟着**真读到的**字节走,
/// 而不是跟着攻击者写在 varint 里的数字走)。4096 个元素 = 128 KB 的 frames,
/// 对真实素材而言一次到位,对畸形输入而言无关痛痒。
const RESERVE_CAP: usize = 4096;

fn reserve(claimed: u32) -> usize {
    (claimed as usize).min(RESERVE_CAP)
}

/// `BitmapSequence`(标签 46),对应
/// `src/codec/tags/BitmapSequence.cpp::ReadBitmapSequence`:
///
/// ```text
/// width      : EncodedInt32
/// height     : EncodedInt32
/// frameRate  : float32 小端
/// count      : EncodedUint32
/// 第一趟: count 个 **bit**(isKeyframe),字节内低位在前
/// ——— 位读结束后字节游标向上取整到下一个字节边界 ———
/// 第二趟: 对每一帧
///           bitmapCount : EncodedUint32
///           对每个块      x : EncodedInt32
///                        y : EncodedInt32
///                        fileBytes : EncodedUint32 长度 + 该长度的原始字节
/// ```
///
/// **两趟是分开的**:所有帧的 isKeyframe 先连着写完,才轮到各帧的块数据。
/// 按"每帧读一个 bit 再读它的块"那样写会全盘错位。
fn parse_bitmap_sequence<'a>(body: &'a [u8]) -> Result<BitmapSequence<'a>, PagError> {
    let mut r = Reader::new(body);
    let width = r.read_encoded_i32()?;
    let height = r.read_encoded_i32()?;
    let frame_rate = r.read_f32()?;
    let count = r.read_encoded_u32()?;

    // ---- 硬化:libpag **没有**这一步,我们加的 ----
    // count 是攻击者可控的 varint,上限 u32::MAX。libpag 靠"每次循环查一遍
    // hasException()"来收场,但它在查之前已经 push 了一个新对象 ——
    // 也就是说畸形 count 会先让它分配一大批对象再退出。
    // 我们在分配前先卡住:结构上每帧至少要花掉
    //   第一趟 1 bit(isKeyframe)+ 第二趟 1 字节(bitmapCount 的 varint 最短 1 字节)
    // 即 ceil(count/8) + count 字节。剩余字节装不下就直接拒绝。
    // 这样 frames 的规模恒被输入长度**线性**压住。
    //
    // **但线性 ≠ 常数小,常数在这里如实写出来:约 32×。**
    //   size_of::<BitmapFrame>() == 32  (bool + Vec 的 24 字节)
    //   size_of::<BitmapRect>()  == 24  (i32 + i32 + &[u8] 胖指针)
    // 上面的不等式只把 count 压到 ≤ 剩余**字节数**,于是一个 N 字节的标签体
    // 最多能驱动 32N 字节的 frames(每帧再叠 24 字节/块)。10 MB 的构造 `.pag`
    // 因此能在第一个 EOF 之前吃掉几百 MB。
    //
    // 我们**不再往下加绝对帧数上界** —— 那要么拍一个没有依据的数字,要么冒
    // 拒掉合法长动画的风险,两样都比"如实披露"差。能做且做了的是下面这条:
    // 预留容量按 RESERVE_CAP 封顶,**声称的**数字换不来一次大分配;真要长到
    // 那么大,得先逐字节拿出对应的真实数据,Vec 均摊增长跟着实读走。
    let need = u64::from(count).div_ceil(8) + u64::from(count);
    if need > r.remaining() as u64 {
        return Err(PagError::FrameCountTooLarge {
            count,
            available: r.remaining(),
        });
    }

    // 第一趟:连着 count 个 bit
    let mut frames: Vec<BitmapFrame<'_>> = Vec::with_capacity(reserve(count));
    for _ in 0..count {
        frames.push(BitmapFrame {
            is_keyframe: r.read_bit_bool()?,
            bitmaps: Vec::new(),
        });
    }

    // 第二趟:每帧的块。注意这里**依赖上面位读把字节游标向上取整**,
    // 否则第一个 bitmapCount 会从半个字节里读出来。
    for frame in &mut frames {
        let bitmap_count = r.read_encoded_u32()?;
        // 同款硬化:每个块至少 3 字节(x/y/长度三个 varint 各至少 1 字节)
        if u64::from(bitmap_count) * 3 > r.remaining() as u64 {
            return Err(PagError::BitmapCountTooLarge {
                count: bitmap_count,
                available: r.remaining(),
            });
        }
        frame.bitmaps.reserve(reserve(bitmap_count));
        for _ in 0..bitmap_count {
            let x = r.read_encoded_i32()?;
            let y = r.read_encoded_i32()?;
            let bytes = r.read_byte_data()?;
            frame.bitmaps.push(BitmapRect { x, y, bytes });
        }
    }

    Ok(BitmapSequence {
        width,
        height,
        frame_rate,
        frames,
    })
}
