//! 解析结果的数据模型。
//!
//! 全部**零拷贝**:帧字节是对输入缓冲区的借用(`&'a [u8]`),
//! 解析一个几 MB 的序列帧档不会把帧数据再复制一份。

/// 一个解析出来的 PAG 文件。
///
/// 生命周期 `'a` 绑在输入缓冲区上 —— 帧字节是借来的,不是拷贝的。
#[derive(Clone, Debug, PartialEq)]
pub struct PagFile<'a> {
    /// 文件头里的版本字节。libpag 当前写 1(`src/codec/Version.h`),
    /// 解码器接受 0..=2(3 是加密档,会被 `PagError::Encrypted` 挡掉)。
    pub version: u8,
    /// 文件里的全部 composition,**按文件出现顺序**。
    pub compositions: Vec<Composition<'a>>,
}

impl<'a> PagFile<'a> {
    /// 解析一个 `.pag` 的字节。任何畸形输入都返回 `Err`,不 panic。
    pub fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::PagError> {
        crate::parse::parse(bytes)
    }

    /// 主 composition。
    ///
    /// **是列表里的最后一个**,不是第一个 —— 抄自 libpag `src/base/File.cpp:85`
    /// 的 `mainComposition = compositions.back();`。`File::width()` /
    /// `height()` / `duration()` / `frameRate()` 全部取自它。
    ///
    /// 解析成功时一定非空(空列表会在 `parse` 里就报 `NoCompositions`),
    /// 但仍返回 `Option` 而不是直接索引 —— 不给自己留 panic 的口子。
    pub fn main_composition(&self) -> Option<&Composition<'a>> {
        self.compositions.last()
    }

    /// 主 composition 的属性(宽高/时长/帧率/背景色)。
    ///
    /// ⚠️ **这是主 composition 的属性,不一定是 [`Self::bitmap_sequences`] 里
    /// 任何一条序列的属性。** 矢量根套位图子 composition 的混合档上,这两个
    /// 方法返回的是**两个不同 composition 的数据**:`attributes()` 给你根的
    /// 1920x1080@30,`bitmap_sequences()` 给你子档的 4x4@24。要判断能不能把
    /// 序列当成整个文件的画面,用 [`Self::is_pure_bitmap`]。
    pub fn attributes(&self) -> Option<&CompositionAttributes> {
        self.main_composition()?.attributes.as_ref()
    }

    /// 文件里出现过**哪些序列类型**。
    ///
    /// **这条判定规则是我们自己定的,不是 libpag 的 API** —— libpag 没有
    /// "文件是什么档"这个概念,只有每个 composition 各自的类型
    /// (`CompositionType::Vector / Bitmap / Video`)。规则写死在这里:
    /// 按**整个文件**里出现过哪些 composition 类型来分档。
    ///
    /// # 它**不**回答的问题
    ///
    /// **`kind() == FileKind::Bitmap` 不等于"这个文件能靠重放序列还原"。**
    /// 一个矢量根 composition 引用位图子 composition 的混合档,
    /// 这里同样返回 `Bitmap` —— 而那个矢量根可以对子档施加变换 / 蒙版 /
    /// 混合模式,这些全在 `LayerBlock` 里,本 crate **有意不解析 `LayerBlock`**,
    /// 因此在**原理上**无从判断能不能忽略它们。
    ///
    /// 要那个判断,用 [`Self::is_pure_bitmap`]。
    ///
    /// (之所以不干脆改成看主 composition 的类型:那样混合档会被误判成纯矢量档,
    /// 更糟 —— 会漏掉"文件里其实有现成帧数据"这个事实。两个问题分成两个方法答。)
    pub fn kind(&self) -> FileKind {
        let has_bitmap = self
            .compositions
            .iter()
            .any(|c| c.kind == CompositionKind::Bitmap);
        let has_video = self
            .compositions
            .iter()
            .any(|c| c.kind == CompositionKind::Video);
        match (has_bitmap, has_video) {
            (true, true) => FileKind::Mixed,
            (true, false) => FileKind::Bitmap,
            (false, true) => FileKind::Video,
            (false, false) => FileKind::Vector,
        }
    }

    /// 遍历文件里所有位图序列(可能来自多个 composition;
    /// 同一个 composition 也可能有多档不同分辨率的序列)。
    ///
    /// ⚠️ 这些序列**不一定属于主 composition** —— 见 [`Self::attributes`] 上的警告。
    pub fn bitmap_sequences(&self) -> impl Iterator<Item = &BitmapSequence<'a>> {
        self.compositions
            .iter()
            .flat_map(|c| c.bitmap_sequences.iter())
    }

    /// 这个文件能不能**只靠重放位图序列**还原画面 —— 也就是「c2 不需要 libpag」
    /// 这个论点唯一站得住的判据。
    ///
    /// 判据:文件里**每一个** composition 都是 [`CompositionKind::Bitmap`]
    /// (且至少有一个)。此时不存在任何矢量图层能对序列施加变换、蒙版或混合,
    /// 主 composition 的序列逐帧重放**就是**整个文件的画面。
    ///
    /// # 为什么不能用 `kind() == FileKind::Bitmap` 代替
    ///
    /// 因为它分不出这两种文件:
    ///
    /// | 文件 | `kind()` | `is_pure_bitmap()` |
    /// |---|---|---|
    /// | 只有一个 `BitmapCompositionBlock` | `Bitmap` | `true` |
    /// | 矢量根 + 位图子 composition | `Bitmap` | `false` |
    ///
    /// 第二种里,`attributes()`(根的)和 `bitmap_sequences()`(子档的)
    /// 根本不是同一个 composition 的数据,而根可以对子档做什么,
    /// 我们**不解析 `LayerBlock`,查不到**。
    ///
    /// 所以这条判据取最保守的一档:**只要文件里出现过任何矢量或视频
    /// composition,就返回 `false`**,哪怕它其实是个空壳。宁可少支持。
    pub fn is_pure_bitmap(&self) -> bool {
        !self.compositions.is_empty()
            && self
                .compositions
                .iter()
                .all(|c| c.kind == CompositionKind::Bitmap)
    }
}

/// 文件里出现过哪些序列类型。判定规则见 [`PagFile::kind`] —— 是本 crate 的约定,
/// 不是官方定义。
///
/// **这几个变体说的是"文件里有什么",不是"文件能怎么画"。**
/// `Bitmap` 既可能是纯位图档,也可能是矢量根套位图子 composition ——
/// 要区分请用 [`PagFile::is_pure_bitmap`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// 没有任何位图序列或视频序列 composition。
    /// 本 crate **不解析矢量图层**,只能告诉你它是这一档。
    Vector,
    /// 含位图序列帧 composition —— 帧字节能取出来。
    /// **可能同时还有矢量 composition**,见类型级说明。
    Bitmap,
    /// 含视频序列 composition(H.264)。本 crate **不解析**视频序列。
    Video,
    /// 位图序列与视频序列都有。
    Mixed,
}

/// composition 的类型,由引入它的块标签决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompositionKind {
    /// `VectorCompositionBlock`(标签 2)
    Vector,
    /// `BitmapCompositionBlock`(标签 45)
    Bitmap,
    /// `VideoCompositionBlock`(标签 50)
    Video,
}

/// 一个 composition。
#[derive(Clone, Debug, PartialEq)]
pub struct Composition<'a> {
    /// composition 的引用 ID(块体的第一个字段)
    pub id: u32,
    pub kind: CompositionKind,
    /// 来自 `CompositionAttributes`(标签 3)。
    ///
    /// **`PagFile::parse` 返回的值里一定是 `Some`** —— 缺这个标签的文件通不过
    /// libpag 的 `Composition::verify()`(宽高默认 0),`parse` 会报
    /// `VerifyFailed { reason: MissingCompositionAttributes }`。
    /// 类型仍是 `Option` 是为了让"标签流里没读到"这个中间状态可表达,
    /// 不是给调用方留一个真会发生的分支。
    pub attributes: Option<CompositionAttributes>,
    /// 位图序列(标签 46)。一个 composition 可以有多档不同分辨率的序列,
    /// libpag 按 width 升序存(`BitmapCompositionTag.cpp` 的 `lessFirst`)。
    pub bitmap_sequences: Vec<BitmapSequence<'a>>,
    /// 见到但**没有解析**的 `VideoSequence`(标签 51)个数。
    ///
    /// 视频序列内部是 H.264 NAL(libpag 有 `src/codec/utils/NALUReader`),
    /// 其字段布局本 crate **未核实**,因此只计数、不解析。
    pub video_sequence_count: usize,
    /// 只有 `VideoCompositionBlock` 有:块体里 id 之后紧跟的那个 `Boolean`。
    pub has_alpha: Option<bool>,
}

/// `CompositionAttributes`(标签 3)的全部字段。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositionAttributes {
    pub width: i32,
    pub height: i32,
    /// 时长,单位是**帧**(libpag 的 `Frame` 类型),不是毫秒。
    pub duration_frames: u64,
    pub frame_rate: f32,
    /// 背景色 RGB。注意只有三个分量 —— `ReadColor` 读 3 个 uint8,没有 alpha。
    pub background_color: [u8; 3],
}

impl CompositionAttributes {
    /// 时长(秒)= 帧数 / 帧率。帧率非正时返回 `None`(不给自己留除零)。
    pub fn duration_seconds(&self) -> Option<f64> {
        if self.frame_rate > 0.0 {
            Some(self.duration_frames as f64 / f64::from(self.frame_rate))
        } else {
            None
        }
    }
}

/// 一档位图序列帧(`BitmapSequence`,标签 46)。
///
/// **帧之间是差分的**,不能单独解第 N 帧 —— 见 [`BitmapSequence::start_frame_for`]。
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapSequence<'a> {
    /// 这一档序列的画布宽度(可能小于 composition 宽度:导出时会按缩放出多档)
    pub width: i32,
    pub height: i32,
    pub frame_rate: f32,
    pub frames: Vec<BitmapFrame<'a>>,
}

impl<'a> BitmapSequence<'a> {
    /// 要画出第 `target` 帧,必须从哪一帧开始按顺序重放。
    ///
    /// **这是序列帧档最容易用错的一条语义。** PAG 的位图序列不是"每帧一张完整
    /// 图片",而是**关键帧 + 脏矩形差分**:导出插件算出与上一帧的差异矩形,
    /// 只编码那一块(`exporter/src/export/sequence/BitmapSequence.cpp` 里的
    /// `diffRect` / `ExpandRectRange`)。
    ///
    /// 播放侧的复原算法(`src/rendering/sequences/BitmapSequenceReader.cpp`):
    /// 从 `target` 往回找最近的关键帧,然后从它开始逐帧把每个 `BitmapRect`
    /// 按 `(x, y)` 贴到画布上。关键帧那一帧如果它的第一块尺寸小于画布,
    /// 先清屏再贴。
    ///
    /// 注意 libpag 的实现还带一个"上一次解到哪"的缓存快进
    /// (`frame == lastDecodeFrame + 1` 也能当起点),那是播放器的优化,
    /// 与容器格式无关,所以这里只给纯粹的"最近关键帧"语义。
    ///
    /// **与 libpag 的一处有意偏离**(README 的差异表里也有一行):
    /// 找不到任何关键帧时我们返回 `None`,而 libpag 的 `findStartFrame`
    /// (`BitmapSequenceReader.cpp:113-122`)把 `startFrame` 初始化为 `0` 再循环,
    /// 循环不命中就**从第 0 帧开始重放**。真实文件的第 0 帧必是关键帧,
    /// 所以这条差异在合法输入上不可见;但拿 0 硬顶等于对畸形序列悄悄编一个答案,
    /// 我们宁可让调用方看见 `None`。
    pub fn start_frame_for(&self, target: usize) -> Option<usize> {
        if target >= self.frames.len() {
            return None;
        }
        (0..=target).rev().find(|&i| self.frames[i].is_keyframe)
    }

    /// 时长(秒)。帧率非正时返回 `None`。
    pub fn duration_seconds(&self) -> Option<f64> {
        if self.frame_rate > 0.0 {
            Some(self.frames.len() as f64 / f64::from(self.frame_rate))
        } else {
            None
        }
    }

    /// 这一档序列里所有帧块用到的编码(去重后,按首次出现顺序)。
    ///
    /// 正常导出应该全是 `Webp`。出现别的说明素材来路特殊,值得告警。
    pub fn encodings(&self) -> Vec<ImageEncoding> {
        let mut out: Vec<ImageEncoding> = Vec::new();
        for frame in &self.frames {
            for bitmap in &frame.bitmaps {
                let e = bitmap.encoding();
                if !out.contains(&e) {
                    out.push(e);
                }
            }
        }
        out
    }
}

/// 序列里的一帧。
#[derive(Clone, Debug, PartialEq)]
pub struct BitmapFrame<'a> {
    /// 是否关键帧。关键帧不依赖前一帧;非关键帧只带脏矩形。
    pub is_keyframe: bool,
    /// 这一帧要贴的块。**可能为空**(整帧与上一帧无差异)。
    pub bitmaps: Vec<BitmapRect<'a>>,
}

/// 一帧里的一个块:贴到画布 `(x, y)` 处的一张编码图片。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BitmapRect<'a> {
    /// 贴图左上角在画布上的 X
    pub x: i32,
    /// 贴图左上角在画布上的 Y
    pub y: i32,
    /// **编码后的**图片字节(零拷贝借用)。宽高在这段字节自己的头里,
    /// PAG 容器**不**单独存块的宽高 —— libpag 也是解码后才知道
    /// (`codec->width()` / `codec->height()`)。
    ///
    /// 本 crate **不解码**:上层拿这段字节喂自己的图片解码器。
    pub bytes: &'a [u8],
}

impl BitmapRect<'_> {
    /// 嗅探这段字节是什么图片编码。
    ///
    /// AE 导出插件写的是 **WebP**(`exporter/src/export/sequence/BitmapSequence.cpp`
    /// `#include <webp/encode.h>`,失败时报 `AlertInfoType::WebpEncodeError`)。
    /// 但**容器本身不记录编码** —— 播放侧走的是泛用嗅探
    /// (`tgfx::ImageCodec::MakeFrom(imageBytes)`),所以我们也嗅探而不是假定。
    pub fn encoding(&self) -> ImageEncoding {
        ImageEncoding::sniff(self.bytes)
    }
}

/// 帧字节的图片编码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageEncoding {
    /// RIFF/WEBP 容器
    Webp,
    Png,
    Jpeg,
    /// 长度为 0。
    ///
    /// ⚠️ **容器解析路径永远不会产出它** —— 零长度块在 libpag 里是
    /// **文件级致命错误**(`readByteData` 对 `length == 0` 返回 `nullptr` →
    /// `BitmapFrame::verify()` 失败 → 整个文件被拒),我们照做,
    /// 见 [`crate::VerifyFailure::EmptyFrameBytes`]。
    /// 这个变体只可能来自直接调用 [`ImageEncoding::sniff`] 传空切片。
    ///
    /// (早期版本这里写的是"libpag 把这当空帧,是合法数据不是错误" ——
    /// 那是把 `BitmapSequenceReader.cpp:78` 的注释读串了:那句
    /// "The returned image could be nullptr if the frame is an empty frame."
    /// 注的是**上一行 `ImageCodec::MakeFrom` 的返回值**,不是零长度 `ByteData`。)
    Empty,
    /// 认不出来。**不要猜**,原样交给上层。
    Unknown,
}

impl ImageEncoding {
    /// 按魔数嗅探。
    ///
    /// - WebP:`"RIFF" ???? "WEBP"`,共 12 字节的 RIFF 头。
    ///   libpag `src/codec/utils/WebpDecoder.h` 的
    ///   `#define RIFF_HEADER_SIZE 12  // Size of the RIFF header ("RIFFnnnnWEBP").`
    /// - PNG:8 字节签名 `89 50 4E 47 0D 0A 1A 0A`(PNG 规范)
    /// - JPEG:SOI 标记 `FF D8 FF`
    pub fn sniff(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::Empty;
        }
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return Self::Webp;
        }
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
            return Self::Png;
        }
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Self::Jpeg;
        }
        Self::Unknown
    }
}
