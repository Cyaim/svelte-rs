//! # sv-lottie
//!
//! Lottie 矢量动画:**velato 的 `RenderSink` → sv-shell 的路径动词**。
//!
//! ## 为什么它不需要 GPU(整个 crate 的立论)
//!
//! "lottie = GPU 特性"是个直觉陷阱。velato 0.11 把渲染出口抽成了后端无关的
//! `RenderSink` trait,`vello::Scene` 只是它自带的一个实现(`runtime/vello.rs`
//! 全文 56 行)。开 `default-features = false` 之后,依赖树里
//! **没有 vello、没有 wgpu、没有 naga**,只剩 kurbo + peniko + serde ——
//! 也就是说 lottie 的**求值**(解析 JSON、按帧号插值出一堆带画刷的贝塞尔路径)
//! 是纯 CPU 的算术,与用什么光栅化后端无关。
//!
//! 主线的 `Painter::fill_path` / `stroke_path`(tiny-skia / vello / Recording
//! 三个后端都已实现)正好就是这些路径的出口。于是 **CPU 默认后端可以原生吃下
//! lottie**,不必等 vello 成为默认后端。
//!
//! ## 数据流
//!
//! ```text
//! lottie JSON ──Composition::from_slice──▶ Lottie
//!                                            │  render(frame, placement, alpha, sink)
//!                                            ▼
//!                        velato::Renderer::append ──▶ PainterSink (impl RenderSink)
//!                                                          │ fill_path / stroke_path
//!                                                          ▼
//!                                                   impl PathSink
//!                                          (RecordingSink / 你的 Painter 桥)
//! ```
//!
//! ## 时间轴与帧循环
//!
//! [`Timeline`] / [`Playback`] 只做算术:给一个 wall-clock 毫秒,得到帧号。
//! **本 crate 不接帧循环** —— 接进 sv-shell 的 `anim::pump` 要新增一条媒体通道、
//! 要处理"每帧写但不 bump 版本",那是 sv-shell 的改动。
//!
//! ## 已知缺口
//!
//! 见 README 与 [`RenderStats`] 上的逐条计数器,一次问完用
//! [`RenderStats::degraded`]。一句话版本:**渐变退化为纯色、非矩形裁剪
//! (图层遮罩)失效、矩形遮罩一律按 intersect 处理(subtract 会画反)、
//! 混合模式丢弃、各向异性描边宽度只是近似**。
//!
//! **计数器不是全集。** velato 的导入器会在 `RenderSink` 之前就丢掉一些东西
//! (虚线描边、遮罩的 mode 与 opacity),那些丢失在这一层不可观测,
//! `degraded()` 返回 `false` 也可能已经画得不对了。逐条清单在 `sink.rs`
//! 模块头的「本适配器观测不到的降级」。

mod path;
mod sink;
mod timeline;

/// README 里的用法示例当 doctest 跑(`cargo test -p sv-lottie` 会带上它)。
///
/// 挂在一个 `#[cfg(doctest)]` 的空 struct 上而不是 `#![doc = include_str!]`:
/// 后者会把整份 README 塞进 crate 文档首页,与上面这段 `//!` 打架
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub use path::{
    LineCap, LineJoin, PathCmd, PathFill, PathSink, RecordingSink, SinkCmd, StrokeStyle,
};
pub use sink::{DEFAULT_TOLERANCE, PainterSink, RenderStats};
/// 转发 sv-ui 的颜色类型:实现 [`PathSink`] 必须能命名它,
/// 不转发的话每个下游都要自己再依赖一次 sv-ui
pub use sv_ui::Color;
pub use timeline::{Playback, Timeline};

use peniko::kurbo::Affine;

/// 加载失败的原因
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// JSON 不合 Lottie schema(velato 的 serde 层报错)
    Parse(String),
    /// velato 在**合法** Lottie 上 `todo!()` 了,被我们接住转成错误。
    /// 消息是 panic 的 payload 原文
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(m) => write!(f, "lottie 解析失败:{m}"),
            Self::Unsupported(m) => write!(f, "lottie 用到了 velato 尚未实现的特性:{m}"),
        }
    }
}

impl std::error::Error for Error {}

/// 把动画摆到目标矩形里的方式(等比 + 平移)。
///
/// v1 只有等比缩放:各向异性缩放会让描边宽度补偿从"精确"退化成"近似"
/// (见 `sink.rs` 的 `stroke_style`),而 `object-fit` 那一族语义属于
/// CSS 属性面,不由 lottie 定义。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub tx: f32,
    pub ty: f32,
    pub scale: f32,
}

impl Placement {
    /// 原尺寸摆在原点
    pub const IDENTITY: Placement = Placement {
        tx: 0.0,
        ty: 0.0,
        scale: 1.0,
    };

    fn to_affine(self) -> Affine {
        Affine::translate((self.tx as f64, self.ty as f64)) * Affine::scale(self.scale as f64)
    }
}

impl Default for Placement {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// 一份已解析的 lottie 资产 + 一个可复用的求值器。
///
/// **持有 `velato::Renderer` 是有意的**:它内部有一个跨帧复用的几何批缓冲
/// (`append` 开头 `batch.clear()`),每帧新建一个等于把这份缓冲白扔。
/// 代价是 [`Self::render`] 要 `&mut self`。
///
/// 一份 `Composition` 常驻约 1 MB(lottie-3 spike §5.3 实测,与 JSON 大小
/// 相关性不强:8 KB 与 132 KB 的资产都落在 0.7–1.1 MB),多个节点共用同一份
/// 是常态(列表里 20 个 spinner)—— 所以它应当放在壳侧的资源注册表里按 id 共享,
/// 不要往场景树上塞。
pub struct Lottie {
    comp: velato::Composition,
    renderer: velato::Renderer,
}

/// 手写而不是 derive:`velato::Composition` 的 Debug 会把整棵图层树连同每条
/// 关键帧全打出来(一个 57KB 的资产能刷几万行),在断言失败信息里毫无用处
impl std::fmt::Debug for Lottie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lottie")
            .field("size", &self.size())
            .field("timeline", &self.timeline())
            .field("layers", &self.comp.layers.len())
            .finish()
    }
}

impl Lottie {
    /// 从 JSON 字节加载。
    ///
    /// **为什么包 `catch_unwind`**:velato 0.11 在**合法** Lottie 上会 panic
    /// 而不是返回 `Err`。逐个数过(velato 0.11.0 源码,`#[cfg(test)]` 里的
    /// 不算):`import/converters.rs` 有 **4 个 `todo!()`**(`:211` `:213`
    /// `:243` `:252`)+ **3 个 `unimplemented!()`**(`:103` 未支持的资产类型、
    /// `:805` `:806` Add / HardMix 混合模式),**共 7 处,全在导入器**
    /// (即解析期,正好是本函数覆盖的窗口)。
    ///
    /// 其中 `:213` 是纯 bug:schema 把图层 transform 的旋转 `r` 声明成
    /// `Option`(Lottie 规范里它本来就可省),转换器却在 `None` 分支写
    /// `todo!("split rotation")`。也就是说**一个没有 `r` 键的合法图层会崩进程**。
    /// 崩进程是最坏处理(R4 红线),所以这里把 unwind 接住转成
    /// [`Error::Unsupported`]。仓库没有开 `panic = "abort"`,这条路可用
    /// (有回归测试 `unsupported_lottie_reports_error_instead_of_panicking` 守着)。
    ///
    /// 这是创可贴不是治疗:根治是给 velato 提 PR 把那 7 处换成 `Error` 变体。
    ///
    /// **渲染期(`Renderer::append`)没有 `catch_unwind`** —— 那是每帧都要跑的
    /// 热路径,不该为它付 unwind 屏障的代价。读过 velato 0.11 的 `runtime/`
    /// 全部两处 `unwrap`(`model/animated.rs:257`、`render.rs:306`)之后:
    /// **两处都被紧邻的上一行守住**(前者在 `:236` 的
    /// `if vertices.is_empty() { return; }` 之后 22 行,后者由同一个 `if` 的
    /// `self.drawn_geometry < self.geometries.len()` 条件保证),读下来不可达。
    /// 也就是说 0.11 上没找到可达的渲染期 panic —— 但这不是上游的 API 承诺,
    /// 真炸了只能靠调用方兜。渲染期真正被覆盖的是**裁剪栈污染**:
    /// [`PainterSink`] 的 `Drop` 会在展开时补发欠下的 `pop_clip`。
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let parsed = std::panic::catch_unwind(|| velato::Composition::from_slice(bytes));
        match parsed {
            Ok(Ok(comp)) => Ok(Self {
                comp,
                renderer: velato::Renderer::new(),
            }),
            Ok(Err(e)) => Err(Error::Parse(e.to_string())),
            Err(payload) => Err(Error::Unsupported(panic_message(&*payload))),
        }
    }

    /// 同 [`Self::from_slice`],入参是字符串
    pub fn from_json_str(s: &str) -> Result<Self, Error> {
        Self::from_slice(s.as_bytes())
    }

    /// 合成画布的固有尺寸(lottie 的 `w` / `h`)
    pub fn size(&self) -> (f32, f32) {
        (self.comp.width as f32, self.comp.height as f32)
    }

    /// 时间轴事实
    pub fn timeline(&self) -> Timeline {
        Timeline {
            start_frame: self.comp.frames.start,
            end_frame: self.comp.frames.end,
            frame_rate: self.comp.frame_rate,
        }
    }

    /// 新建一个从头开始、循环播放的播放态
    pub fn playback(&self) -> Playback {
        Playback::new(self.timeline())
    }

    /// 等比缩放居中填进内容盒(CSS `object-fit: contain` 语义)。
    /// 传入的是**物理像素**矩形(调用方已乘 scale)
    pub fn fit_contain(&self, x: f32, y: f32, w: f32, h: f32) -> Placement {
        let (cw, ch) = self.size();
        if cw <= 0.0 || ch <= 0.0 || w <= 0.0 || h <= 0.0 {
            return Placement {
                tx: x,
                ty: y,
                scale: 1.0,
            };
        }
        let scale = (w / cw).min(h / ch);
        Placement {
            tx: x + (w - cw * scale) * 0.5,
            ty: y + (h - ch * scale) * 0.5,
            scale,
        }
    }

    /// 求值第 `frame` 帧并发到 `sink`。
    ///
    /// `frame` 用 [`Timeline::frame_at_ms`] 或 [`Playback::frame`] 算;
    /// 直接传一个越界的帧号不会崩,只会画出空帧(velato 按
    /// `layer.frames.contains(&frame)` 逐图层判活跃)。
    ///
    /// `alpha` 是整体不透明度(0..=1),velato 会把它乘进每个画刷。
    pub fn render<S: PathSink + ?Sized>(
        &mut self,
        frame: f64,
        place: Placement,
        alpha: f32,
        sink: &mut S,
    ) -> RenderStats {
        self.render_with_tolerance(frame, place, alpha, DEFAULT_TOLERANCE, sink)
    }

    /// 同 [`Self::render`],但指定曲线展平容差(设备像素)
    pub fn render_with_tolerance<S: PathSink + ?Sized>(
        &mut self,
        frame: f64,
        place: Placement,
        alpha: f32,
        tolerance: f64,
        sink: &mut S,
    ) -> RenderStats {
        let mut adapter = PainterSink::with_tolerance(sink, tolerance);
        self.renderer.append(
            &self.comp,
            frame,
            place.to_affine(),
            alpha.clamp(0.0, 1.0) as f64,
            &mut adapter,
        );
        // finish() 而不是 stats():排空图层栈,把欠下的 pop_clip 补发出去。
        // 少一次 pop_clip = 宿主 Painter 的裁剪栈被永久污染
        adapter.finish()
    }
}

/// 从 panic payload 里挖出人能看懂的消息
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "velato panicked(payload 不是字符串)".to_string()
    }
}
