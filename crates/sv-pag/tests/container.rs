//! 容器解析的固件测试。
//!
//! **固件全部是按 libpag 源码手工构造的字节序列,仓库里不存任何 `.pag` 二进制资产。**
//! 每个构造函数都注明它对应规范的哪一条(源码文件 + 函数),
//! `minimal_bitmap_file` 里还逐字段标了每个字节是什么。
//!
//! 🔴 **这些固件证明的是"解析器与我们读到的 libpag 源码一致",
//! 不是"解析器能读真文件"。** 见 README 的头号缺口。

use sv_pag::{CompositionKind, FileKind, ImageEncoding, PagError, PagFile, VerifyFailure};

// ---------------------------------------------------------------------------
// 固件构造工具:按 libpag 的**编码**侧写,和解析器的读侧互为独立实现
// ---------------------------------------------------------------------------

/// 变长无符号整数,对应 `EncodeStream::writeEncodedUint32`:
/// 每字节低 7 位数据(低位在前),高位是续位标志。
fn enc_u32(mut v: u32) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            return out;
        }
    }
}

/// 变长 64 位无符号整数(`ReadTime` 用的那个)。
fn enc_u64(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            return out;
        }
    }
}

/// 变长有符号整数,对应 `readEncodedInt32` 的逆:
/// **最低位是符号位(1 = 负)**,其余位是绝对值 —— 不是标准 zigzag。
///
/// **定义域是 `i32::MIN + 1 ..= i32::MAX`,`i32::MIN` 编不出来。**
/// 这不是我们的偷懒:符号-绝对值编码里 |i32::MIN| = 2^31 已经溢出 i32 的正半轴,
/// 读侧 `(data >> 1) as i32` 最大只能给出 `i32::MAX`(data = 0xFFFF_FFFE 时),
/// 取负最多到 `-i32::MAX`(data = 0xFFFF_FFFF)。也就是说 **`i32::MIN` 根本不在
/// `EncodedInt32` 的值域里**,libpag 自己也编不出来。
fn enc_i32(v: i32) -> Vec<u8> {
    assert_ne!(
        v,
        i32::MIN,
        "i32::MIN 不在 EncodedInt32 的值域内(符号-绝对值编码,|MIN| 溢出)"
    );
    let mag = v.unsigned_abs();
    let sign = u32::from(v < 0);
    enc_u32((mag << 1) | sign)
}

/// 一个完整标签,对应 `TagHeader.cpp::WriteTypeAndLength`:
/// `uint16 小端 = (code << 6) | min(length, 63)`;length ≥ 63 时再跟一个 uint32。
fn tag(code: u16, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let len = body.len() as u32;
    if len < 63 {
        out.extend_from_slice(&((code << 6) | len as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&((code << 6) | 63).to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
    }
    out.extend_from_slice(body);
    out
}

/// 结束标签 = `uint16 0`(`WriteEndTag`)。
fn end_tag() -> Vec<u8> {
    vec![0x00, 0x00]
}

/// 套上文件头,对应 `Codec::Encode`:
/// `'P' 'A' 'G' | version:u8 | bodyLength:u32 小端 | 'U'`
fn wrap(version: u8, compression: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"PAG");
    out.push(version);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.push(compression);
    out.extend_from_slice(body);
    out
}

/// 12 字节的最小 WebP 头:`"RIFF" + 长度 + "WEBP"`。
/// (`WebpDecoder.h`:`RIFF_HEADER_SIZE 12  // "RIFFnnnnWEBP"`)
/// **不是能解码的真 WebP**,只是让编码嗅探认得出来。
fn fake_webp() -> Vec<u8> {
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend_from_slice(b"WEBP");
    v
}

/// PNG 的 8 字节签名。
fn png_sig() -> Vec<u8> {
    vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
}

// ---------------------------------------------------------------------------
// 主固件:一个最小的「位图序列帧」档
// ---------------------------------------------------------------------------

/// `CompositionAttributes`(标签 3)体,对应 `ReadCompositionAttributes`:
/// width:EncInt32 / height:EncInt32 / duration:EncUint64 / frameRate:f32 / RGB:3×u8
fn comp_attrs_body(w: i32, h: i32, duration: u64, fps: f32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(enc_i32(w));
    b.extend(enc_i32(h));
    b.extend(enc_u64(duration));
    b.extend_from_slice(&fps.to_le_bytes());
    b.extend_from_slice(&[0x00, 0x00, 0x00]); // 背景色 RGB
    b
}

/// 一帧的构造描述:(是否关键帧, 该帧的块列表 [(x, y, 编码字节)])。
type FrameSpec = (bool, Vec<(i32, i32, Vec<u8>)>);

/// `BitmapSequence`(标签 46)体,对应 `ReadBitmapSequence`。
fn bitmap_seq_body(w: i32, h: i32, fps: f32, frames: &[FrameSpec]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(enc_i32(w));
    b.extend(enc_i32(h));
    b.extend_from_slice(&fps.to_le_bytes());
    b.extend(enc_u32(frames.len() as u32));

    // 第一趟:连着 N 个 bit,**字节内低位在前**,末尾补齐到字节边界
    // (位读结束后字节游标 = ceil(位数/8),见 StreamContext.h::BitsToBytes)
    let mut bits = vec![0u8; frames.len().div_ceil(8)];
    for (i, (is_key, _)) in frames.iter().enumerate() {
        if *is_key {
            bits[i / 8] |= 1 << (i % 8);
        }
    }
    b.extend_from_slice(&bits);

    // 第二趟:每帧的块
    for (_, rects) in frames {
        b.extend(enc_u32(rects.len() as u32));
        for (x, y, bytes) in rects {
            b.extend(enc_i32(*x));
            b.extend(enc_i32(*y));
            b.extend(enc_u32(bytes.len() as u32)); // readByteData 的长度前缀
            b.extend_from_slice(bytes);
        }
    }
    b
}

/// 最小合法「位图序列帧」档:1 个 BitmapComposition,2 帧,4x4,24fps。
fn minimal_bitmap_file() -> Vec<u8> {
    let attrs = comp_attrs_body(4, 4, 2, 24.0);
    let seq = bitmap_seq_body(
        4,
        4,
        24.0,
        &[
            // 第 0 帧:关键帧,整幅贴在 (0,0),WebP
            (true, vec![(0, 0, fake_webp())]),
            // 第 1 帧:非关键帧,只贴 (2,3) 处的脏矩形,PNG
            (false, vec![(2, 3, png_sig())]),
        ],
    );

    // BitmapCompositionBlock(标签 45)体 = id:EncUint32 + 子标签流 + 结束标签
    let mut comp = enc_u32(1);
    comp.extend(tag(3, &attrs)); // CompositionAttributes
    comp.extend(tag(46, &seq)); // BitmapSequence
    comp.extend(end_tag());

    // 文件体 = 一个 composition 块 + 结束标签
    let mut body = tag(45, &comp);
    body.extend(end_tag());

    wrap(1, b'U', &body)
}

/// 最小「矢量档」:一个空的 VectorCompositionBlock(标签 2)。
fn minimal_vector_file() -> Vec<u8> {
    let attrs = comp_attrs_body(100, 50, 30, 30.0);
    let mut comp = enc_u32(7);
    comp.extend(tag(3, &attrs));
    comp.extend(end_tag());
    let mut body = tag(2, &comp);
    body.extend(end_tag());
    wrap(1, b'U', &body)
}

/// 最小「视频档」:VideoCompositionBlock(标签 50)。
/// 注意它的体是 id:EncUint32 + **hasAlpha:1 字节 Boolean** + 子标签流。
fn minimal_video_file() -> Vec<u8> {
    let attrs = comp_attrs_body(64, 64, 10, 25.0);
    let mut comp = enc_u32(3);
    comp.push(0x01); // hasAlpha = true
    comp.extend(tag(3, &attrs));
    // VideoSequence(标签 51):体内容本 crate 未核实,这里塞几个字节
    // 只为验证"见到了、计了数、按长度跳过、没试图解析"
    comp.extend(tag(51, &[0xDE, 0xAD, 0xBE, 0xEF]));
    comp.extend(end_tag());
    let mut body = tag(50, &comp);
    body.extend(end_tag());
    wrap(1, b'U', &body)
}

// ---------------------------------------------------------------------------
// 金样:逐字节标注的固件
// ---------------------------------------------------------------------------

/// `minimal_bitmap_file()` 的**每一个字节**,逐条标出它按规范对应什么。
///
/// 为什么要有这条:上面的固件构造器(写侧)和被测解析器(读侧)虽然是两段
/// 独立的代码,但都是同一个人照同一份源码写的 —— **两边一起理解错,测试照样全绿**。
/// 把字节钉死在这里,理解错就必须在这张表上显形:任何人可以拿这 66 个字节
/// 对着 libpag 源码逐条复核,不需要读我们的任何一行代码。
///
/// 这张表是手工核对过的(不是从程序输出粘贴后就算数),核对方式见每行注释。
#[rustfmt::skip]
const GOLDEN_MINIMAL_BITMAP: [u8; 66] = [
    // ---- 文件头(Codec.cpp::ReadBodyBytes / Codec::Encode)----
    0x50, 0x41, 0x47,       // 魔数 'P' 'A' 'G'
    0x01,                   // 版本 = 1(Version.h: `static const uint8_t Version = 1;`)
    0x39, 0x00, 0x00, 0x00, // 体长度 = uint32 小端 0x39 = 57
    0x55,                   // 压缩标志 'U' = CompressionAlgorithm::UNCOMPRESSED

    // ---- 文件体开始(偏移 9),共 57 字节 ----
    // 标签头 uint16 小端 = 0x0B75 = 2933
    //   length = 2933 & 63 = 53   (2933 - 45*64 = 2933 - 2880)
    //   code   = 2933 >> 6 = 45   = BitmapCompositionBlock
    0x75, 0x0B,

    // ---- BitmapCompositionBlock 体(偏移 11),共 53 字节 ----
    0x01,                   // id = EncodedUint32 = 1

    // 标签头 = 0x00CA = 202;length = 202 & 63 = 10;code = 202 >> 6 = 3 = CompositionAttributes
    0xCA, 0x00,
    // ---- CompositionAttributes 体(偏移 14),共 10 字节 ----
    0x08,                   // width  = EncodedInt32:data=8,符号位=0,值=8>>1=4
    0x08,                   // height = 同上 = 4
    0x02,                   // duration = EncodedUint64 = 2(单位是**帧**)
    0x00, 0x00, 0xC0, 0x41, // frameRate = f32 小端 0x41C00000 = 24.0
    0x00, 0x00, 0x00,       // 背景色 R,G,B(ReadColor 只有三个分量,无 alpha)

    // 标签头 = 0x0BA4 = 2980;length = 2980 & 63 = 36;code = 2980 >> 6 = 46 = BitmapSequence
    0xA4, 0x0B,
    // ---- BitmapSequence 体(偏移 26),共 36 字节 ----
    0x08,                   // width  = 4
    0x08,                   // height = 4
    0x00, 0x00, 0xC0, 0x41, // frameRate = 24.0
    0x02,                   // 帧数 = EncodedUint32 = 2
    // 第一趟:2 个 bit 的 isKeyframe,**字节内低位在前**
    //   bit0 = 1 → 第 0 帧是关键帧;bit1 = 0 → 第 1 帧不是
    // 位读后字节游标 = ceil(2/8) = 1,所以这里恰好消耗 1 个字节
    0x01,
    // 第二趟,第 0 帧:
    0x01,                   //   块数 = 1
    0x00,                   //   x = EncodedInt32:data=0 → 0
    0x00,                   //   y = 0
    0x0C,                   //   fileBytes 长度 = EncodedUint32 = 12
    0x52, 0x49, 0x46, 0x46, //   "RIFF"  ┐
    0x04, 0x00, 0x00, 0x00, //   RIFF 块大小 │ 12 字节的最小 WebP 头
    0x57, 0x45, 0x42, 0x50, //   "WEBP"  ┘  (WebpDecoder.h: RIFF_HEADER_SIZE 12)
    // 第二趟,第 1 帧:
    0x01,                   //   块数 = 1
    0x04,                   //   x = EncodedInt32:data=4,符号位=0,值=4>>1=2
    0x06,                   //   y = data=6,符号位=0,值=3
    0x08,                   //   fileBytes 长度 = 8
    0x89, 0x50, 0x4E, 0x47, //   PNG 签名前半
    0x0D, 0x0A, 0x1A, 0x0A, //   PNG 签名后半

    0x00, 0x00,             // BitmapCompositionBlock 的结束标签(uint16 0)
    0x00, 0x00,             // 文件体的结束标签
];

#[test]
fn the_fixture_builder_produces_exactly_the_golden_bytes() {
    // 构造器和金样对不上,说明要么构造器改了、要么我们对格式的理解变了 ——
    // 两种情况都必须回到 libpag 源码上重新核一遍,而不是顺手改金样。
    assert_eq!(
        minimal_bitmap_file().as_slice(),
        GOLDEN_MINIMAL_BITMAP.as_slice()
    );
}

#[test]
fn the_golden_bytes_parse_to_the_documented_values() {
    // 直接拿金样喂解析器,绕开构造器 —— 这样读侧的正确性不依赖写侧。
    let pag = PagFile::parse(&GOLDEN_MINIMAL_BITMAP).expect("金样应当解析成功");
    assert_eq!(pag.version, 1);
    assert_eq!(pag.kind(), FileKind::Bitmap);

    let a = pag.attributes().unwrap();
    assert_eq!((a.width, a.height), (4, 4));
    assert_eq!(a.duration_frames, 2);
    assert_eq!(a.frame_rate, 24.0);

    let seq = pag.bitmap_sequences().next().unwrap();
    assert_eq!(seq.frames.len(), 2);
    assert!(seq.frames[0].is_keyframe);
    assert!(!seq.frames[1].is_keyframe);
    assert_eq!(seq.frames[0].bitmaps[0].encoding(), ImageEncoding::Webp);
    assert_eq!(seq.frames[0].bitmaps[0].bytes.len(), 12);
    assert_eq!(
        (seq.frames[1].bitmaps[0].x, seq.frames[1].bitmaps[0].y),
        (2, 3)
    );
    assert_eq!(seq.frames[1].bitmaps[0].encoding(), ImageEncoding::Png);
}

// ---------------------------------------------------------------------------
// 正向:合法最小文件
// ---------------------------------------------------------------------------

#[test]
fn minimal_bitmap_file_parses() {
    let bytes = minimal_bitmap_file();
    let pag = PagFile::parse(&bytes).expect("最小位图档应当解析成功");

    assert_eq!(pag.version, 1);
    assert_eq!(pag.kind(), FileKind::Bitmap);
    // 只有一个 BitmapComposition ⇒ 没有矢量根能对序列施加变换
    assert!(pag.is_pure_bitmap());
    assert_eq!(pag.compositions.len(), 1);

    let comp = pag.main_composition().unwrap();
    assert_eq!(comp.id, 1);
    assert_eq!(comp.kind, CompositionKind::Bitmap);
    // 位图档没有 hasAlpha 字段(那是视频块独有的)
    assert_eq!(comp.has_alpha, None);
    assert_eq!(comp.video_sequence_count, 0);

    let attrs = pag.attributes().expect("应当有 CompositionAttributes");
    assert_eq!(attrs.width, 4);
    assert_eq!(attrs.height, 4);
    assert_eq!(attrs.duration_frames, 2);
    assert_eq!(attrs.frame_rate, 24.0);
    assert_eq!(attrs.background_color, [0, 0, 0]);
    // 2 帧 / 24fps
    assert!((attrs.duration_seconds().unwrap() - 2.0 / 24.0).abs() < 1e-12);
}

#[test]
fn bitmap_frames_and_encodings_come_out() {
    let bytes = minimal_bitmap_file();
    let pag = PagFile::parse(&bytes).unwrap();
    let seqs: Vec<_> = pag.bitmap_sequences().collect();
    assert_eq!(seqs.len(), 1);
    let seq = seqs[0];

    assert_eq!(seq.width, 4);
    assert_eq!(seq.height, 4);
    assert_eq!(seq.frame_rate, 24.0);
    assert_eq!(seq.frames.len(), 2);

    // 第 0 帧:关键帧,一个 WebP 块贴在 (0,0)
    assert!(seq.frames[0].is_keyframe);
    assert_eq!(seq.frames[0].bitmaps.len(), 1);
    let r0 = &seq.frames[0].bitmaps[0];
    assert_eq!((r0.x, r0.y), (0, 0));
    assert_eq!(r0.encoding(), ImageEncoding::Webp);
    assert_eq!(r0.bytes, fake_webp().as_slice());

    // 第 1 帧:非关键帧,PNG 块贴在 (2,3)
    assert!(!seq.frames[1].is_keyframe);
    let r1 = &seq.frames[1].bitmaps[0];
    assert_eq!((r1.x, r1.y), (2, 3));
    assert_eq!(r1.encoding(), ImageEncoding::Png);
    assert_eq!(r1.bytes, png_sig().as_slice());

    assert_eq!(
        seq.encodings(),
        vec![ImageEncoding::Webp, ImageEncoding::Png]
    );

    // 差分语义:第 1 帧不是关键帧,要从第 0 帧开始重放
    assert_eq!(seq.start_frame_for(0), Some(0));
    assert_eq!(seq.start_frame_for(1), Some(0));
    // 越界帧号不 panic,返回 None
    assert_eq!(seq.start_frame_for(2), None);
    assert_eq!(seq.start_frame_for(usize::MAX), None);
}

#[test]
fn vector_file_is_classified_as_vector() {
    let bytes = minimal_vector_file();
    let pag = PagFile::parse(&bytes).unwrap();
    assert_eq!(pag.kind(), FileKind::Vector);
    assert!(!pag.is_pure_bitmap());
    assert_eq!(
        pag.main_composition().unwrap().kind,
        CompositionKind::Vector
    );
    // 矢量档没有序列帧可取 —— 如实为空,而不是编一个出来
    assert_eq!(pag.bitmap_sequences().count(), 0);
    let attrs = pag.attributes().unwrap();
    assert_eq!((attrs.width, attrs.height), (100, 50));
    assert_eq!(attrs.frame_rate, 30.0);
}

#[test]
fn video_file_is_counted_not_parsed() {
    let bytes = minimal_video_file();
    let pag = PagFile::parse(&bytes).unwrap();
    assert_eq!(pag.kind(), FileKind::Video);
    assert!(!pag.is_pure_bitmap());
    let comp = pag.main_composition().unwrap();
    assert_eq!(comp.kind, CompositionKind::Video);
    // hasAlpha 只有视频块有;读漏它会让后面整个标签流错位一个字节,
    // 所以属性还能解出来本身就是这个字段读对了的证据
    assert_eq!(comp.has_alpha, Some(true));
    assert_eq!(comp.video_sequence_count, 1);
    assert_eq!(pag.attributes().unwrap().width, 64);
    // 视频序列不解析,拿不到帧
    assert_eq!(pag.bitmap_sequences().count(), 0);
}

#[test]
fn multiple_compositions_use_the_last_as_main() {
    // libpag `src/base/File.cpp:85`: mainComposition = compositions.back();
    let a = comp_attrs_body(10, 10, 1, 10.0);
    let b = comp_attrs_body(999, 888, 5, 60.0);
    let mut c1 = enc_u32(1);
    c1.extend(tag(3, &a));
    c1.extend(end_tag());
    let mut c2 = enc_u32(2);
    c2.extend(tag(3, &b));
    c2.extend(end_tag());

    let mut body = tag(2, &c1);
    body.extend(tag(2, &c2));
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    let pag = PagFile::parse(&bytes).unwrap();
    assert_eq!(pag.compositions.len(), 2);
    let attrs = pag.attributes().unwrap();
    assert_eq!((attrs.width, attrs.height), (999, 888));
}

#[test]
fn mixed_file_reports_mixed() {
    // 矢量 + 位图 + 视频同时存在
    let attrs = comp_attrs_body(8, 8, 1, 12.0);
    let seq = bitmap_seq_body(8, 8, 12.0, &[(true, vec![(0, 0, fake_webp())])]);

    let mut vec_comp = enc_u32(1);
    vec_comp.extend(tag(3, &attrs));
    vec_comp.extend(end_tag());

    let mut bmp_comp = enc_u32(2);
    bmp_comp.extend(tag(3, &attrs));
    bmp_comp.extend(tag(46, &seq));
    bmp_comp.extend(end_tag());

    let mut vid_comp = enc_u32(3);
    vid_comp.push(0x00);
    vid_comp.extend(tag(3, &attrs));
    // VideoComposition::verify() 要求 sequences 非空,所以这里必须有一条
    vid_comp.extend(tag(51, &[0xDE, 0xAD]));
    vid_comp.extend(end_tag());

    let mut body = tag(2, &vec_comp);
    body.extend(tag(45, &bmp_comp));
    body.extend(tag(50, &vid_comp));
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    let pag = PagFile::parse(&bytes).unwrap();
    assert_eq!(pag.kind(), FileKind::Mixed);
    assert_eq!(pag.compositions.len(), 3);
    assert!(!pag.is_pure_bitmap());
}

// ---------------------------------------------------------------------------
// F1:`kind()` 分不出的那一档 —— 矢量根 + 位图子 composition
// ---------------------------------------------------------------------------

/// 一个矢量根 composition(1920x1080@30,**在后 ⇒ 是主 composition**)
/// 加一个位图子 composition(4x4@24,在前)。
///
/// 这是 `kind()` 单独用不得的原因:它对这个文件和「纯位图档」返回**同一个**
/// `FileKind::Bitmap`,但两者能不能靠重放序列还原完全不同 ——
/// 矢量根可以对子档施加变换/蒙版/混合,而那些在 `LayerBlock` 里,我们不解析。
fn vector_root_over_bitmap_child_file() -> Vec<u8> {
    let seq = bitmap_seq_body(4, 4, 24.0, &[(true, vec![(0, 0, fake_webp())])]);
    let mut bmp = enc_u32(2);
    bmp.extend(tag(3, &comp_attrs_body(4, 4, 1, 24.0)));
    bmp.extend(tag(46, &seq));
    bmp.extend(end_tag());

    let mut root = enc_u32(1);
    root.extend(tag(3, &comp_attrs_body(1920, 1080, 90, 30.0)));
    root.extend(end_tag());

    let mut body = tag(45, &bmp); // 位图子档在前
    body.extend(tag(2, &root)); // 矢量根在后 ⇒ compositions.back() 是它
    body.extend(end_tag());
    wrap(1, b'U', &body)
}

#[test]
fn kind_bitmap_alone_cannot_tell_a_pure_bitmap_file_from_a_vector_root() {
    let mixed = vector_root_over_bitmap_child_file();
    let mixed = PagFile::parse(&mixed).unwrap();
    let pure = minimal_bitmap_file();
    let pure = PagFile::parse(&pure).unwrap();

    // 两个文件的 kind() **一模一样** —— 这正是不能拿它当判据的原因
    assert_eq!(mixed.kind(), FileKind::Bitmap);
    assert_eq!(pure.kind(), FileKind::Bitmap);

    // is_pure_bitmap() 才分得开
    assert!(!mixed.is_pure_bitmap());
    assert!(pure.is_pure_bitmap());
}

#[test]
fn attributes_and_sequences_can_come_from_different_compositions() {
    // 混合档上 attributes()(主 = 矢量根)与 bitmap_sequences()(子档)
    // 返回的是两个不同 composition 的数据。这不是 bug,是事实 ——
    // 但调用方必须知道,所以钉一条测试在这里。
    let bytes = vector_root_over_bitmap_child_file();
    let pag = PagFile::parse(&bytes).unwrap();

    assert_eq!(
        pag.main_composition().unwrap().kind,
        CompositionKind::Vector
    );
    let a = pag.attributes().unwrap();
    assert_eq!((a.width, a.height, a.frame_rate), (1920, 1080, 30.0));

    let s = pag.bitmap_sequences().next().unwrap();
    assert_eq!((s.width, s.height, s.frame_rate), (4, 4, 24.0));

    // 照着 seq 的宽高开画布只覆盖这一个图层,不是整个文件的画面
    assert_ne!((a.width, a.height), (s.width, s.height));
}

// ---------------------------------------------------------------------------
// 反向:文件头
// ---------------------------------------------------------------------------

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = minimal_bitmap_file();
    bytes[0] = b'X';
    assert_eq!(
        PagFile::parse(&bytes),
        Err(PagError::BadMagic { found: *b"XAG" })
    );

    // 一段和 PAG 完全无关、但长度够的数据
    let junk = vec![0u8; 64];
    assert!(matches!(
        PagFile::parse(&junk),
        Err(PagError::BadMagic { .. })
    ));
}

#[test]
fn encrypted_version_is_rejected_explicitly() {
    // libpag `Codec.cpp`:EncryptedVersion = 3,先于 KnownVersion 判定。
    // 必须报"加密"而不是"版本不支持" —— 两者的处置方式完全不同。
    let mut bytes = minimal_bitmap_file();
    bytes[3] = 3;
    assert_eq!(
        PagFile::parse(&bytes),
        Err(PagError::Encrypted { version: 3 })
    );
}

#[test]
fn unknown_version_is_rejected() {
    // KnownVersion = 3,所以 4 及以上一律拒
    for v in [4u8, 5, 100, 255] {
        let mut bytes = minimal_bitmap_file();
        bytes[3] = v;
        assert_eq!(
            PagFile::parse(&bytes),
            Err(PagError::UnsupportedVersion { version: v })
        );
    }
    // 0/1/2 是 libpag 接受的区间(它写的是 1)
    for v in [0u8, 1, 2] {
        let mut bytes = minimal_bitmap_file();
        bytes[3] = v;
        assert!(PagFile::parse(&bytes).is_ok(), "版本 {v} 应当被接受");
    }
}

#[test]
fn compressed_body_is_rejected_not_guessed() {
    // CompressionAlgorithm 里 'Z'(ZLIB)/'L'(LZMA)有定义,
    // 但 libpag 自己的解码器也只接受 'U' —— 我们照样显式拒绝,不猜封装。
    for c in [b'Z', b'L', 0x00, 0xFF] {
        let mut bytes = minimal_bitmap_file();
        bytes[8] = c;
        assert_eq!(
            PagFile::parse(&bytes),
            Err(PagError::UnsupportedCompression { code: c })
        );
    }
}

#[test]
fn files_shorter_than_the_minimum_header_are_rejected() {
    // libpag 的门槛是 11 字节:9 字节头 + 至少一个 2 字节结束标签
    let full = minimal_bitmap_file();
    for n in 0..11 {
        let slice = &full[..n.min(full.len())];
        assert_eq!(
            PagFile::parse(slice),
            Err(PagError::TooShort { len: n }),
            "长度 {n} 应当报 TooShort"
        );
    }
}

#[test]
fn empty_body_has_no_compositions() {
    let body = end_tag(); // 只有一个结束标签
    let bytes = wrap(1, b'U', &body);
    assert_eq!(bytes.len(), 11); // 正好卡在最小长度上
    assert_eq!(PagFile::parse(&bytes), Err(PagError::NoCompositions));
}

// ---------------------------------------------------------------------------
// 反向:截断 —— 每个字段边界都截一刀
// ---------------------------------------------------------------------------

#[test]
fn every_truncation_of_a_valid_file_errors_and_never_panics() {
    // 这是本 crate 最重要的一条测试:把一个合法文件从每一个字节位置切断,
    // 要求**全部返回 Err**。任何一处 panic 都会让这条测试直接失败。
    let full = minimal_bitmap_file();
    assert!(full.len() > 40, "固件太短就测不出什么了:{}", full.len());

    for n in 0..full.len() {
        let result = PagFile::parse(&full[..n]);
        assert!(
            result.is_err(),
            "截断到 {n}/{} 字节居然解析成功了",
            full.len()
        );
    }
    // 完整长度必须成功 —— 否则上面的循环全是假阳性
    assert!(PagFile::parse(&full).is_ok());
}

#[test]
fn truncating_the_other_fixtures_also_never_panics() {
    // 每个固件都带上它**完整**时应有的结果 —— 上一版这里是
    // `assert!(parse(&fixture).is_ok() || fixture.len() == 11)`,
    // 空体固件(长度恰好 11、返回 NoCompositions)从 `||` 那一支逃掉了,
    // 等于对它什么都没断言。
    let cases: [(Vec<u8>, Result<(), PagError>); 4] = [
        (minimal_vector_file(), Ok(())),
        (minimal_video_file(), Ok(())),
        (vector_root_over_bitmap_child_file(), Ok(())),
        (wrap(1, b'U', &end_tag()), Err(PagError::NoCompositions)),
    ];
    for (fixture, expected) in cases {
        for n in 0..fixture.len() {
            // 截断一律 Err(短于 11 字节报 TooShort,其余报别的),且绝不 panic
            assert!(
                PagFile::parse(&fixture[..n]).is_err(),
                "截断到 {n}/{} 字节居然解析成功了",
                fixture.len()
            );
        }
        // 完整长度的结果必须**精确**符合预期,否则上面的循环可能全是假阳性
        assert_eq!(
            PagFile::parse(&fixture).map(|_| ()),
            expected,
            "固件完整解析的结果与预期不符"
        );
    }
}

#[test]
fn body_length_larger_than_the_file_is_clamped_like_libpag() {
    // Codec.cpp: bodyLength = std::min(bodyLength, stream->bytesAvailable());
    // 声称的体长大于实际剩余时**不报错**,按剩余截断 ——
    // 报错会拒掉 libpag 能打开的文件。真正的越界由标签循环兜住。
    let mut bytes = minimal_bitmap_file();
    let real_body_len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    bytes[4..8].copy_from_slice(&(real_body_len + 10_000).to_le_bytes());
    // 截断后内容仍然完整,所以照样解析成功
    assert!(PagFile::parse(&bytes).is_ok());
}

// ---------------------------------------------------------------------------
// 反向:声称长度超过剩余字节(经典越界 / OOM 来源)
// ---------------------------------------------------------------------------

#[test]
fn tag_claiming_more_bytes_than_remain_is_rejected() {
    // 标签头的长度字段是攻击者可控的 u32。构造一个体长 = u32::MAX 的标签。
    let mut body = Vec::new();
    body.extend_from_slice(&((45u16 << 6) | 63).to_le_bytes()); // 逃逸到 uint32 长度
    body.extend_from_slice(&u32::MAX.to_le_bytes());
    body.extend_from_slice(&[0x01, 0x02, 0x03]); // 只有 3 字节真数据
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    // available = 5:标签头之后剩下的 3 字节数据 + 2 字节结束标签。
    // (第一版这里写 3,忘了结束标签也在 body 里 —— 解析器是对的,断言错了)
    assert_eq!(
        PagFile::parse(&bytes),
        Err(PagError::TagLengthOverflow {
            code: 45,
            length: u32::MAX,
            available: 5,
        })
    );
}

#[test]
fn frame_byte_data_claiming_more_bytes_than_remain_is_rejected() {
    // readByteData 的长度前缀同样是变长整数,同样可控。
    // 手工造一个 BitmapSequence:1 帧 1 块,块字节声称 100000 字节但实际没有。
    let mut seq = Vec::new();
    seq.extend(enc_i32(4)); // width
    seq.extend(enc_i32(4)); // height
    seq.extend_from_slice(&24.0f32.to_le_bytes()); // frameRate
    seq.extend(enc_u32(1)); // count = 1 帧
    seq.push(0x01); // 1 个 bit:isKeyframe = true(补齐到 1 字节)
    seq.extend(enc_u32(1)); // bitmapCount = 1
    seq.extend(enc_i32(0)); // x
    seq.extend(enc_i32(0)); // y
    seq.extend(enc_u32(100_000)); // 声称 10 万字节
    seq.extend_from_slice(&[0xAA, 0xBB]); // 实际只有 2 字节

    let mut comp = enc_u32(1);
    comp.extend(tag(46, &seq));
    comp.extend(end_tag());
    let mut body = tag(45, &comp);
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    assert!(
        matches!(
            PagFile::parse(&bytes),
            Err(PagError::UnexpectedEof { need: 100_000, .. })
        ),
        "实际得到:{:?}",
        PagFile::parse(&bytes)
    );
}

#[test]
fn absurd_frame_count_is_rejected_without_allocating() {
    // 这条防的是"5 字节的 varint 让我们为 40 亿个 frame 结构体分配内存"。
    // libpag 没有这个检查,是我们加的硬化。
    let mut seq = Vec::new();
    seq.extend(enc_i32(4));
    seq.extend(enc_i32(4));
    seq.extend_from_slice(&24.0f32.to_le_bytes());
    seq.extend(enc_u32(u32::MAX)); // 声称 4294967295 帧
    seq.extend_from_slice(&[0x00; 4]); // 后面只有 4 字节

    let mut comp = enc_u32(1);
    comp.extend(tag(46, &seq));
    comp.extend(end_tag());
    let mut body = tag(45, &comp);
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    assert!(
        matches!(
            PagFile::parse(&bytes),
            Err(PagError::FrameCountTooLarge {
                count: u32::MAX,
                ..
            })
        ),
        "实际得到:{:?}",
        PagFile::parse(&bytes)
    );
}

#[test]
fn absurd_bitmap_count_is_rejected_without_allocating() {
    let mut seq = Vec::new();
    seq.extend(enc_i32(4));
    seq.extend(enc_i32(4));
    seq.extend_from_slice(&24.0f32.to_le_bytes());
    seq.extend(enc_u32(1)); // 1 帧 —— 能通过帧数上界
    seq.push(0x01); // isKeyframe
    seq.extend(enc_u32(u32::MAX)); // 但这一帧声称有 40 亿个块
    seq.extend_from_slice(&[0x00; 4]);

    let mut comp = enc_u32(1);
    comp.extend(tag(46, &seq));
    comp.extend(end_tag());
    let mut body = tag(45, &comp);
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    assert!(
        matches!(
            PagFile::parse(&bytes),
            Err(PagError::BitmapCountTooLarge {
                count: u32::MAX,
                ..
            })
        ),
        "实际得到:{:?}",
        PagFile::parse(&bytes)
    );
}

#[test]
fn missing_end_tag_errors_instead_of_looping_forever() {
    // 标签流没有结束标签 —— 循环必须靠 EOF 收场,不能空转
    let attrs = comp_attrs_body(4, 4, 1, 24.0);
    let mut comp = enc_u32(1);
    comp.extend(tag(3, &attrs));
    // 故意不写 end_tag()
    let mut body = tag(45, &comp);
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);
    assert!(PagFile::parse(&bytes).is_err());
}

// ---------------------------------------------------------------------------
// 编码原语:这些是最容易写错、错了又最难发现的地方
// ---------------------------------------------------------------------------

/// 把一条位图序列包进一个能通过 `verify()` 的最小位图档。
fn bitmap_file_with_sequence(seq_body: &[u8]) -> Vec<u8> {
    let mut comp = enc_u32(1);
    comp.extend(tag(3, &comp_attrs_body(4, 4, 1, 24.0)));
    comp.extend(tag(46, seq_body));
    comp.extend(end_tag());
    let mut body = tag(45, &comp);
    body.extend(end_tag());
    wrap(1, b'U', &body)
}

#[test]
fn encoded_int32_sign_bit_is_not_zigzag() {
    // readEncodedInt32: value = data >> 1; (data & 1) ? -value : value
    // 标准 zigzag 是 (n >> 1) ^ -(n & 1) —— data = 3 时 PAG 解出 -1,zigzag 解出 -2。
    //
    // 负值只能走**块坐标** x/y:序列的 width/height 被 libpag 的
    // `Sequence::verify()` 卡在 > 0,负宽高的文件根本进不来(也不该进来)。
    // 块坐标没有这个约束,而且它和 width/height 走的是同一个 read_encoded_i32,
    // 所以打穿它就等于打穿这个原语。
    //
    // 取值一路顶到 **±i32::MAX**:那里 data = 0xFFFF_FFFE / 0xFFFF_FFFF,
    // 是 5 组 varint 的最后一组只剩 4 个有效位的边界,读侧 `(data >> 1) as i32`
    // 与取负两条路都在这个点上最容易写错。
    // (`i32::MIN` 不在 EncodedInt32 的值域里,见 enc_i32 的说明。)
    for v in [
        0i32,
        1,
        -1,
        2,
        -2,
        127,
        -127,
        128,
        -128,
        65535,
        -65535,
        1 << 30,
        -(1 << 30),
        i32::MAX,
        -i32::MAX,
    ] {
        let seq = bitmap_seq_body(4, 4, 24.0, &[(true, vec![(v, -v, fake_webp())])]);
        let bytes = bitmap_file_with_sequence(&seq);

        let pag = PagFile::parse(&bytes).unwrap_or_else(|e| panic!("v={v} 解析失败:{e}"));
        let s = pag.bitmap_sequences().next().unwrap();
        assert_eq!(s.frames[0].bitmaps[0].x, v, "x 在 v={v} 时解错");
        assert_eq!(s.frames[0].bitmaps[0].y, -v, "y 在 v={v} 时解错");
    }

    // 正值一侧再单独打一遍 width/height(它们同样走这个原语)
    for v in [1i32, 2, 127, 128, 65535, 1 << 30, i32::MAX] {
        let seq = bitmap_seq_body(v, v, 24.0, &[(true, vec![(0, 0, fake_webp())])]);
        let bytes = bitmap_file_with_sequence(&seq);
        let pag = PagFile::parse(&bytes).unwrap_or_else(|e| panic!("width={v} 解析失败:{e}"));
        let s = pag.bitmap_sequences().next().unwrap();
        assert_eq!((s.width, s.height), (v, v), "width/height 在 v={v} 时解错");
    }
}

#[test]
fn bit_align_rounds_the_byte_cursor_up() {
    // **这条是整个 crate 最容易悄悄写错的地方。**
    // 位读结束后字节游标要向上取整(BitsToBytes = ceil(bits/8))。
    // 帧数 ≤ 8 时一个字节装得下,写错也看不出来 —— 所以这里特意用 9 帧和 17 帧,
    // 跨过字节边界,让"少进一个字节 / 多进一个字节"立刻暴露成解析失败或数据错乱。
    for n in [1usize, 7, 8, 9, 15, 16, 17, 33] {
        let frames: Vec<FrameSpec> = (0..n)
            // 交替关键帧,这样 bit 图样不是全 0 或全 1
            .map(|i| (i % 3 == 0, vec![(i as i32, 0, fake_webp())]))
            .collect();
        let seq = bitmap_seq_body(4, 4, 24.0, &frames);
        let bytes = bitmap_file_with_sequence(&seq);

        let pag = PagFile::parse(&bytes).unwrap_or_else(|e| panic!("{n} 帧解析失败:{e}"));
        let s = pag.bitmap_sequences().next().unwrap();
        assert_eq!(s.frames.len(), n);
        for (i, f) in s.frames.iter().enumerate() {
            assert_eq!(
                f.is_keyframe,
                i % 3 == 0,
                "{n} 帧固件的第 {i} 帧关键帧位错了"
            );
            // x 坐标同时充当"这一帧的块数据没错位"的校验
            assert_eq!(f.bitmaps[0].x, i as i32, "{n} 帧固件的第 {i} 帧块错位了");
        }
    }
}

#[test]
fn long_tag_bodies_use_the_uint32_escape() {
    // length == 63 是**逃逸标记**不是真长度:
    // WriteTypeAndLength 的判据是 `if (length < 63)`,所以体长恰好 63 也要走 uint32。
    // 构造一个体长跨过 62/63/64 的序列,验证两条路径都对。
    for payload in [60usize, 61, 62, 63, 64, 200] {
        let blob = vec![0xABu8; payload];
        let mut comp = enc_u32(1);
        // 用一个我们不认识的标签号(99)承载,验证"跳过"路径也能正确处理长体
        comp.extend(tag(99, &blob));
        comp.extend(tag(3, &comp_attrs_body(1, 2, 3, 4.0)));
        comp.extend(end_tag());
        // 用矢量块承载:位图块要通过 verify() 就得带一条序列,那与本条无关
        let mut body = tag(2, &comp);
        body.extend(end_tag());
        let bytes = wrap(1, b'U', &body);

        let pag =
            PagFile::parse(&bytes).unwrap_or_else(|e| panic!("体长 {payload} 的标签解析失败:{e}"));
        let attrs = pag.attributes().unwrap();
        assert_eq!(
            (attrs.width, attrs.height),
            (1, 2),
            "体长 {payload} 时错位了"
        );
    }
}

#[test]
fn unknown_tags_are_skipped_not_guessed() {
    // 文件级和 composition 级都塞一堆我们没核实过布局的标签,
    // 要求全部按长度跳过、不影响后续解析。
    let attrs = comp_attrs_body(16, 16, 4, 15.0);
    let mut comp = enc_u32(1);
    comp.extend(tag(5, &[0xFF; 20])); // LayerBlock —— 有子标签流,但我们不下钻
    comp.extend(tag(3, &attrs));
    comp.extend(tag(55, &[0x11; 7])); // AudioBytes
    comp.extend(end_tag());

    let mut body = Vec::new();
    body.extend(tag(1, &[0x22; 5])); // FontTables
    body.extend(tag(31, &[0x33; 9])); // FileAttributes
    body.extend(tag(47, &[0x44; 30])); // ImageBytes
    body.extend(tag(2, &comp));
    body.extend(tag(700, &[0x55; 3])); // 表里根本没有的标签号
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);

    let pag = PagFile::parse(&bytes).unwrap();
    assert_eq!(pag.compositions.len(), 1);
    let a = pag.attributes().unwrap();
    assert_eq!((a.width, a.height, a.frame_rate), (16, 16, 15.0));
}

#[test]
fn a_frame_with_zero_blocks_is_legal() {
    // 块数为 0 = 这一帧与上一帧完全相同。这是**合法**的:
    // libpag 的 BitmapFrame::verify() 是 `std::all_of(bitmaps...)`,
    // 空区间恒真;写侧 WriteBitmapSequence 也会正常写出 bitmapCount = 0。
    let seq = bitmap_seq_body(
        4,
        4,
        24.0,
        &[
            (true, vec![(0, 0, fake_webp())]),
            (false, vec![]), // 没有块
            (false, vec![(2, 2, png_sig())]),
        ],
    );
    let bytes = bitmap_file_with_sequence(&seq);

    let pag = PagFile::parse(&bytes).unwrap();
    let s = pag.bitmap_sequences().next().unwrap();
    assert_eq!(s.frames.len(), 3);
    assert!(s.frames[1].bitmaps.is_empty());
    assert_eq!(s.frames[2].bitmaps[0].encoding(), ImageEncoding::Png);
}

#[test]
fn a_zero_length_block_makes_the_whole_file_invalid() {
    // **这条曾经是反的。** 早期版本把 BitmapSequenceReader.cpp:78 的注释
    // "The returned image could be nullptr if the frame is an empty frame."
    // 当成"零长度 ByteData 是合法空帧"的出处 —— 但那句注的是**上一行
    // ImageCodec::MakeFrom 的返回值**,与 ByteData 的长度无关。
    //
    // libpag 的真实链条是反的,逐条都读过原文:
    //   DecodeStream::readByteData (DecodeStream.cpp:147)
    //       `if (length == 0 || ...) return nullptr;`
    //   → BitmapRect::fileBytes == nullptr
    //   → BitmapFrame::verify   (base/BitmapSequence.cpp:34-39) 失败
    //   → BitmapSequence::verify → Composition::verify → 失败
    //   → Codec::VerifyAndMake  (Codec.cpp:150-176) delete 全部、返回 nullptr
    // 也就是说:**整个文件被拒**。
    //
    // 写侧同样印证:WriteBitmapSequence (codec/tags/BitmapSequence.cpp:64-80)
    // 在计数和写入两处都 `if (length() == 0) continue;` —— libpag 从不产出
    // 零长度块。所以这种字节序列只可能来自畸形/伪造文件。
    let seq = bitmap_seq_body(
        4,
        4,
        24.0,
        &[
            (true, vec![(0, 0, fake_webp())]),
            (false, vec![(0, 0, vec![])]), // 有块但零字节
        ],
    );
    let bytes = bitmap_file_with_sequence(&seq);

    assert_eq!(
        PagFile::parse(&bytes),
        Err(PagError::VerifyFailed {
            composition: 0,
            reason: VerifyFailure::EmptyFrameBytes {
                sequence: 0,
                frame: 1,
                bitmap: 0,
            },
        })
    );
}

// ---------------------------------------------------------------------------
// libpag 的 verify() 门:畸形但结构完整的文件必须被拒,不能原样交给上层
// ---------------------------------------------------------------------------

#[test]
fn a_sequence_with_nonpositive_geometry_is_rejected_like_libpag() {
    // Sequence::verify() (src/base/Sequence.cpp:40):
    //   composition != nullptr && width > 0 && height > 0 && frameRate > 0
    // 不拒的后果很具体:README 让上层拿 seq.width × seq.height 开画布,
    // 而 `-4i32 as usize` == 18446744073709551612。
    // "不 panic"不能靠把 panic 转移给调用方来实现。
    for (w, h, fps) in [
        (-4i32, 4i32, 24.0f32),
        (4, 0, 24.0),
        (4, 4, 0.0),
        (4, 4, -1.0),
        (4, 4, f32::NAN),
    ] {
        let seq = bitmap_seq_body(w, h, fps, &[(true, vec![(0, 0, fake_webp())])]);
        let bytes = bitmap_file_with_sequence(&seq);
        assert!(
            matches!(
                PagFile::parse(&bytes),
                Err(PagError::VerifyFailed {
                    reason: VerifyFailure::SequenceGeometry { .. },
                    ..
                })
            ),
            "{w}x{h}@{fps} 应当被 verify 拒掉,实际:{:?}",
            PagFile::parse(&bytes)
        );
    }
}

#[test]
fn a_bitmap_sequence_with_no_frames_is_rejected() {
    // BitmapSequence::verify() (src/base/BitmapSequence.cpp:75):`frames.empty()` 即失败
    let seq = bitmap_seq_body(4, 4, 24.0, &[]);
    let bytes = bitmap_file_with_sequence(&seq);
    assert_eq!(
        PagFile::parse(&bytes),
        Err(PagError::VerifyFailed {
            composition: 0,
            reason: VerifyFailure::EmptyBitmapSequence { sequence: 0 },
        })
    );
}

#[test]
fn a_composition_with_nonpositive_geometry_is_rejected() {
    // Composition::verify() (src/base/Composition.cpp:59):
    //   width > 0 && height > 0 && duration > 0 && frameRate > 0
    for (w, h, dur, fps) in [
        (0i32, 4i32, 1u64, 24.0f32),
        (4, -1, 1, 24.0),
        (4, 4, 0, 24.0), // duration = 0 帧
        (4, 4, 1, 0.0),
    ] {
        let mut comp = enc_u32(1);
        comp.extend(tag(3, &comp_attrs_body(w, h, dur, fps)));
        comp.extend(end_tag());
        let mut body = tag(2, &comp);
        body.extend(end_tag());
        let bytes = wrap(1, b'U', &body);
        assert!(
            matches!(
                PagFile::parse(&bytes),
                Err(PagError::VerifyFailed {
                    reason: VerifyFailure::CompositionGeometry { .. },
                    ..
                })
            ),
            "{w}x{h} dur={dur} fps={fps} 应当被拒,实际:{:?}",
            PagFile::parse(&bytes)
        );
    }
}

#[test]
fn a_composition_without_attributes_is_rejected() {
    // libpag 里这些字段默认 0,于是 Composition::verify() 的 width > 0 不成立。
    let mut comp = enc_u32(1);
    comp.extend(end_tag());
    let mut body = tag(2, &comp);
    body.extend(end_tag());
    let bytes = wrap(1, b'U', &body);
    assert_eq!(
        PagFile::parse(&bytes),
        Err(PagError::VerifyFailed {
            composition: 0,
            reason: VerifyFailure::MissingCompositionAttributes,
        })
    );
}

#[test]
fn a_bitmap_or_video_composition_without_sequences_is_rejected() {
    // BitmapComposition::verify() / VideoComposition::verify() 都要求
    // sequences 非空(src/base/BitmapComposition.cpp、VideoComposition.cpp)。
    let attrs = comp_attrs_body(4, 4, 1, 24.0);

    let mut bmp = enc_u32(1);
    bmp.extend(tag(3, &attrs));
    bmp.extend(end_tag());
    let mut body = tag(45, &bmp);
    body.extend(end_tag());
    assert_eq!(
        PagFile::parse(&wrap(1, b'U', &body)),
        Err(PagError::VerifyFailed {
            composition: 0,
            reason: VerifyFailure::NoSequences,
        })
    );

    let mut vid = enc_u32(1);
    vid.push(0x00); // hasAlpha
    vid.extend(tag(3, &attrs));
    vid.extend(end_tag());
    let mut body = tag(50, &vid);
    body.extend(end_tag());
    assert_eq!(
        PagFile::parse(&wrap(1, b'U', &body)),
        Err(PagError::VerifyFailed {
            composition: 0,
            reason: VerifyFailure::NoSequences,
        })
    );
}

#[test]
fn verify_failure_reports_which_composition() {
    // VerifyAndMake 是逐个查的,报出下标才好定位真文件的问题。
    // 第 0 个 composition 合法,第 1 个缺属性。
    let mut good = enc_u32(1);
    good.extend(tag(3, &comp_attrs_body(4, 4, 1, 24.0)));
    good.extend(end_tag());
    let mut bad = enc_u32(2);
    bad.extend(end_tag());

    let mut body = tag(2, &good);
    body.extend(tag(2, &bad));
    body.extend(end_tag());
    assert_eq!(
        PagFile::parse(&wrap(1, b'U', &body)),
        Err(PagError::VerifyFailed {
            composition: 1,
            reason: VerifyFailure::MissingCompositionAttributes,
        })
    );
}

#[test]
fn tag_name_table_matches_the_verified_enum() {
    assert_eq!(sv_pag::tag_name(0), Some("End"));
    assert_eq!(sv_pag::tag_name(2), Some("VectorCompositionBlock"));
    assert_eq!(sv_pag::tag_name(3), Some("CompositionAttributes"));
    assert_eq!(sv_pag::tag_name(45), Some("BitmapCompositionBlock"));
    assert_eq!(sv_pag::tag_name(46), Some("BitmapSequence"));
    assert_eq!(sv_pag::tag_name(50), Some("VideoCompositionBlock"));
    assert_eq!(sv_pag::tag_name(51), Some("VideoSequence"));
    assert_eq!(sv_pag::tag_name(94), Some("ImageScaleModes"));
    // 官方注释标明 34~44 是保留段
    for c in 34..=44u16 {
        assert_eq!(sv_pag::tag_name(c), None, "{c} 应当是保留号");
    }
    // 9 在枚举里被跳过了(8 是 TextSource,10 是 TextMoreOption)
    assert_eq!(sv_pag::tag_name(9), None);
    // 95 及以上是核实时还不存在的号
    assert_eq!(sv_pag::tag_name(95), None);
    assert_eq!(sv_pag::tag_name(1023), None);
}

#[test]
fn image_encoding_sniffing() {
    assert_eq!(ImageEncoding::sniff(&[]), ImageEncoding::Empty);
    assert_eq!(ImageEncoding::sniff(&fake_webp()), ImageEncoding::Webp);
    assert_eq!(ImageEncoding::sniff(&png_sig()), ImageEncoding::Png);
    assert_eq!(
        ImageEncoding::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
        ImageEncoding::Jpeg
    );
    // RIFF 但不是 WEBP(比如 WAV)不能误判
    let mut riff_wav = b"RIFF".to_vec();
    riff_wav.extend_from_slice(&4u32.to_le_bytes());
    riff_wav.extend_from_slice(b"WAVE");
    assert_eq!(ImageEncoding::sniff(&riff_wav), ImageEncoding::Unknown);
    // 短于 12 字节的 RIFF 不能越界读
    assert_eq!(ImageEncoding::sniff(b"RIFF"), ImageEncoding::Unknown);
    assert_eq!(ImageEncoding::sniff(&[0x00, 0x01]), ImageEncoding::Unknown);
}

// ---------------------------------------------------------------------------
// 兜底:任意字节都不能 panic
// ---------------------------------------------------------------------------

/// 一个便宜的确定性伪随机(xorshift),不引依赖。
fn xorshift(seed: u32) -> impl FnMut() -> u32 {
    let mut state = seed;
    move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    }
}

#[test]
fn random_bytes_behind_a_valid_header_never_panic() {
    // 纯随机体。**这条测不到多少东西,别指望它** ——
    // 随机 u16 标签头有 1/64 的概率撞上 length == 63 的逃逸标记、
    // 接着读一个随机 u32 当长度,于是绝大多数样本第一个标签就
    // TagLengthOverflow 出局,一个 composition 都构造不出来。
    // 真正有覆盖率的是下面那条 `mutating_a_valid_file_...`。
    // 留着它是因为它守的是另一件事:标签头那一层的守卫不能被绕过。
    let mut next = xorshift(0x1234_5678);
    for _ in 0..2000 {
        let len = 11 + (next() % 200) as usize;
        let mut buf: Vec<u8> = (0..len).map(|_| (next() & 0xFF) as u8).collect();
        buf[0] = b'P';
        buf[1] = b'A';
        buf[2] = b'G';
        buf[3] = 1;
        let body_len = (len - 9) as u32;
        buf[4..8].copy_from_slice(&body_len.to_le_bytes());
        buf[8] = b'U';

        // 解析结果无所谓,不 panic 就行
        let _ = PagFile::parse(&buf);
    }
}

#[test]
fn mutating_a_valid_file_never_panics_and_actually_reaches_the_parsers() {
    // **这条是真正有覆盖率的那一条。**
    //
    // 上一版只有纯随机体那条,自述说它"让样本真能走进标签解析"——
    // 实测是 2000 个样本里 **0 个**构造出过 composition
    // (1895 个死在 TagLengthOverflow、101 个 UnexpectedEof、4 个 NoCompositions),
    // 也就是 parse_composition / parse_composition_attributes /
    // parse_bitmap_sequence / read_ubits / read_encoded_* / read_byte_data
    // **一次都没被模糊到** —— 尤其 parse_bitmap_sequence 还是本 crate
    // 唯一的单源结构。
    //
    // 修法:拿合法文件当种子做**原地字节变异**,这样样本大多数时候仍是
    // 结构可解的,能一路走到最里层。下面的覆盖率断言把这一点钉死,
    // 防止它以后又悄悄退化回零覆盖。
    let seeds = [
        minimal_bitmap_file(),
        minimal_vector_file(),
        minimal_video_file(),
        vector_root_over_bitmap_child_file(),
        {
            // 多帧、跨字节边界的序列 —— 位读路径的模糊入口
            let frames: Vec<FrameSpec> = (0..17i32)
                .map(|i| (i % 3 == 0, vec![(i, 0, fake_webp())]))
                .collect();
            bitmap_file_with_sequence(&bitmap_seq_body(8, 8, 24.0, &frames))
        },
    ];

    let mut next = xorshift(0xC0FF_EE01);
    let mut reached_composition = 0u32;
    let mut reached_sequence = 0u32;
    let mut reached_frames = 0u32;
    let mut reached_bytes = 0u32;

    for i in 0..20_000u32 {
        let seed = &seeds[(i as usize) % seeds.len()];
        let mut buf = seed.clone();
        // 1..=3 处单字节变异。只动体,不动魔数 —— 动了就退化成上一条测试。
        let mutations = 1 + next() % 3;
        for _ in 0..mutations {
            let at = 3 + (next() as usize) % (buf.len() - 3);
            buf[at] ^= (next() & 0xFF) as u8;
        }
        // 偶尔再截一刀,把"变异 + 截断"这个组合也覆盖上
        if next().is_multiple_of(4) {
            let keep = (next() as usize) % buf.len();
            buf.truncate(keep);
        }

        if let Ok(pag) = PagFile::parse(&buf) {
            reached_composition += 1;
            let seqs: Vec<_> = pag.bitmap_sequences().collect();
            if !seqs.is_empty() {
                reached_sequence += 1;
            }
            if seqs.iter().any(|s| !s.frames.is_empty()) {
                reached_frames += 1;
            }
            if seqs
                .iter()
                .any(|s| s.frames.iter().any(|f| !f.bitmaps.is_empty()))
            {
                reached_bytes += 1;
                // 顺手把访问器也走一遍 —— 它们同样不许 panic
                for s in &seqs {
                    let _ = s.encodings();
                    let _ = s.duration_seconds();
                    for n in 0..s.frames.len() + 2 {
                        let _ = s.start_frame_for(n);
                    }
                }
            }
            let _ = pag.kind();
            let _ = pag.is_pure_bitmap();
            let _ = pag.attributes().and_then(|a| a.duration_seconds());
        }
    }

    // 覆盖率断言:这条测试必须**真的走进**它声称在保护的代码。
    // 阈值取得很松(实测远高于此),它防的是"退化回 0",不是性能回归。
    assert!(
        reached_composition >= 100,
        "模糊样本没能构造出 composition(命中 {reached_composition} 次),这条测试等于没测"
    );
    assert!(
        reached_sequence >= 100,
        "模糊样本没能走进 parse_bitmap_sequence(命中 {reached_sequence} 次)"
    );
    assert!(
        reached_frames >= 100,
        "模糊样本没能走进位读 + 帧循环(命中 {reached_frames} 次)"
    );
    assert!(
        reached_bytes >= 100,
        "模糊样本没能走进 read_byte_data(命中 {reached_bytes} 次)"
    );
}
