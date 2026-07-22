//! Painter 抽象 —— 可切换渲染后端的边界(调研 14 裁决落地)。
//!
//! 设计要点:
//! - **trait 即时调用**为接口形态;[`RecordingPainter`] 是它的显示列表实现,
//!   免费获得金样测试(零像素、零 GPU),未来可升级为帧间 diff 载体;
//! - 词汇对齐 vello `Scene` 的动词(fill/stroke/glyph run/layer),M2 接
//!   vello 时 1:1 映射;
//! - **文本走定位好的 glyph run**:shaping 在上层(render 的 shape_text),
//!   光栅在 backend 内(CPU 端按 [`GlyphKey`] 走 swash 光栅,GPU 端走
//!   draw_glyphs)——painter 不拿字符串也不拿位图(Slint 软件渲染器与
//!   GPU 灾难的双重教训);
//! - `dyn` 只存在于 sv-shell 边界内,严禁类型参数上浮到 sv-ui/编译器产物
//!   (tachys 泛型爆炸的教训;这里每帧低千级动态调用 ≈ 个位数 µs)。
//!
//! 坐标:物理像素(调用方已乘 scale)。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sv_ui::Color;
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, Paint, PathBuilder, Pattern, Pixmap, PixmapRef,
    PremultipliedColorU8, SpreadMode, Stroke, Transform,
};

/// 路径命令(**自有轻量类型,刻意不借 kurbo/peniko**)。
///
/// 为什么不直接用 kurbo 的 `BezPath`:vello 在本仓库是 **optional dependency**
/// (`backend-vello` feature,默认关),而 Painter 是 CPU 后端也要实现的接口 ——
/// 让接口签名依赖只在某个 feature 下存在的类型,等于把 GPU 后端焊死进 CPU 路径。
/// ADR-3b 的"词汇对齐 vello Scene"说的是**动词形状**对齐,不是类型对齐。
///
/// 坐标同其它动词:物理像素(调用方已乘 scale)。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// 二次贝塞尔:(控制点, 终点)
    QuadTo(f32, f32, f32, f32),
    /// 三次贝塞尔:(控制点 1, 控制点 2, 终点)。SVG/Lottie 的主力曲线
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// 填充规则。SVG/Lottie 两种都用得到(`fill-rule: nonzero|evenodd`),
/// 缺了 EvenOdd 的话"带孔的图标"会被填成实心 —— 这是最常见的图标渲染 bug
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PathFill {
    #[default]
    NonZero,
    EvenOdd,
}

/// 线端形状(SVG `stroke-linecap` / lottie 同名属性)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// 折点形状(SVG `stroke-linejoin`)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// 描边风格。**这一个动词打包成结构体**,与本 trait "参数不打包"的惯例
/// (见 `stroke_rounded_rect` 的注释)相反 —— 差异是有意的:
/// vello 的 `Scene::stroke` 收 `&kurbo::Stroke`、tiny-skia 的 `stroke_path`
/// 收 `&tiny_skia::Stroke`,**打包才是"对齐后端词汇"**,不打包反而要在
/// 每个后端里现场拼一个结构体。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    /// 斜接上限(超过就退化成 Bevel);SVG 缺省 4.0
    pub miter_limit: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            cap: LineCap::default(),
            join: LineJoin::default(),
            miter_limit: 4.0,
        }
    }
}

// ---------------------------------------------------------------------------
// 位图载体
// ---------------------------------------------------------------------------

/// 图像 id 分配器。进程内单调自增,0 保留给"未分配"。
///
/// 用 `AtomicU64` 而不是 `Cell`:响应式运行时是单线程的,但**图片解码天然
/// 想放线程池**(几十毫秒的 PNG 解码放在 UI 线程上会直接掉帧),载体本身
/// 不该把这条路堵死。一次 relaxed fetch_add 相对一次解码是零成本
static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

/// 后端无关的像素载体:**预乘 RGBA8**,行主序,`stride == width * 4`。
///
/// # 为什么是 `Arc<[u8]>` + 稳定 id(裁决与代价)
///
/// 位图动词和 `fill_path` 那种纯几何动词的根本差别是**它带资源**。
/// 一张 1920×1080 RGBA 是 8.29 MB;按 60fps 每帧重传就是 **498 MB/s** 的
/// 纯拷贝 —— lottie/PAG 是逐帧画的,这条路必须从类型上堵死,而不是靠调用
/// 方自觉。所以:
///
/// - **像素共享而非拷贝**:`Arc<[u8]>`,clone 只动引用计数。载体放进场景树、
///   跨帧存活、被多个节点引用都不产生像素拷贝。
/// - **自带稳定 id,后端按 id 缓存**。这一条是被 vello 的真实实现逼出来的:
///   vello 的图集residency 表是按 `Blob::id()` 索引的
///   (vello_encoding 0.9 `image_cache.rs:114` `self.map.entry(image.data.id())`),
///   而 `Blob::new` **每次调用都从全局计数器领一个新 id**
///   (linebender_resource_handle 0.1.1 `blob.rs:95` `ID_COUNTER.fetch_add`)。
///   也就是说"每帧现造一个 Blob"= 每帧把整张图重新上传进图集。
///   载体带 id、后端拿 id 做缓存键,这个坑才关得掉。
/// - **内容 hash 在构造时算一次**([`PixelImage::content_hash`]),不在绘制
///   时算。量级是**毫秒**:1080p 的 8.29 MB 在本机量到 1.0–1.6 ms,
///   随构建配置浮动(仓库没有 `[profile.test]`,`cargo test` 走
///   `[profile.dev] opt-level = 1`;独立 `rustc -O` 更快)。
///   加载时付一次没问题,每帧付一次不行。
///   **这两个数字是单机单次采样,不是回归基线** —— 要守性能请写 benchmark,
///   引用这行注释当基线只会得到假阳性。
///
/// 代价,写明白:
/// - id 是**身份**不是内容摘要。同样的像素构造两次会得到两个 id → 后端会
///   缓存两份。逐帧动画每帧换内容 = 每帧换 id = 每帧重传,这是 vello 图集
///   模型的固有成本,不是本类型的缺陷(要绕开只能走 `external_texture`
///   那条 GPU→GPU 通道,见 [`PainterCaps::external_texture`])。
/// - `Arc` 的原子计数在单线程下是白付的,但只在 clone 时付,不在绘制时付。
///
/// # 预乘(**最容易错的一处**)
///
/// 字节必须是**预乘**的:`r ≤ a && g ≤ a && b ≤ a`。tiny-skia 的
/// `PixmapRef::from_bytes` 文档原文:"The `data` is assumed to have
/// premultiplied RGBA pixels (byteorder: RGBA)"(tiny-skia 0.11.4
/// `pixmap.rs:298`),它**不做校验**,喂直通 alpha 进去会得到 r>a 的非法
/// 像素,表现为半透明区域整体发白/发灰。PNG 解码出来的是**直通** alpha,
/// 走 [`PixelImage::from_straight_alpha`] 转换,别自己乘。
#[derive(Clone)]
pub struct PixelImage {
    id: u64,
    width: u32,
    height: u32,
    /// 内容 hash(构造时算一次)
    hash: u64,
    /// 预乘 RGBA8,`len == width * height * 4`(**严格相等**,见
    /// [`image_byte_len`] 与 [`PixelImage::new`] 里"为什么不是至少这么多")
    pixels: Arc<[u8]>,
}

/// 尺寸 → 字节数;溢出或零尺寸返回 `None`。
///
/// **三处口径的唯一来源**:[`PixelImage::new`] /
/// [`PixelImage::from_straight_alpha`] / [`PixelImage::valid_len`]。
///
/// 抽出来不只是去重 —— 是为了让"溢出必须拒绝"这条**可测**。内联在 `new`
/// 里的时候,一个 wrapping 实现对 `u32::MAX × u32::MAX × 4` 算出来的
/// 18446744073675997188 会被紧跟着的 `pixels.len() != need` 顺手挡掉,于是
/// `new(u32::MAX, u32::MAX, …).is_none()` 对 checked / wrapping **两种实现
/// 给出同一个结果** —— 一条恒真断言,看着像在守溢出,其实什么也没守。
/// 直接断言这个函数返回 `None` 才分得开(见
/// `image_byte_len_refuses_to_overflow`)
fn image_byte_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
        .filter(|&n| n > 0)
}

/// 丢图留痕。**为什么必须有这一条**:
/// `docs/plans/pag-2-integration.md` §6.2 的裁决标题原文是
/// "运行期只保留可查询,不做静默跳过",给的手段是 `PainterCaps` 增一位
/// `image: bool`。这里**没有加那个位**,理由见 [`PainterCaps`] 的文档
/// (它对三个后端恒 true,是纯噪音);但裁决要解决的问题是真的 ——
/// 位图动词一旦不画就是**整块内容消失**,而消失得毫无痕迹。
/// 这条 warn 是那个位的替代品,覆盖"**我们自己**决定不画"的分支。
///
/// 刻意**不覆盖**退化 dst(`w<=0`/`h<=0`):零尺寸矩形是日常合法情形
/// (折叠的 flex 项、滚出视口的节点),打进去只会把日志淹掉。
///
/// 为什么是 `eprintln!` 而不是 `log::warn!`(tiny-skia 自己在
/// `shaders/pattern.rs:101` 用的是后者):sv-shell 没有 `log` 依赖,
/// 为一条诊断引一个门面依赖是另一个裁决;壳层现有的错误路径
/// (`vello_backend.rs` 的 render 失败)也都是 eprintln
pub(crate) fn warn_dropped_image(why: &str, img: &PixelImage) {
    eprintln!(
        "sv-shell: draw_image 丢弃一张图({why}):声称 {}×{},实有 {} 字节",
        img.width,
        img.height,
        img.pixels.len()
    );
}

impl PixelImage {
    /// 从**已预乘**的 RGBA8 字节建一张图。
    ///
    /// 零宽/零高、或字节数不**恰好**等于 `width * height * 4`(含尺寸相乘
    /// 溢出)一律返回 `None` —— 这是"优雅拒绝"的第一道闸:非法的
    /// [`PixelImage`] 压根构造不出来,后端就不必在热路径上反复防御。
    ///
    /// 为什么要求**严格相等**而不是"至少这么多":GPU 后端把整个缓冲原样
    /// 交给 `wgpu::Queue::write_texture`(vello 0.9 `wgpu_engine.rs:515-522`,
    /// `bytes_per_row = width * 4`),多出来的尾巴没有任何语义却会变成
    /// "到底传多少"的悬念;而 stride ≠ `width*4` 的带填充缓冲本来就不在
    /// 本类型的契约内。严格相等让三个后端口径完全一致。
    ///
    /// **传 `Vec<u8>` 有一次整块拷贝**(`Vec<u8> → Arc<[u8]>` 的 `From` 就是
    /// 重新分配 + memcpy)。这次拷贝与 hash 是同一量级(1080p 本机各 1–2 ms),
    /// 也就是说 `new` 的成本大致就是"一次 hash + 一次 memcpy"。解码器如果
    /// 本来就能直接产出 `Arc<[u8]>`,传它可以把拷贝那一半省掉。
    /// 量级参考,不是基线 —— 同 [`PixelImage`] 类型文档里的那句提醒。
    pub fn new(width: u32, height: u32, pixels: impl Into<Arc<[u8]>>) -> Option<Self> {
        let pixels: Arc<[u8]> = pixels.into();
        let need = image_byte_len(width, height)?;
        if pixels.len() != need {
            return None;
        }
        Some(Self {
            id: NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            hash: content_hash(&pixels[..need]),
            pixels,
        })
    }

    /// 从**直通(straight/unpremultiplied)alpha** 的 RGBA8 建图,顺手预乘。
    ///
    /// 存在的理由:PNG/JPEG 解码器吐的都是直通 alpha,而三个后端要的都是
    /// 预乘。不给这条路,每个调用方都要自己写一遍乘法,而**写反一次就是
    /// 一个几乎看不出来、只在半透明边缘发白的 bug**。宁可在这里多一次
    /// 整图分配(只在加载时付一次)
    pub fn from_straight_alpha(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let need = image_byte_len(width, height)?;
        if rgba.len() != need {
            return None;
        }
        let mut out = Vec::with_capacity(need);
        for p in rgba[..need].chunks_exact(4) {
            let a = p[3] as u32;
            // +127 四舍五入(等价 round(c*a/255));a==255 时恒等
            let m = |c: u8| (((c as u32 * a) + 127) / 255) as u8;
            out.extend_from_slice(&[m(p[0]), m(p[1]), m(p[2]), p[3]]);
        }
        Self::new(width, height, out)
    }

    /// 后端缓存键(见类型文档)。**刻意不进金样**:它是进程内自增计数器,
    /// 快照会随测试执行顺序漂移
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// 内容 hash(构造时算一次)。金样后端用它当身份 —— 内容一样的两张图
    /// hash 相同,金样才不会因为"又 new 了一张一样的图"而变
    pub fn content_hash(&self) -> u64 {
        self.hash
    }

    /// 预乘 RGBA8 字节。长度**恰好**是 `width * height * 4` ——
    /// 构造器只接受严格相等(见 [`PixelImage::new`]),没有"尾部多余字节"
    /// 这回事,调用方不需要自己再截一刀
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 共享底层缓冲(GPU 后端零拷贝上传用)
    pub fn shared_pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }

    /// 防御性尺寸校验:口径与 [`PixelImage::new`] 完全一致(严格相等),
    /// 通过则返回字节数。
    ///
    /// `new` 已经拦过一次,这里是**第二道闸** —— 字段是私有的,但同模块
    /// (含 `#[cfg(test)]` 子模块)仍能绕过构造器直接写结构体,而越界读一张
    /// 图是 UB 级事故。一次比较换这个保险很划算,三个后端都在开头调它
    pub(crate) fn valid_len(&self) -> Option<usize> {
        image_byte_len(self.width, self.height).filter(|&n| self.pixels.len() == n)
    }

    /// **只给测试**:绕过构造器造一个畸形载体(声称 `w×h`,字节数不符)。
    ///
    /// 为什么要有这个函数,而不是让每条测试自己写结构体字面量:字段私有于
    /// 本模块,`vello_backend.rs` 的测试**写不出**那个字面量 —— 于是 GPU 端
    /// 那道 `valid_len` 闸整整一轮没有任何测试驱动过,而它挡的是 vello 0.9
    /// 在图集上传处对空图的**硬 panic**(`wgpu_engine.rs:507-513`
    /// `Tried to draw an invalid empty image`)。闸门失效 = 崩窗口,
    /// 这种分支不能靠"读代码确认它写了"
    #[cfg(test)]
    pub(crate) fn bogus_for_test(width: u32, height: u32, pixels: &[u8]) -> Self {
        Self {
            id: 0,
            width,
            height,
            hash: 0,
            pixels: Arc::from(pixels),
        }
    }
}

impl std::fmt::Debug for PixelImage {
    /// 手写:derive 会把整个像素缓冲打进 `{:?}`,一张 1080p 图能刷屏几 MB
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PixelImage")
            .field("id", &self.id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("hash", &format_args!("{:#018x}", self.hash))
            .field("bytes", &self.pixels.len())
            .finish()
    }
}

/// 内容 hash:FNV-1a 的**按 8 字节字**变体(非密码学,只做身份)。
///
/// 为什么不用 `std::collections::hash_map::DefaultHasher`:std 明确不保证
/// 它的算法跨 Rust 版本稳定,而这个值要进**金样快照** —— 换个工具链就整片
/// 变红,是最恶心的一类假阳性。为什么不逐字节:逐字节 FNV 对 1080p 的
/// 8.29 MB 要跑八百多万轮,按字之后只剩百万轮出头。
///
/// 长度也混进去:防"全零缓冲不同长度撞在一起"这类退化碰撞
fn content_hash(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        h ^= u64::from_le_bytes(c.try_into().expect("chunks_exact(8) 恒为 8 字节"));
        h = h.wrapping_mul(PRIME);
    }
    for &b in chunks.remainder() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h ^= bytes.len() as u64;
    h.wrapping_mul(PRIME)
}

/// 缩放滤镜裁决:**每轴 dst/src 都是整数倍(含 1:1)→ 最近邻,其余 → 双线性**。
///
/// - 不一律双线性:整数倍放大用双线性会把本该锐利的边糊成渐变(图标、
///   像素画、@1x 资源在整数 DPI 下放大都在这一档),而这一档恰好是唯一
///   能做到**逐像素可预测**的,测试也只有在这一档才断言得动;
/// - 不一律最近邻:非整数缩放(1.25/1.5 倍 DPI、照片按容器缩放)最近邻会
///   丢行丢列,肉眼可见的锯齿;
/// - 1:1 这一档其实是把 tiny-skia 的隐式行为显式化:它对"逆变换是纯平移"
///   的情形本来就强制降级成 Nearest(tiny-skia 0.11.4
///   `shaders/pattern.rs:112-114`),但依赖别人的内部降级不是接口该有的样子。
///
/// **已知缺口**:tiny-skia 没有 mipmap(同文件 `// TODO: minimizing scale via
/// mipmap`),缩小超过 2× 仍然会走样。大图缩小请在上层预缩放
/// 目标矩形可画吗?**两个后端共用这一个定义**,别各写各的。
///
/// 踩过的坑:第一版两边各写了 `w > 0.0 && h > 0.0 && x.is_finite() &&
/// y.is_finite()`,漏了 `w`/`h` 自己的有限性 —— `w = f32::INFINITY` 满足
/// `w > 0.0`,于是穿过闸门。CPU 端侥幸被 `tiny_skia::Rect::from_xywh` 的
/// 有限性检查兜住了,vello 端没人兜,直接把一个无穷大的 Affine 编进场景。
/// 这就是"同一条规则写两遍"的标准结局
pub(crate) fn dst_rect_drawable(x: f32, y: f32, w: f32, h: f32) -> bool {
    // `w > 0.0` 已排除 NaN 与负数,再加 is_finite 排除 +∞
    x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0
}

pub(crate) fn image_filter_nearest(sx: f32, sy: f32) -> bool {
    let integral = |s: f32| {
        let r = s.round();
        r >= 1.0 && (s - r).abs() <= 1e-3
    };
    integral(sx) && integral(sy)
}

/// 字形光栅键:字体身份 + 字形 id + 字号(f32 以位模式存储,保 Hash/Eq)。
/// 三项唯一决定一张覆盖度位图(HiDPI 已把 scale 乘进 px;调研 24 P0:
/// font_key 让 fallback 后同帧多字体的缓存不串位)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    /// 字体身份([`crate::text::FontHandle::key`];单字体阶段恒 0)
    pub font_key: u64,
    /// 字形 id(swash charmap 映射)
    pub id: u16,
    /// 字号的 f32 位模式(`f32::to_bits`)
    pub px_bits: u32,
}

impl GlyphKey {
    pub fn new(font: crate::text::FontHandle, id: u16, px: f32) -> Self {
        Self {
            font_key: font.key,
            id,
            px_bits: px.to_bits(),
        }
    }

    /// 字号(vello 端 `font_size` / CPU 端 scaler size 用)
    pub fn px(&self) -> f32 {
        f32::from_bits(self.px_bits)
    }
}

/// 一枚已定位字形(物理坐标)。
/// CPU 路径用 (key, x, y):光栅键 + **基线原点**(位图左上角由光栅返回的
/// Placement 换算:bitmap_x = x + left,bitmap_y = y - top);
/// GPU 路径用 (id, ox, oy):字形 id + 基线原点(vello draw_glyphs 语义)
#[derive(Clone, Copy, Debug)]
pub struct GlyphPos {
    pub key: GlyphKey,
    /// 基线原点 x(CPU 光栅路径)
    pub x: f32,
    /// 基线原点 y(CPU 光栅路径)
    pub y: f32,
    /// 字形 id(GPU 路径)
    pub id: u16,
    /// 基线原点 x(GPU 路径)
    pub ox: f32,
    /// 基线原点 y(GPU 路径)
    pub oy: f32,
}

impl GlyphPos {
    /// 字号(一段 run 内一致)
    pub fn px(&self) -> f32 {
        self.key.px()
    }
}

/// 后端能力协商(调研 15:为 3D 复合预留通道,避免 M2 设计堵路)
#[derive(Clone, Copy, Debug, Default)]
pub struct PainterCaps {
    /// 能否合成外部 wgpu 纹理(`<surface3d>` 的前置;CPU 后端恒 false)。
    ///
    /// **它和 [`Painter::draw_image`] 是两条不同的通道,不是强弱关系:**
    /// - `draw_image` 收的是 **CPU 侧字节**([`PixelImage`]),后端负责采样
    ///   (CPU 端直接读,GPU 端上传进图集)。方向是 CPU → 合成器,任何后端
    ///   都做得到,所以它是 trait 的**无条件义务**,不是能力位;
    /// - `external_texture` 说的是一张**已经在 GPU 上**的纹理(3D 场景的
    ///   渲染结果、将来硬件解码的视频帧)零拷贝进我们的合成器。方向是
    ///   GPU → 合成器,CPU 后端永远做不到,所以它必须是能力位。
    ///
    /// 换句话说:`draw_image` 落地**不改**这一位,也**不需要**新增一个
    /// `image: bool` —— 恒 true 的能力位是纯噪音,与 `fill_path` /
    /// `stroke_path` 同样没有能力位是一个道理。
    ///
    /// **这一条是对 `docs/plans/pag-2-integration.md` §6.2 的明知故犯**:
    /// 那节裁决"运行期只保留可查询,不做静默跳过",并点名要加 `image: bool`。
    /// 不加那个位的理由如上(`draw_image` 没有默认实现,谁都得实现,位恒
    /// true);但裁决**要解决的问题**没有被驳回 —— 它的替代品是
    /// `warn_dropped_image`:凡是后端自己决定不画的分支都留一行痕。
    /// 真到了有后端**画不了**位图的那天(候选:鸿蒙早期无 GPU 档),
    /// 这个位应当补上,那时它才不恒 true。
    pub external_texture: bool,
    /// 能否做高斯模糊(box-shadow/backdrop-filter 的前置)
    pub blur: bool,
}

/// 渲染后端要实现的最小指令集
pub trait Painter {
    /// 能力位(默认全 false;调用方按 caps 降级)
    fn caps(&self) -> PainterCaps {
        PainterCaps::default()
    }
    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color);
    /// 参数多但不打包成结构体:动词签名对齐 vello Scene,
    /// 换后端时一一对照;打包会在每帧热路径上多一次构造
    #[allow(clippy::too_many_arguments)]
    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        width: f32,
        color: Color,
    );
    /// 一段已定位字形(shaping 已完成;backend 只负责光栅/上屏)。
    /// run 级带字体身份(调研 24 P0):CPU 端按 GlyphKey.font_key 光栅,
    /// GPU 端按 handle 取/建 FontData——fallback 混排即同帧多次调用
    fn glyph_run(&mut self, font: crate::text::FontHandle, glyphs: &[GlyphPos], color: Color);
    /// 压入矩形裁剪(嵌套取交集;TextInput 溢出与滚动容器共用——调研 21/22。
    /// 物理像素坐标。radius:CPU 后端 v0 矩形近似(角部最多溢出 ~radius²px,
    /// 调研 22 §2.3 裁决),vello 端精确)
    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32);
    fn pop_clip(&mut self);

    /// 任意路径填充(SVG 图标 / Lottie 矢量动画的地基;调研 26 点名的
    /// "图标管线最大风险"就是缺这个动词)。
    ///
    /// **没有默认实现是刻意的**:给个 no-op 默认会让新后端"静默不画",
    /// 而漏画在自绘 UI 里极难定位 —— 宁可编译期逼着实现者面对它。
    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color);

    /// 任意路径描边。
    ///
    /// **为什么不省掉它**(描边完全可以在上层用轮廓展开成填充再走 `fill_path`):
    /// tiny-skia 与 vello **都有原生描边**,vello 侧还是 GPU 管线的一等公民;
    /// 上层展开等于在 CPU 上做两个后端本来都不用做的事,而且每帧都做。
    /// 何况 SVG 图标以描边为主(调研 26 §5:arco 图标 stroke 为主),
    /// 这条路径是热的。
    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color);

    /// 画一张位图([`PixelImage`],预乘 RGBA8)到目标矩形。
    ///
    /// # 目标矩形语义:**拉伸铺满,不保持比例,不裁剪**
    ///
    /// 源图整块 `(0,0,img.width,img.height)` 仿射映射到 `(x,y,w,h)`。
    /// 每轴独立缩放 —— 把一张正方形图画进 6×2 的框就会被压扁,这是**定义**,
    /// 不是缺陷。
    ///
    /// 为什么选它而不是 contain/cover:保持比例必然连带"多出来的地方摆哪、
    /// 底色是什么、超出的部分裁不裁"三个策略问题,而这三个都是**布局**的
    /// 决定(CSS 里 `object-fit`/`object-position` 就是布局属性,不是绘制
    /// 原语)。Painter 是原语层,拉伸是这里唯一无歧义、可组合的语义 ——
    /// 上层想要 contain,自己把 dst 算成居中的等比矩形再调这个动词即可。
    ///
    /// 坐标同其它动词:物理像素(调用方已乘 scale)。参数摊平不打包,与
    /// `fill_rounded_rect` / `push_clip` 一致(见 `stroke_rounded_rect` 注释)。
    ///
    /// # 退化输入
    ///
    /// `w <= 0`、`h <= 0`、非有限值、或 `img` 尺寸与字节数对不上时**静默丢弃**,
    /// 不 panic 不越界。零尺寸图在 [`PixelImage::new`] 就被拒了,这里是第二道闸。
    ///
    /// # 缩放质量
    ///
    /// 见 `image_filter_nearest`:整数倍(含 1:1)最近邻,其余双线性。
    ///
    /// **没有默认实现是刻意的**,与 `fill_path` / `stroke_path` 同一纪律:
    /// 位图是"画不出来就整块内容消失"的动词,静默 no-op 的默认实现会让新
    /// 后端悄悄丢图。
    fn draw_image(&mut self, x: f32, y: f32, w: f32, h: f32, img: &PixelImage);
}

// ---------------------------------------------------------------------------
// 记录型后端:命令快照(金样测试 / 未来缓存载体)
// ---------------------------------------------------------------------------

/// 简化命令(数值取整,快照稳定;字形只记数量与颜色)
#[derive(Clone, PartialEq, Debug)]
pub enum PaintCmd {
    FillRect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
        color: Color,
    },
    StrokeRect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        width: i32,
        color: Color,
    },
    Glyphs {
        count: usize,
        color: Color,
        /// 字体身份(对拍多字体 run 的发射顺序;单字体阶段恒 0)
        font_key: u64,
    },
    PushClip {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        radius: i32,
    },
    PopClip,
    /// 路径填充。金样只记**命令条数 + 规则 + 颜色 + 取整包围盒** ——
    /// 逐点记录会让金样长到没人看得懂,而包围盒足以抓住"画错位置/画歪了"
    Path {
        cmds: usize,
        fill: PathFill,
        color: Color,
        bbox: (i32, i32, i32, i32),
    },
    /// 路径描边(同上口径;`width` 取整,cap/join 直记)
    StrokePath {
        cmds: usize,
        width: i32,
        cap: LineCap,
        join: LineJoin,
        color: Color,
        bbox: (i32, i32, i32, i32),
    },
    /// 位图绘制。**像素一个字节都不进命令流** —— 一张 1080p 图进快照就是
    /// 8 MB 二进制垃圾,金样从此没人看得懂也没人愿意 review。
    ///
    /// 记三样:
    /// - `src`:源尺寸(抓"画错了哪张图"里的尺寸维度);
    /// - `hash`:[`PixelImage::content_hash`],**内容**身份。
    ///   刻意不记 `PixelImage::id` —— 那是进程内自增计数器,同一份内容在
    ///   不同测试顺序下会拿到不同 id,快照会无缘无故漂移;
    /// - `bbox`:目标矩形,取整口径与 `Path`/`StrokePath` 对齐
    ///   (`(left.floor, top.floor, right.ceil, bottom.ceil)`,保守外框)。
    Image {
        src: (u32, u32),
        hash: u64,
        bbox: (i32, i32, i32, i32),
    },
}

#[derive(Default)]
pub struct RecordingPainter {
    pub cmds: Vec<PaintCmd>,
}

impl Painter for RecordingPainter {
    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        self.cmds.push(PaintCmd::FillRect {
            x: x as i32,
            y: y as i32,
            w: w as i32,
            h: h as i32,
            radius: radius as i32,
            color,
        });
    }

    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        width: f32,
        color: Color,
    ) {
        let _ = radius;
        self.cmds.push(PaintCmd::StrokeRect {
            x: x as i32,
            y: y as i32,
            w: w as i32,
            h: h as i32,
            width: width as i32,
            color,
        });
    }

    fn glyph_run(&mut self, font: crate::text::FontHandle, glyphs: &[GlyphPos], color: Color) {
        self.cmds.push(PaintCmd::Glyphs {
            count: glyphs.len(),
            color,
            font_key: font.key,
        });
    }

    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        self.cmds.push(PaintCmd::PushClip {
            x: x as i32,
            y: y as i32,
            w: w as i32,
            h: h as i32,
            radius: radius as i32,
        });
    }

    fn pop_clip(&mut self) {
        self.cmds.push(PaintCmd::PopClip);
    }

    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        self.cmds.push(PaintCmd::Path {
            cmds: path.len(),
            fill,
            color,
            bbox: path_bbox_i32(path),
        });
    }

    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        self.cmds.push(PaintCmd::StrokePath {
            cmds: path.len(),
            width: style.width as i32,
            cap: style.cap,
            join: style.join,
            color,
            bbox: path_bbox_i32(path),
        });
    }

    fn draw_image(&mut self, x: f32, y: f32, w: f32, h: f32, img: &PixelImage) {
        // 记录型后端**不做**退化剔除:它的职责是"调用方到底要求画了什么",
        // 一次退化的 draw_image 恰恰是金样最该抓住的东西。
        // `f32 as i32` 在 Rust 里是饱和转换(NaN → 0),不会 UB
        self.cmds.push(PaintCmd::Image {
            src: (img.width, img.height),
            hash: img.hash,
            bbox: (
                x.floor() as i32,
                y.floor() as i32,
                (x + w).ceil() as i32,
                (y + h).ceil() as i32,
            ),
        });
    }
}

/// 路径包围盒(取整)。控制点也计入 —— 曲线不会超出控制点凸包,
/// 所以这是个**保守**包围盒:金样用它抓"画错位置",宁可大不可小
fn path_bbox_i32(path: &[PathCmd]) -> (i32, i32, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut hit = |x: f32, y: f32| {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    };
    for c in path {
        match *c {
            PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => hit(x, y),
            PathCmd::QuadTo(cx, cy, x, y) => {
                hit(cx, cy);
                hit(x, y);
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                hit(c1x, c1y);
                hit(c2x, c2y);
                hit(x, y);
            }
            PathCmd::Close => {}
        }
    }
    if x0 > x1 {
        return (0, 0, 0, 0);
    }
    (
        x0.floor() as i32,
        y0.floor() as i32,
        x1.ceil() as i32,
        y1.ceil() as i32,
    )
}

// ---------------------------------------------------------------------------
// tiny-skia CPU 后端(首个真实实现;能力冻结,定位过渡与测试基准)
// ---------------------------------------------------------------------------

pub struct TinySkiaPainter<'a> {
    pixmap: &'a mut Pixmap,
    /// 累积交集后的裁剪矩形栈(物理像素;top 即当前生效裁剪)。
    /// v0 裁决(调研 22 §2.3):手动矩形交集,不用 tiny-skia Mask——
    /// Mask 每层要分配整画布 w×h 字节且嵌套逐像素相乘,与 CPU 栈能力
    /// 冻结(ADR-3b)相悖;圆角裁剪为矩形近似(角部最多溢出 ~radius²px)
    clips: Vec<[f32; 4]>,
}

impl<'a> TinySkiaPainter<'a> {
    pub fn new(pixmap: &'a mut Pixmap) -> Self {
        Self {
            pixmap,
            clips: Vec::new(),
        }
    }

    /// 绘制矩形与当前裁剪求交;None = 完全被裁掉
    fn clipped(&self, x: f32, y: f32, w: f32, h: f32) -> Option<(f32, f32, f32, f32)> {
        match self.clips.last() {
            Some([cx, cy, cw, ch]) => {
                let x0 = x.max(*cx);
                let y0 = y.max(*cy);
                let x1 = (x + w).min(cx + cw);
                let y1 = (y + h).min(cy + ch);
                (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
            }
            None => Some((x, y, w, h)),
        }
    }
}

/// 字形覆盖度缓存(线程级):同一字形同字号只光栅一次。
/// swash ScaleContext 线程级复用(其内部按 CacheKey 缓存字体状态)。
/// 上限按条目数粗控(每条 ≈ 字号² 字节;2048 条 @16px ≈ 1.3MB)
mod glyph_cache {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use swash::scale::{Render, ScaleContext, Source};
    use swash::zeno::{Format, Placement};

    use super::GlyphKey;

    const CAP: usize = 2048;

    thread_local! {
        static CTX: RefCell<ScaleContext> = RefCell::new(ScaleContext::new());
        static HOT: RefCell<HashMap<GlyphKey, (Placement, Vec<u8>)>> =
            RefCell::new(HashMap::new());
        static COLD: RefCell<HashMap<GlyphKey, (Placement, Vec<u8>)>> =
            RefCell::new(HashMap::new());
    }

    fn rasterize(key: GlyphKey) -> (Placement, Vec<u8>) {
        CTX.with(|ctx| {
            let mut ctx = ctx.borrow_mut();
            // 按字形键里的字体身份取 FontRef(调研 24 P0;单字体阶段即 UI 字体)
            let font = crate::text::FontHandle { key: key.font_key }.font_ref();
            let mut scaler = ctx.builder(font).size(key.px()).hint(false).build();
            // Outline → alpha 覆盖度位图;Placement 的 top 是基线上方距离
            Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut scaler, key.id)
                .map(|img| (img.placement, img.data))
                .unwrap_or((
                    Placement {
                        left: 0,
                        top: 0,
                        width: 0,
                        height: 0,
                    },
                    Vec::new(),
                ))
        })
    }

    pub fn with<R>(key: GlyphKey, f: impl FnOnce(&Placement, &[u8]) -> R) -> R {
        HOT.with(|h| {
            let mut hot = h.borrow_mut();
            if !hot.contains_key(&key) {
                let entry = COLD
                    .with(|c| c.borrow_mut().remove(&key))
                    .unwrap_or_else(|| rasterize(key));
                // 分代淘汰:热代满则整代降为冷代(旧冷代随之丢弃)。
                // 活跃字形要么在热代、要么下次命中从冷代无成本晋升,
                // 单帧最多重光栅"整代未用"的字形——不会像整体清空那样
                // 把当前工作集也打掉(帧时长尖峰,伤 1% low)
                if hot.len() >= CAP {
                    let demoted = std::mem::take(&mut *hot);
                    COLD.with(|c| *c.borrow_mut() = demoted);
                }
                hot.insert(key, entry);
            }
            let (p, cov) = &hot[&key];
            f(p, cov)
        })
    }
}

fn skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn rounded_rect_path(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32, r: f32) {
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.5 {
        if let Some(rect) = tiny_skia::Rect::from_xywh(x, y, w, h) {
            pb.push_rect(rect);
        }
        return;
    }
    const K: f32 = 0.552_284_8;
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + K * r, y, x + w, y + r - K * r, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(
        x + w,
        y + h - r + K * r,
        x + w - r + K * r,
        y + h,
        x + w - r,
        y + h,
    );
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - K * r, y + h, x, y + h - r + K * r, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - K * r, x + r - K * r, y, x + r, y);
    pb.close();
}

fn blend_pixel(
    data: &mut [PremultipliedColorU8],
    pw: u32,
    ph: u32,
    x: i32,
    y: i32,
    c: Color,
    cov: u8,
) {
    if x < 0 || y < 0 || x >= pw as i32 || y >= ph as i32 {
        return;
    }
    let idx = (y as u32 * pw + x as u32) as usize;
    let dst = data[idx];
    let a = (cov as f32 / 255.0) * (c.a as f32 / 255.0);
    let inv = 1.0 - a;
    let na = (255.0 * a + dst.alpha() as f32 * inv).round().min(255.0);
    let nr = (c.r as f32 * a + dst.red() as f32 * inv).round().min(na);
    let ng = (c.g as f32 * a + dst.green() as f32 * inv).round().min(na);
    let nb = (c.b as f32 * a + dst.blue() as f32 * inv).round().min(na);
    if let Some(px) = PremultipliedColorU8::from_rgba(nr as u8, ng as u8, nb as u8, na as u8) {
        data[idx] = px;
    }
}

impl Painter for TinySkiaPainter<'_> {
    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let Some((x, y, w, h)) = self.clipped(x, y, w, h) else {
            return;
        };
        let mut pb = PathBuilder::new();
        rounded_rect_path(&mut pb, x, y, w, h, radius);
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(skia_color(color));
            paint.anti_alias = true;
            self.pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        width: f32,
        color: Color,
    ) {
        // 视口外整体剔除;部分越界时不几何裁剪(描边收缩会造出幻影边,
        // 允许出血,滚动容器边框在 push_clip 之外绘制,实践中罕见触发)
        if self.clipped(x, y, w, h).is_none() {
            return;
        }
        // 沿边框中心线描边(内缩半宽),视觉贴合 border-box
        let half = width / 2.0;
        let mut pb = PathBuilder::new();
        rounded_rect_path(
            &mut pb,
            x + half,
            y + half,
            w - width,
            h - width,
            (radius - half).max(0.0),
        );
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(skia_color(color));
            paint.anti_alias = true;
            let stroke = Stroke {
                width,
                ..Stroke::default()
            };
            self.pixmap
                .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    fn glyph_run(&mut self, _font: crate::text::FontHandle, glyphs: &[GlyphPos], color: Color) {
        // 字体身份已编进每个 GlyphKey(光栅缓存按其分桶),此处不需再用。
        // 字形走手动混合,mask 不经过 fill_path——用裁剪矩形逐像素判界
        let clip = self.clips.last().map(|c| {
            [
                c[0].floor() as i32,
                c[1].floor() as i32,
                (c[0] + c[2]).ceil() as i32,
                (c[1] + c[3]).ceil() as i32,
            ]
        });
        let (pw, ph) = (self.pixmap.width(), self.pixmap.height());
        let data = self.pixmap.pixels_mut();
        for g in glyphs {
            glyph_cache::with(g.key, |placement, coverage| {
                // 基线原点 → 位图左上角(top 是基线到位图顶的距离,向上为正)
                let x0 = g.x.round() as i32 + placement.left;
                let y0 = g.y.round() as i32 - placement.top;
                let (w, h) = (placement.width as usize, placement.height as usize);
                for yy in 0..h {
                    for xx in 0..w {
                        let cov = coverage[yy * w + xx];
                        if cov == 0 {
                            continue;
                        }
                        let (px, py) = (x0 + xx as i32, y0 + yy as i32);
                        if let Some([cx0, cy0, cx1, cy1]) = clip
                            && (px < cx0 || px >= cx1 || py < cy0 || py >= cy1)
                        {
                            continue;
                        }
                        blend_pixel(data, pw, ph, px, py, color, cov);
                    }
                }
            });
        }
    }

    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, _radius: f32) {
        // radius 忽略:矩形近似(调研 22 §2.3;Mask 精确路线留作升级项)
        let rect = match self.clips.last() {
            Some([px, py, pw, ph]) => {
                let x0 = x.max(*px);
                let y0 = y.max(*py);
                let x1 = (x + w).min(px + pw);
                let y1 = (y + h).min(py + ph);
                [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
            }
            None => [x, y, w, h],
        };
        self.clips.push(rect);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        // finish() 对空路径/只有 MoveTo 的退化路径返回 None —— 静默跳过,
        // 这不是错误(SVG 里空 <path d=""> 合法)
        let Some(p) = build_sk_path(path) else { return };
        let mut paint = Paint::default();
        paint.set_color(skia_color(color));
        paint.anti_alias = true;
        // 裁剪:矩形裁剪走 Mask 太贵(见 push_clip 的矩形交集裁决),这里
        // 用 tiny-skia 的 clip_mask 参数传 None,靠调用方保证路径在裁剪内。
        // **已知缺口**:滚动容器内的路径图标不会被裁掉;等真有这个场景再补
        // (补法是把 clips 栈末项转成 Mask 传进来)
        self.pixmap.fill_path(
            &p,
            &paint,
            match fill {
                PathFill::NonZero => FillRule::Winding,
                PathFill::EvenOdd => FillRule::EvenOdd,
            },
            Transform::identity(),
            None,
        );
    }

    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        let Some(p) = build_sk_path(path) else { return };
        let mut paint = Paint::default();
        paint.set_color(skia_color(color));
        paint.anti_alias = true;
        let stroke = Stroke {
            width: style.width,
            miter_limit: style.miter_limit,
            line_cap: match style.cap {
                LineCap::Butt => tiny_skia::LineCap::Butt,
                LineCap::Round => tiny_skia::LineCap::Round,
                LineCap::Square => tiny_skia::LineCap::Square,
            },
            line_join: match style.join {
                LineJoin::Miter => tiny_skia::LineJoin::Miter,
                LineJoin::Round => tiny_skia::LineJoin::Round,
                LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
            },
            ..Stroke::default()
        };
        // 裁剪同 fill_path:见那边的已知缺口注释
        self.pixmap
            .stroke_path(&p, &paint, &stroke, Transform::identity(), None);
    }

    fn draw_image(&mut self, x: f32, y: f32, w: f32, h: f32, img: &PixelImage) {
        // 第二道尺寸闸(见 PixelImage::valid_len);dst 退化走共用判定
        let Some(len) = img.valid_len() else {
            // 留痕:构造器已经拦过一次,还能走到这里说明有人绕过了
            // `PixelImage::new`,不是正常输入(见 warn_dropped_image 的裁决)
            warn_dropped_image("载体尺寸与字节数不符", img);
            return;
        };
        // 退化 dst 刻意**不**留痕:零尺寸矩形是日常合法情形
        if !dst_rect_drawable(x, y, w, h) {
            return;
        }
        let Some(src) = PixmapRef::from_bytes(&img.pixels[..len], img.width, img.height) else {
            // valid_len 已保证"长度恰好 + 尺寸非零",所以 from_bytes 只可能
            // 因尺寸本身过大而失败(tiny-skia 0.11.4 `pixmap.rs:299-306`:
            // `IntSize::from_wh` / `data_len_for_size`,width 上限 i32::MAX/4)。
            // 这是"整张图凭空消失"里最难查的一种,必须留痕
            warn_dropped_image("tiny-skia 拒绝该尺寸(width 上限 i32::MAX/4)", img);
            return;
        };
        // 与裁剪栈求交:只缩**几何**,不动图案锚点 —— pattern 的 transform
        // 独立于填充矩形,所以裁掉右半边不会把左半边的采样也拽过去
        let Some((cx, cy, cw, ch)) = self.clipped(x, y, w, h) else {
            return;
        };
        let Some(rect) = tiny_skia::Rect::from_xywh(cx, cy, cw, ch) else {
            return;
        };

        // 源 → 目标的仿射(纯缩放 + 平移,拉伸铺满 dst)。
        // 这里刻意**不用** tiny-skia 的 `draw_pixmap`:它的目标矩形写死成
        // 源图尺寸(0.11.4 `painter.rs:478`,`pixmap.size().to_int_rect(x,y)`),
        // 缩放只能靠外层 transform 去拉整块几何,而那样就没法把矩形换成
        // "与裁剪求交后的矩形"。直接组 Pattern + fill_rect 是同一条代码路径
        // (draw_pixmap 内部就是这么干的),但几何归我们控
        let sx = w / img.width as f32;
        let sy = h / img.height as f32;
        let quality = if image_filter_nearest(sx, sy) {
            FilterQuality::Nearest
        } else {
            FilterQuality::Bilinear
        };
        let paint = Paint {
            // SpreadMode::Pad:采样越界时钳到边缘像素。用 Repeat 的话
            // 双线性在右/下边缘会把对侧像素混进来(经典"图片边缘漏出一条
            // 左边颜色"的 bug)
            shader: Pattern::new(
                src,
                SpreadMode::Pad,
                quality,
                1.0,
                Transform::from_row(sx, 0.0, 0.0, sy, x, y),
            ),
            blend_mode: BlendMode::SourceOver,
            // 与 tiny-skia 自己的 draw_pixmap 一致(0.11.4 `painter.rs:496`
            // 注释原文 "Skia doesn't use it too")。代价是分数坐标下边缘按
            // 像素中心取整,最多半像素误差;收益是相邻两张图之间不会出现
            // 半透明接缝 —— 图集/九宫格拼接时那条缝远比半像素位移显眼
            anti_alias: false,
            force_hq_pipeline: false,
        };
        self.pixmap
            .fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// PathCmd 序列 → tiny-skia Path。填充与描边共用,免得两处各写一遍
/// 然后慢慢长歪
fn build_sk_path(path: &[PathCmd]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for c in path {
        match *c {
            PathCmd::MoveTo(x, y) => pb.move_to(x, y),
            PathCmd::LineTo(x, y) => pb.line_to(x, y),
            PathCmd::QuadTo(cx, cy, x, y) => pb.quad_to(cx, cy, x, y),
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => pb.cubic_to(c1x, c1y, c2x, c2y, x, y),
            PathCmd::Close => pb.close(),
        }
    }
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 画一个正方形环:外圈顺时针 + 内圈**同向**。
    /// NonZero 下绕数处处非零 → 实心;EvenOdd 下内圈区域绕数为 2 → 挖空。
    /// 这是图标渲染最经典的一处坑:填充规则弄错,所有"带孔图标"(圆环、
    /// 字母 O、回字形)会被填成实心块,而单看代码完全正常
    fn ring(inner: f32) -> Vec<PathCmd> {
        let mut p = vec![
            PathCmd::MoveTo(10.0, 10.0),
            PathCmd::LineTo(90.0, 10.0),
            PathCmd::LineTo(90.0, 90.0),
            PathCmd::LineTo(10.0, 90.0),
            PathCmd::Close,
        ];
        p.extend([
            PathCmd::MoveTo(inner, inner),
            PathCmd::LineTo(100.0 - inner, inner),
            PathCmd::LineTo(100.0 - inner, 100.0 - inner),
            PathCmd::LineTo(inner, 100.0 - inner),
            PathCmd::Close,
        ]);
        p
    }

    fn px(pm: &Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let p = pm.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue(), p.alpha())
    }

    #[test]
    fn fill_rule_nonzero_vs_evenodd() {
        let red = Color::rgb(255, 0, 0);
        // NonZero:同向内圈不挖孔 → 中心被填
        let mut pm = Pixmap::new(100, 100).unwrap();
        TinySkiaPainter::new(&mut pm).fill_path(&ring(30.0), PathFill::NonZero, red);
        assert_eq!(px(&pm, 50, 50).3, 255, "NonZero 下同向内圈不该挖孔");

        // EvenOdd:内圈挖孔 → 中心透明,而环带仍被填
        let mut pm = Pixmap::new(100, 100).unwrap();
        TinySkiaPainter::new(&mut pm).fill_path(&ring(30.0), PathFill::EvenOdd, red);
        assert_eq!(px(&pm, 50, 50).3, 0, "EvenOdd 下内圈应挖空");
        assert_eq!(px(&pm, 20, 50).3, 255, "环带本身仍要填上");
    }

    /// 曲线命令真的进了路径:三次贝塞尔鼓出去的部分要被填到
    #[test]
    fn cubic_curve_is_rasterized() {
        let mut pm = Pixmap::new(100, 100).unwrap();
        // 从左下到右下,控制点把曲线顶到上方 —— 形成一个鼓包
        let path = [
            PathCmd::MoveTo(10.0, 90.0),
            PathCmd::CubicTo(10.0, 0.0, 90.0, 0.0, 90.0, 90.0),
            PathCmd::Close,
        ];
        TinySkiaPainter::new(&mut pm).fill_path(&path, PathFill::NonZero, Color::rgb(0, 0, 255));
        assert_eq!(px(&pm, 50, 60).3, 255, "鼓包内部应被填充");
        assert_eq!(px(&pm, 50, 5).3, 0, "鼓包上方之外不该染色");
    }

    /// 退化路径静默跳过而不是崩(SVG 里空 `<path d="">` 合法)
    #[test]
    fn degenerate_paths_do_not_panic() {
        let mut pm = Pixmap::new(10, 10).unwrap();
        let mut p = TinySkiaPainter::new(&mut pm);
        p.fill_path(&[], PathFill::NonZero, Color::rgb(1, 2, 3));
        p.fill_path(
            &[PathCmd::MoveTo(1.0, 1.0)],
            PathFill::NonZero,
            Color::rgb(1, 2, 3),
        );
        p.fill_path(&[PathCmd::Close], PathFill::EvenOdd, Color::rgb(1, 2, 3));
        assert_eq!(px(&pm, 5, 5).3, 0, "退化路径不该画出任何东西");
    }

    /// 描边:线宽真的生效,且**只染到线上**——一条水平线,线上有色、
    /// 上下远处无色。描边最容易出的错是"当成填充画了",那样整个闭合区域会被涂满
    #[test]
    fn stroke_paints_the_line_not_the_area() {
        let mut pm = Pixmap::new(100, 100).unwrap();
        // 一个方框:填充会涂满内部,描边只画边
        let square = [
            PathCmd::MoveTo(20.0, 20.0),
            PathCmd::LineTo(80.0, 20.0),
            PathCmd::LineTo(80.0, 80.0),
            PathCmd::LineTo(20.0, 80.0),
            PathCmd::Close,
        ];
        let style = StrokeStyle {
            width: 6.0,
            ..StrokeStyle::default()
        };
        TinySkiaPainter::new(&mut pm).stroke_path(&square, &style, Color::rgb(0, 128, 0));
        assert_eq!(px(&pm, 50, 20).3, 255, "上边线上应有色");
        assert_eq!(px(&pm, 20, 50).3, 255, "左边线上应有色");
        assert_eq!(
            px(&pm, 50, 50).3,
            0,
            "方框内部**不该**被涂满(那是填充的行为)"
        );
        assert_eq!(px(&pm, 50, 5).3, 0, "线外不该染色");
    }

    /// 线宽真的传下去了:粗线覆盖的像素严格多于细线。
    /// 这条防的是"StrokeStyle 被构造了但没接到后端"这类静默失效
    #[test]
    fn stroke_width_reaches_the_backend() {
        let line = [PathCmd::MoveTo(10.0, 50.0), PathCmd::LineTo(90.0, 50.0)];
        let count = |w: f32| {
            let mut pm = Pixmap::new(100, 100).unwrap();
            TinySkiaPainter::new(&mut pm).stroke_path(
                &line,
                &StrokeStyle {
                    width: w,
                    ..StrokeStyle::default()
                },
                Color::rgb(0, 0, 0),
            );
            (0..100)
                .flat_map(|y| (0..100).map(move |x| (x, y)))
                .filter(|(x, y)| px(&pm, *x, *y).3 > 0)
                .count()
        };
        let (thin, thick) = (count(1.0), count(9.0));
        assert!(
            thick > thin * 4,
            "线宽 9 覆盖的像素应远多于线宽 1:{thin} vs {thick}"
        );
    }

    /// 线端形状生效:Round 端帽会在端点外多出一个半圆,Butt 不会
    #[test]
    fn line_cap_shape_reaches_the_backend() {
        let line = [PathCmd::MoveTo(30.0, 50.0), PathCmd::LineTo(70.0, 50.0)];
        let probe = |cap: LineCap| {
            let mut pm = Pixmap::new(100, 100).unwrap();
            TinySkiaPainter::new(&mut pm).stroke_path(
                &line,
                &StrokeStyle {
                    width: 10.0,
                    cap,
                    ..StrokeStyle::default()
                },
                Color::rgb(0, 0, 0),
            );
            // 端点外 3px 处
            px(&pm, 27, 50).3
        };
        assert_eq!(probe(LineCap::Butt), 0, "Butt 端帽不该越过端点");
        assert!(probe(LineCap::Round) > 0, "Round 端帽应在端点外多出半圆");
    }

    // -----------------------------------------------------------------
    // draw_image
    // -----------------------------------------------------------------

    /// 一个 RGBA8 像素(**预乘**口径,调用方自己保证 r/g/b ≤ a)
    const fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
        [r, g, b, a]
    }

    /// 2×2 已知像素:左上红、右上绿、左下蓝、右下白(全不透明)
    fn img_2x2() -> PixelImage {
        let mut v = Vec::new();
        v.extend_from_slice(&rgba(255, 0, 0, 255));
        v.extend_from_slice(&rgba(0, 255, 0, 255));
        v.extend_from_slice(&rgba(0, 0, 255, 255));
        v.extend_from_slice(&rgba(255, 255, 255, 255));
        PixelImage::new(2, 2, v).expect("2×2×4 = 16 字节")
    }

    /// 1:1 落点与颜色**逐像素**对拍。
    /// 弱断言("有非背景像素")抓不住的东西全在这:上下颠倒、左右镜像、
    /// 行主序写成列主序、RGBA 认成 BGRA —— 每一条都能画出"有内容"的图
    #[test]
    fn draw_image_1to1_lands_pixel_exact() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        TinySkiaPainter::new(&mut pm).draw_image(1.0, 1.0, 2.0, 2.0, &img_2x2());

        assert_eq!(px(&pm, 1, 1), (255, 0, 0, 255), "左上应是红");
        assert_eq!(px(&pm, 2, 1), (0, 255, 0, 255), "右上应是绿");
        assert_eq!(px(&pm, 1, 2), (0, 0, 255, 255), "左下应是蓝");
        assert_eq!(px(&pm, 2, 2), (255, 255, 255, 255), "右下应是白");
        // 目标矩形之外一个像素都不许染
        for (x, y) in [(0, 0), (3, 3), (0, 1), (1, 0), (3, 1), (1, 3)] {
            assert_eq!(px(&pm, x, y).3, 0, "({x},{y}) 在 dst 之外,不该被画到");
        }
    }

    /// dst 与源尺寸不等:**每轴独立拉伸铺满**(不保持比例、不裁剪)。
    /// 2×2 的正方形图画进 6×2 → x 轴 3 倍、y 轴 1 倍,被压扁是**定义**。
    /// 整数倍走最近邻(见 image_filter_nearest),所以这里可以逐像素断言
    #[test]
    fn draw_image_stretches_to_fill_dst() {
        let mut pm = Pixmap::new(6, 2).unwrap();
        TinySkiaPainter::new(&mut pm).draw_image(0.0, 0.0, 6.0, 2.0, &img_2x2());

        for x in 0..3 {
            assert_eq!(px(&pm, x, 0), (255, 0, 0, 255), "x={x} 应来自源左上(红)");
            assert_eq!(px(&pm, x, 1), (0, 0, 255, 255), "x={x} 应来自源左下(蓝)");
        }
        for x in 3..6 {
            assert_eq!(px(&pm, x, 0), (0, 255, 0, 255), "x={x} 应来自源右上(绿)");
            assert_eq!(
                px(&pm, x, 1),
                (255, 255, 255, 255),
                "x={x} 应来自源右下(白)"
            );
        }
    }

    /// 非整数缩放走双线性:不逐像素对拍(插值系数不该写死进测试),
    /// 但要守住两条**语义**不变量:铺满整个 dst、且不凭空冒出透明缝
    #[test]
    fn draw_image_non_integer_scale_covers_dst() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        TinySkiaPainter::new(&mut pm).draw_image(0.0, 0.0, 3.0, 3.0, &img_2x2());
        for y in 0..3 {
            for x in 0..3 {
                assert_eq!(px(&pm, x, y).3, 255, "({x},{y}) 在 dst 内应完全不透明");
            }
        }
        assert_eq!(px(&pm, 3, 3).3, 0, "dst 之外仍不该染色");
    }

    /// **预乘口径**。这条是整个动词最容易错的一处:输入按直通 alpha 写、
    /// 或者在后端里又乘了一次 alpha,都能编译、都能画出"看着有东西"的图,
    /// 只有把数值算出来对拍才抓得住。
    ///
    /// 源用预乘 (64,128,32,128) 画在**不透明白底**上,SourceOver 的结果是
    /// `src + dst*(1-a)`,a = 128/255 → dst 项系数 = (255-128)/255,
    /// 白底贡献 127:r=64+127=191,g=128+127=255,b=32+127=159,a=128+127=255。
    /// 若误把直通值 (128,255,64,128) 当预乘喂进去,r 会变成 255 —— 差 64。
    /// 若在后端里重复预乘一次,r 会变成 32+127=159 —— 差 32。两边都红。
    #[test]
    fn draw_image_respects_premultiplied_alpha() {
        let mut v = Vec::new();
        v.extend_from_slice(&rgba(64, 128, 32, 128)); // 半透明彩色(预乘)
        v.extend_from_slice(&rgba(0, 0, 0, 0)); // 全透明
        v.extend_from_slice(&rgba(255, 255, 255, 255)); // 不透明白
        v.extend_from_slice(&rgba(0, 0, 0, 128)); // 半透明黑(预乘)
        let img = PixelImage::new(2, 2, v).unwrap();

        let mut pm = Pixmap::new(2, 2).unwrap();
        pm.fill(tiny_skia::Color::WHITE);
        TinySkiaPainter::new(&mut pm).draw_image(0.0, 0.0, 2.0, 2.0, &img);

        // ±1 容差:tiny-skia 有 f32 / u16 两条光栅管线,按混合模式自动选,
        // u16 那条在最后一位上会差 1(它的 `force_hq_pipeline` 开关就是
        // 为这件事存在的)。语义错会差几十,不会差 1
        let near = |got: (u8, u8, u8, u8), want: (u8, u8, u8, u8), what: &str| {
            let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
            assert!(
                d(got.0, want.0) <= 1
                    && d(got.1, want.1) <= 1
                    && d(got.2, want.2) <= 1
                    && d(got.3, want.3) <= 1,
                "{what}: 期望 ~{want:?},实得 {got:?}"
            );
        };
        // 实测:这四个点全部**精确**命中,一位不差(容差留给将来管线变动)
        near(px(&pm, 0, 0), (191, 255, 159, 255), "半透明彩色叠白底");
        near(px(&pm, 1, 0), (255, 255, 255, 255), "全透明源不该改动背景");
        near(px(&pm, 0, 1), (255, 255, 255, 255), "不透明白覆盖白仍是白");
        near(
            px(&pm, 1, 1),
            (127, 127, 127, 255),
            "半透明黑叠白底应是中灰",
        );
    }

    /// 直通 alpha 的便捷入口真的做了预乘(PNG 解码出来就是直通)。
    /// 顺带守住 `a=255` 时必须是**恒等**变换 —— 用 `c*a/256` 这种"快速"
    /// 写法会把纯白 255 变成 254,整张不透明图整体暗一档
    #[test]
    fn from_straight_alpha_premultiplies() {
        let src = [255u8, 128, 0, 128, 255, 255, 255, 255];
        let img = PixelImage::from_straight_alpha(2, 1, &src).unwrap();
        assert_eq!(
            &img.pixels()[0..4],
            // round(255*128/255)=128, round(128*128/255)=64, 0, alpha 原样
            &[128, 64, 0, 128],
            "直通 → 预乘算错"
        );
        assert_eq!(
            &img.pixels()[4..8],
            &[255, 255, 255, 255],
            "alpha=255 时预乘必须是恒等"
        );
    }

    /// push_clip 裁掉的部分不出现(裁右半边)。
    ///
    /// **这一条单独存在时抓不住"图案锚点被写成裁剪后的原点"这个 bug**:
    /// 裁剪矩形是 (0,0,2,4)、dst 是 (0,0,4,4),求交后的原点 (0,0) 恰好**等于**
    /// dst 原点,两种实现逐位相同。真正的守卫是下面的
    /// `draw_image_clip_does_not_move_the_pattern_anchor`
    #[test]
    fn draw_image_is_clipped() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        let mut p = TinySkiaPainter::new(&mut pm);
        p.push_clip(0.0, 0.0, 2.0, 4.0, 0.0);
        p.draw_image(0.0, 0.0, 4.0, 4.0, &img_2x2());
        p.pop_clip();

        assert_eq!(px(&pm, 0, 0), (255, 0, 0, 255), "裁剪内仍是源左上(红)");
        assert_eq!(px(&pm, 1, 3), (0, 0, 255, 255), "裁剪内仍是源左下(蓝)");
        for y in 0..4 {
            for x in 2..4 {
                assert_eq!(px(&pm, x, y).3, 0, "({x},{y}) 被裁掉了,不该有像素");
            }
        }
    }

    /// "裁剪只缩**几何**,不动**图案锚点**" —— 裁掉**左**边缘与**上**边缘。
    ///
    /// 为什么必须裁左/上而不是裁右/下:`draw_image` 里 pattern 的 transform
    /// 用的是原始 dst 原点 `(x, y)`,而填充矩形用的是求交后的 `(cx, cy)`。
    /// 只裁右/下时 `(cx, cy) == (x, y)`,把锚点误写成 `(cx, cy)` 的实现**逐位
    /// 相同**,测试全绿(上面那条就是这个情形)。一裁左/上,两者立刻分叉:
    /// 误实现会把整张图向右下平移 2px,这里 (2,0) 会读回红而不是绿。
    ///
    /// 这不是纸面差异 —— 任何"裁掉左边缘/上边缘"的裁剪(滚动容器横向滚动、
    /// 弹层被视口左边卡住)都会画出整体偏移的图
    #[test]
    fn draw_image_clip_does_not_move_the_pattern_anchor() {
        // 裁左半:可见的是 dst 右半 → 源图右半列(绿/白)
        let mut pm = Pixmap::new(4, 4).unwrap();
        {
            let mut p = TinySkiaPainter::new(&mut pm);
            p.push_clip(2.0, 0.0, 2.0, 4.0, 0.0);
            p.draw_image(0.0, 0.0, 4.0, 4.0, &img_2x2());
            p.pop_clip();
        }
        assert_eq!(px(&pm, 2, 0), (0, 255, 0, 255), "右上应是源右上(绿)");
        assert_eq!(px(&pm, 3, 3), (255, 255, 255, 255), "右下应是源右下(白)");
        for y in 0..4 {
            for x in 0..2 {
                assert_eq!(px(&pm, x, y).3, 0, "({x},{y}) 被裁掉了,不该有像素");
            }
        }

        // 裁上半:可见的是 dst 下半 → 源图下半行(蓝/白)
        let mut pm = Pixmap::new(4, 4).unwrap();
        {
            let mut p = TinySkiaPainter::new(&mut pm);
            p.push_clip(0.0, 2.0, 4.0, 2.0, 0.0);
            p.draw_image(0.0, 0.0, 4.0, 4.0, &img_2x2());
            p.pop_clip();
        }
        assert_eq!(px(&pm, 0, 2), (0, 0, 255, 255), "左下应是源左下(蓝)");
        assert_eq!(px(&pm, 3, 3), (255, 255, 255, 255), "右下应是源右下(白)");
        for y in 0..2 {
            for x in 0..4 {
                assert_eq!(px(&pm, x, y).3, 0, "({x},{y}) 被裁掉了,不该有像素");
            }
        }
    }

    /// 零尺寸 / 尺寸与字节数不符 —— 第一道闸:压根构造不出来
    #[test]
    fn degenerate_images_are_rejected_at_construction() {
        assert!(PixelImage::new(0, 0, Vec::new()).is_none(), "零尺寸");
        assert!(PixelImage::new(0, 4, vec![0u8; 0]).is_none(), "零宽");
        assert!(PixelImage::new(4, 0, vec![0u8; 0]).is_none(), "零高");
        assert!(PixelImage::new(2, 2, vec![0u8; 4]).is_none(), "字节数不够");
        assert!(PixelImage::new(2, 2, vec![0u8; 17]).is_none(), "字节数多了");
        assert!(
            PixelImage::new(2, 2, vec![0u8; 16]).is_some(),
            "刚好 16 字节"
        );
        // 尺寸相乘溢出:`new` 这一层**只能看出"拒了"**,分不清是溢出闸拦的
        // 还是紧跟着的长度比对拦的(wrapping 实现算出来的天文数字同样对不上
        // 任何真实 Vec 长度)。真正的溢出断言在 image_byte_len_refuses_to_overflow
        assert!(
            PixelImage::new(u32::MAX, u32::MAX, vec![0u8; 16]).is_none(),
            "w*h*4 溢出时必须拒绝"
        );
        assert!(
            PixelImage::from_straight_alpha(2, 2, &[0u8; 4]).is_none(),
            "直通入口同一口径"
        );
    }

    /// 尺寸 → 字节数的溢出闸,**直接测算式本身**。
    ///
    /// 上一条测试为什么不够:`new` 算完还要比一次 `pixels.len() != need`,
    /// 而 wrapping 实现对 `u32::MAX × u32::MAX × 4` 算出来的
    /// 18446744073675997188 同样不等于 16 —— 于是 checked / wrapping 两种实现
    /// 在 `new` 这一层给出**同一个结果**,把 `checked_mul` 换成 `*` 照样全绿。
    /// 把算式单独暴露出来,`None` vs `Some(天文数字)` 才分得开
    #[test]
    fn image_byte_len_refuses_to_overflow() {
        assert_eq!(image_byte_len(2, 2), Some(16));
        assert_eq!(
            image_byte_len(1920, 1080),
            Some(1920 * 1080 * 4),
            "1080p 不该被误伤"
        );
        assert_eq!(image_byte_len(0, 4), None, "零宽");
        assert_eq!(image_byte_len(4, 0), None, "零高");
        assert_eq!(image_byte_len(u32::MAX, u32::MAX), None, "w*h*4 溢出");
        // 恰好踩线:2^31 × 2^31 = 2^62,再 ×4 正好是 2^64 —— 差一位就溢出。
        // (`w*h` 那一步在 64 位上不溢出,溢出的是随后的 ×4;这也是为什么
        //  两个 checked_mul 都不能省)
        assert_eq!(image_byte_len(1 << 31, 1 << 31), None, "×4 恰好溢出到 2^64");
    }

    /// 第二道闸:绕过构造器直接写出的畸形结构体,**CPU 后端**不许 panic /
    /// 越界,也不许画出任何像素。
    ///
    /// GPU 后端的同一道闸在 `vello_backend.rs` 的
    /// `vello_refuses_a_malformed_pixel_image`(那边只能靠
    /// [`PixelImage::bogus_for_test`] 造值 —— 字段私有于本模块)。
    /// 记录型后端刻意**不**参与:它的职责就是原样记下调用方要求画了什么
    #[test]
    fn malformed_image_is_refused_not_panicking() {
        // 声称 64×64(需 16384 字节),实际 4 字节
        let bogus = PixelImage::bogus_for_test(64, 64, &[1, 2, 3, 4]);
        let mut pm = Pixmap::new(8, 8).unwrap();
        {
            let mut p = TinySkiaPainter::new(&mut pm);
            p.draw_image(0.0, 0.0, 8.0, 8.0, &bogus);
            // dst 退化:零宽、负高、非有限
            p.draw_image(0.0, 0.0, 0.0, 4.0, &img_2x2());
            p.draw_image(0.0, 0.0, 4.0, -4.0, &img_2x2());
            p.draw_image(f32::NAN, 0.0, 4.0, 4.0, &img_2x2());
            p.draw_image(0.0, 0.0, f32::INFINITY, 4.0, &img_2x2());
        }
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(px(&pm, x, y).3, 0, "({x},{y}) 退化输入不该画出任何东西");
            }
        }
    }

    /// 金样后端:尺寸 + 内容 hash + 取整目标矩形,**像素一个字节都不进命令流**
    #[test]
    fn recording_painter_keeps_pixels_out_of_the_command_stream() {
        // 64×64 = 16 KiB 像素。整份进命令流的话,金样立刻变成二进制垃圾
        let big = PixelImage::new(64, 64, vec![0x5au8; 64 * 64 * 4]).unwrap();
        let mut rec = RecordingPainter::default();
        rec.draw_image(10.0, 20.0, 40.5, 40.5, &big);

        assert_eq!(
            rec.cmds,
            vec![PaintCmd::Image {
                src: (64, 64),
                hash: big.content_hash(),
                // 取整口径同 Path::bbox:(floor, floor, ceil, ceil)
                bbox: (10, 20, 51, 61),
            }]
        );
        // 一条命令的 Debug 输出必须短到人能读 —— 这就是"像素没进命令流"
        // 最直接的可执行证据
        let dump = format!("{:?}", rec.cmds);
        assert!(
            dump.len() < 200,
            "命令流快照长度 {},像素疑似漏进了命令流:{dump}",
            dump.len()
        );
        assert!(
            std::mem::size_of::<PaintCmd>() < 128,
            "PaintCmd 变大 = 有人往命令里塞了不该塞的东西:{} 字节",
            std::mem::size_of::<PaintCmd>()
        );
    }

    /// 金样记**内容**而不是 id:内容相同的两张图(id 必然不同)必须产出
    /// 完全一样的命令流,否则快照会随构造顺序无缘无故漂移;
    /// 内容差一个字节则必须不同,否则金样根本抓不住换图
    #[test]
    fn recording_painter_is_content_addressed_not_id_addressed() {
        let a = PixelImage::new(2, 2, vec![7u8; 16]).unwrap();
        let b = PixelImage::new(2, 2, vec![7u8; 16]).unwrap();
        let mut c_bytes = vec![7u8; 16];
        c_bytes[15] = 8;
        let c = PixelImage::new(2, 2, c_bytes).unwrap();

        assert_ne!(a.id(), b.id(), "两次构造必须是两个身份");
        assert_eq!(a.content_hash(), b.content_hash(), "同内容 hash 必须相同");
        assert_ne!(a.content_hash(), c.content_hash(), "差一个字节 hash 必须变");

        let shot = |img: &PixelImage| {
            let mut r = RecordingPainter::default();
            r.draw_image(0.0, 0.0, 2.0, 2.0, img);
            r.cmds
        };
        assert_eq!(shot(&a), shot(&b), "同内容的两张图金样必须一致");
        assert_ne!(shot(&a), shot(&c), "内容变了金样必须变");
    }

    /// 共享而非拷贝:clone 一张图不复制像素,且身份/内容都跟着走
    #[test]
    fn pixel_image_clone_shares_the_buffer() {
        let a = img_2x2();
        let b = a.clone();
        assert_eq!(a.id(), b.id(), "clone 保留身份(后端缓存才不会失效)");
        assert!(
            std::ptr::eq(a.pixels().as_ptr(), b.pixels().as_ptr()),
            "clone 必须共享同一份像素缓冲"
        );
    }

    /// dst 可画性判定。单独测是因为它**曾经漏过 `+∞`** —— `w > 0.0` 对
    /// 无穷大成立,CPU 端侥幸被 tiny-skia 的有限性检查兜住,vello 端直接
    /// 把无穷大编进了场景。现在两个后端共用这一个函数,这里是它的守卫
    #[test]
    fn dst_rect_drawable_rejects_every_degenerate_form() {
        assert!(dst_rect_drawable(0.0, 0.0, 1.0, 1.0));
        assert!(dst_rect_drawable(-5.0, -5.0, 1.0, 1.0), "负坐标是合法的");
        assert!(!dst_rect_drawable(0.0, 0.0, 0.0, 1.0), "零宽");
        assert!(!dst_rect_drawable(0.0, 0.0, 1.0, 0.0), "零高");
        assert!(!dst_rect_drawable(0.0, 0.0, -1.0, 1.0), "负宽");
        assert!(!dst_rect_drawable(f32::NAN, 0.0, 1.0, 1.0), "NaN 坐标");
        assert!(!dst_rect_drawable(0.0, f32::NAN, 1.0, 1.0), "NaN 坐标");
        assert!(!dst_rect_drawable(0.0, 0.0, f32::NAN, 1.0), "NaN 宽");
        assert!(
            !dst_rect_drawable(0.0, 0.0, f32::INFINITY, 1.0),
            "+∞ 宽(第一版就漏在这)"
        );
        assert!(!dst_rect_drawable(0.0, 0.0, 1.0, f32::INFINITY), "+∞ 高");
        assert!(!dst_rect_drawable(f32::INFINITY, 0.0, 1.0, 1.0), "+∞ 坐标");
    }

    /// 滤镜裁决本身:整数倍(含 1:1)最近邻,其余双线性
    #[test]
    fn filter_choice_matches_the_documented_rule() {
        assert!(image_filter_nearest(1.0, 1.0), "1:1");
        assert!(image_filter_nearest(3.0, 1.0), "非等比但两轴都是整数倍");
        assert!(!image_filter_nearest(1.5, 1.0), "1.5 倍不是整数倍");
        assert!(!image_filter_nearest(0.5, 0.5), "缩小");
        assert!(!image_filter_nearest(2.0, 0.25), "一轴缩小就不算");
    }

    /// 金样后端记录路径:条数/规则/颜色/包围盒(逐点记录会让金样没法看)
    #[test]
    fn recording_painter_records_path_shape() {
        let mut rec = RecordingPainter::default();
        rec.fill_path(
            &[
                PathCmd::MoveTo(10.0, 20.0),
                PathCmd::CubicTo(10.0, 0.0, 90.0, 0.0, 90.5, 20.0),
                PathCmd::Close,
            ],
            PathFill::EvenOdd,
            Color::rgb(7, 8, 9),
        );
        assert_eq!(
            rec.cmds,
            vec![PaintCmd::Path {
                cmds: 3,
                fill: PathFill::EvenOdd,
                color: Color::rgb(7, 8, 9),
                // 控制点也进包围盒(保守):y 到 0,x 到 91(ceil)
                bbox: (10, 0, 91, 20),
            }]
        );
    }
}
