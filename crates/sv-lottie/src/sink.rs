//! velato `RenderSink` → [`PathSink`] 适配器。
//!
//! velato 0.11 把渲染出口抽成了后端无关的 trait(`runtime/render.rs:14`),
//! `vello::Scene` 只是它自带的一个实现(整个 `runtime/vello.rs` 只有 56 行)。
//! 本文件是第二个实现,出口换成 sv-shell 的路径动词形状。
//!
//! ## 有损映射清单
//!
//! velato 给的动词比 `Painter` 富:它有画刷(纯色/渐变/图像)、任意形状裁剪、
//! 混合模式与图层 alpha;`Painter` 只有"纯色填充/纯色描边/矩形裁剪"。
//! 差额只能降级,**降级要可观测** —— 否则"画少了一层"会变成一个
//! 没人发现的静默 bug(自绘 UI 里这类 bug 最难定位)。
//!
//! | velato 给的 | 这里怎么落 | 计数器 |
//! |---|---|---|
//! | `Brush::Solid` | 直通 | —— |
//! | `Brush::Gradient` | **退化为色标沿 offset 的分段线性均值(一种纯色)** | `gradient_fallbacks` |
//! | `Brush::Image` | 整条 draw 丢弃(velato 本来也不支持图片内嵌) | `image_brushes_skipped` |
//! | `push_clip_layer(轴对齐矩形)`,根裁剪 | 转 `PathSink::push_clip_rect` | `clips_applied` |
//! | `push_clip_layer(轴对齐矩形)`,图层遮罩 | 同上,但**遮罩模式已被上游丢弃**,一律按 intersect 处理 | `mask_clips_intersected` |
//! | `push_clip_layer(任意形状)` | **忽略**(图层遮罩会整体失效) | `clips_ignored` |
//! | `push_layer(blend, alpha, ..)` | blend 丢弃;alpha 乘进后续颜色 | `layers_flattened` |
//! | 各向异性变换下的描边宽度 | 取 `√\|det A\|`(几何均值),**不是正解** | `stroke_width_approximated` |
//! | 展平后为空的路径 | 整条 draw 丢弃 | `empty_paths_skipped` |
//! | 全透明颜色 | 整条 draw 丢弃(省一次光栅) | `transparent_skipped` |
//!
//! ## 本适配器观测不到的降级(诚实清单)
//!
//! 有损不只发生在这一层。**velato 的导入器会在我们看到任何东西之前丢掉一些
//! 东西**,那些丢失在 `RenderSink` 这个口径上是不可见的,本文件的计数器
//! 一个都不会响:
//!
//! - **虚线描边**:`schema/shapes/base_stroke.rs:36` 解析了 `d`,但
//!   `runtime/model/animated.rs:414` 构造描边时只写
//!   `kurbo::Stroke::new(w).with_caps(cap).with_join(join)` ——
//!   `dash_pattern` 恒为空。虚线在 import 层就没了,这里只会看到实线。
//!   [`RenderStats::dashes_ignored`] 因此在 velato 0.11 上**恒为 0**
//!   (保留它是**前向卫兵**:上游哪天开始发 dash,我们不会静默画错。
//!   有单测 `dash_guard_would_fire_if_upstream_ever_emitted_one` 钉住这条)。
//! - **遮罩模式与遮罩不透明度**:`runtime::model::Mask` 带
//!   `mode: peniko::BlendMode` 与 `opacity`,但 `render.rs:129-133` 只发一个裸的
//!   `push_clip_layer`,两者都没传下来。见 [`RenderStats::mask_clips_intersected`]。
//! - **两端不同的端帽**:同上,`with_caps` 把 start/end 设成同一个值,
//!   [`RenderStats::stroke_cap_mismatch`] 同样是前向卫兵。
//!
//! velato 自带的 `impl RenderSink for vello::Scene`(`runtime/vello.rs:20`)
//! 在这几条上的行为与本文件**逐字相同**(它也是 `push_clip_layer` → 裁剪、
//! 也拿不到 mode),所以这不是"CPU 后端比 GPU 后端差",是上游的共同上界。

use peniko::Brush;
use peniko::color::Srgb;
use peniko::kurbo::{Affine, Cap, Join, PathEl, Point, Shape};
use sv_ui::Color;
use velato::model::fixed;

use crate::path::{LineCap, LineJoin, PathCmd, PathFill, PathSink, StrokeStyle};

/// 曲线展平容差,**设备像素**。
///
/// 这个数是我们自己选的自由参数,不是 velato 给的:`Shape::path_elements(tol)`
/// 要一个容差。选大了图标边缘出多边形,选小了每帧点数暴涨。0.1 物理像素是
/// lottie spike(`docs/plans/lottie-3-spike.md` §3.2)实跑用的值。
///
/// 实际影响比想象小:velato 交给 sink 的绝大多数形状已经是 `BezPath`
/// (三次贝塞尔),`path_elements` 原样吐出、根本不看容差;只有 `Rect`/`Circle`
/// 这类解析形状会用到它。
pub const DEFAULT_TOLERANCE: f64 = 0.1;

/// 各向异性判定阈值:两个奇异值之比超过它就认为"描边宽度只是近似"。
///
/// 1.05 = 5%,大约是 1px 描边上半个像素的误差,再小就会被浮点噪声刷屏。
const ANISOTROPY_TOLERANCE: f64 = 1.05;

/// 一帧里发生的所有降级与绘制计数。
///
/// **测试应当断言它**:例如"这个资产不该有渐变" → `gradient_fallbacks == 0`,
/// "图层栈必须平衡" → `unbalanced_pops == 0`。一次问完用 [`Self::degraded`]。
///
/// 注意:这个结构体只覆盖**本适配器**的有损映射。velato 导入器更早丢掉的
/// 东西(虚线、遮罩模式)在这里是不可见的,见模块头「本适配器观测不到的降级」。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub fills: usize,
    pub strokes: usize,
    /// 渐变被拍成纯色的次数
    pub gradient_fallbacks: usize,
    /// 图像画刷导致整条 draw 被丢弃的次数
    pub image_brushes_skipped: usize,
    /// 成功转成矩形裁剪的 `push_clip_layer` 次数(每帧至少 1:合成画布根裁剪)
    pub clips_applied: usize,
    /// 因为形状不是轴对齐矩形而被忽略的裁剪次数
    pub clips_ignored: usize,
    /// **被当成 intersect 处理的图层遮罩数**(`clips_applied` 的子集)。
    ///
    /// velato 的 `runtime::model::Mask` 是带 `mode`(Add/Subtract/Intersect/
    /// Lighten/…)和 `opacity` 的,但 `render.rs:129-133` 把两者**都丢了**,
    /// 只发一个裸的 `push_clip_layer`。到这一层已经无从分辨,于是一律按
    /// intersect 裁剪落地。
    ///
    /// 后果分两种:
    /// - `mode = Add`(lottie 缺省、绝大多数资产)→ intersect 就是**正解**;
    /// - `mode = Subtract` / `inv: true` → 画面**正好反了**(该藏的露出来、
    ///   该露的被裁掉)。这是 fail-closed 而且方向相反,比 `clips_ignored`
    ///   那种 fail-open 难查得多。
    ///
    /// 所以:**非零 = 这一帧有图层遮罩,画面正确性取决于资产用的是哪种 mode**。
    /// 消灭它需要上游把 `mode` 透到 `RenderSink`(velato 的 vello 后端有同样的洞)。
    pub mask_clips_intersected: usize,
    /// 被拍平的 `push_layer`(混合模式丢弃、alpha 乘进颜色)
    pub layers_flattened: usize,
    /// 被忽略的虚线描边。
    ///
    /// **前向卫兵:在 velato 0.11 上恒为 0** —— 虚线在 velato 的导入器里就被
    /// 丢了(`runtime/model/animated.rs:414` 构造 `kurbo::Stroke` 时不写
    /// `dash_pattern`),本适配器根本看不到。见模块头。
    pub dashes_ignored: usize,
    /// 两端端帽不同、只能取一个的次数。
    ///
    /// **前向卫兵:在 velato 0.11 上恒为 0** —— `with_caps` 把 start/end
    /// 设成同一个值。见模块头。
    pub stroke_cap_mismatch: usize,
    /// **描边宽度只是近似**的次数。
    ///
    /// 变换被烘焙进坐标之后描边宽度不再跟着缩放,这里手工补 `√|det A|`。
    /// 各向同性缩放下这个补偿是**精确**的;各向异性下(squash-and-stretch,
    /// lottie 最常见的手法之一)它只是几何均值 —— 最坏情况如
    /// `scale = [400%, 25%]`,`det = 1`,补偿因子恰好 1.0,等于**零修正**,
    /// 而正解是横向 4× / 纵向 0.25×。
    ///
    /// 判据是仿射两个奇异值之比 > 5%。根治要给路径动词加 `transform` 参数。
    pub stroke_width_approximated: usize,
    /// 多余的 `pop_layer`。**非零 = velato 的图层栈失衡**,此时这一帧的
    /// 裁剪/alpha 都不可信(velato_imaging 把同一情形收成
    /// `Error::UnbalancedLayerStack`,我们收成一个计数器 + 不 panic)
    pub unbalanced_pops: usize,
    /// **欠下的 `pop_layer`**:一帧结束时图层栈上还剩几格。
    ///
    /// 非零同样意味着这一帧不可信,但它比 `unbalanced_pops` **更危险** ——
    /// `push_clip_rect` 是往**宿主 `Painter` 的裁剪栈**上压的,漏 pop 会
    /// 污染这一帧之后画的**所有**控件(窗口生命周期内永久)。所以
    /// [`PainterSink::finish`] 与 `Drop` 都会把欠的 `pop_clip` 补发出去,
    /// 这个计数器只是留个案底
    pub unclosed_layers: usize,
    /// 因为颜色全透明而被跳过的 draw(省一次光栅)
    pub transparent_skipped: usize,
    /// 因为展平后路径为空而被跳过的 draw
    pub empty_paths_skipped: usize,
}

impl RenderStats {
    /// 这一帧有没有发生**任何**有损映射。
    ///
    /// `fills` / `strokes` / `clips_applied` 是正常绘制量,不算降级。
    /// 断言 `!stats.degraded()` 比逐条列计数器省事,也不会在新增计数器时漏掉。
    ///
    /// **它只覆盖本适配器能看见的降级** —— velato 导入器更早丢掉的东西
    /// (虚线、遮罩模式)返回 false 也可能已经画错了,见模块头。
    pub fn degraded(&self) -> bool {
        let Self {
            fills: _,
            strokes: _,
            clips_applied: _,
            gradient_fallbacks,
            image_brushes_skipped,
            clips_ignored,
            mask_clips_intersected,
            layers_flattened,
            dashes_ignored,
            stroke_cap_mismatch,
            stroke_width_approximated,
            unbalanced_pops,
            unclosed_layers,
            transparent_skipped,
            empty_paths_skipped,
        } = self;
        // 逐字段解构而不是 `self.a != 0 || self.b != 0`:新增一个计数器却忘了
        // 加进来,这里会编译不过
        *gradient_fallbacks != 0
            || *image_brushes_skipped != 0
            || *clips_ignored != 0
            || *mask_clips_intersected != 0
            || *layers_flattened != 0
            || *dashes_ignored != 0
            || *stroke_cap_mismatch != 0
            || *stroke_width_approximated != 0
            || *unbalanced_pops != 0
            || *unclosed_layers != 0
            || *transparent_skipped != 0
            || *empty_paths_skipped != 0
    }
}

/// 图层栈的一格
#[derive(Clone, Copy)]
struct LayerFrame {
    /// 从根累乘下来的 alpha
    alpha: f32,
    /// 这一格是否真的下发过 `push_clip_rect`(决定 pop 时要不要下发 `pop_clip`)
    clipped: bool,
}

/// velato 的 `RenderSink` 实现:把一帧翻译成 [`PathSink`] 动词。
///
/// `RenderSink` 的方法带 `impl Trait` 参数(非对象安全),所以这里必须是
/// **具体类型**、不能是 `dyn RenderSink` —— 这不构成障碍,`dyn` 依旧只出现在
/// `PathSink` 那一侧。
///
/// **用完请调 [`Self::finish`]**(或直接用 [`crate::Lottie::render`],它替你调了):
/// 它会把欠下的 `pop_clip` 补发出去。`Drop` 里也有同一份逻辑兜底,
/// 所以就算中途 panic 展开,宿主 `Painter` 的裁剪栈也不会被污染。
pub struct PainterSink<'a, S: PathSink + ?Sized> {
    out: &'a mut S,
    tolerance: f64,
    layers: Vec<LayerFrame>,
    /// 跨 draw 复用的路径缓冲。velato 每帧重算全部几何,这里要是每次 draw
    /// 都新建一个 Vec,一帧几十次分配就白扔了
    scratch: Vec<PathCmd>,
    stats: RenderStats,
}

impl<'a, S: PathSink + ?Sized> PainterSink<'a, S> {
    pub fn new(out: &'a mut S) -> Self {
        Self::with_tolerance(out, DEFAULT_TOLERANCE)
    }

    pub fn with_tolerance(out: &'a mut S, tolerance: f64) -> Self {
        Self {
            out,
            tolerance: if tolerance.is_finite() && tolerance > 0.0 {
                tolerance
            } else {
                DEFAULT_TOLERANCE
            },
            layers: Vec::new(),
            scratch: Vec::new(),
            stats: RenderStats::default(),
        }
    }

    /// 当前计数(**渲染中途**的快照)。收尾请用 [`Self::finish`]
    pub fn stats(&self) -> RenderStats {
        self.stats
    }

    /// 收尾:排空图层栈(补发欠下的 `pop_clip`)并返回最终计数。
    ///
    /// **不能省。** `push_clip_rect` 压的是**宿主 `Painter` 的**裁剪栈,
    /// 少一次 `pop_clip` 就等于这一帧之后画的所有控件都被裁在 lottie 那个
    /// 矩形里,而且是窗口生命周期内永久的 —— 只放掉 `self.layers` 修不好它。
    pub fn finish(mut self) -> RenderStats {
        self.drain();
        self.stats
    }

    /// 把图层栈上剩下的格子全部弹掉,每弹一格记一笔 `unclosed_layers`
    fn drain(&mut self) {
        while let Some(f) = self.layers.pop() {
            self.stats.unclosed_layers += 1;
            if f.clipped {
                self.out.pop_clip();
            }
        }
    }

    fn cur_alpha(&self) -> f32 {
        self.layers.last().map_or(1.0, |l| l.alpha)
    }

    /// 把 velato 的形状 + 仿射变换烘焙成设备像素路径。
    ///
    /// **变换必须烘焙进坐标**,不能透传:`Painter` 的全部动词都建立在
    /// "坐标 = 物理像素,调用方已乘 scale"这条不变量上(paint.rs 文件头),
    /// 裁剪栈、命中测试、金样命令流全部长在它上面。
    fn flatten(&mut self, shape: &impl Shape, transform: Affine) {
        self.scratch.clear();
        // 容差是**设备空间**的口径,而 path_elements 在形状本地空间展平,
        // 所以要先除掉变换的尺度,否则放大 10 倍的图标边缘会出多边形
        let tol = self.tolerance / affine_scale(transform).max(1e-6);
        let map = |p: Point| {
            let q = transform * p;
            (q.x as f32, q.y as f32)
        };
        for el in shape.path_elements(tol) {
            let cmd = match el {
                PathEl::MoveTo(p) => {
                    let (x, y) = map(p);
                    PathCmd::MoveTo(x, y)
                }
                PathEl::LineTo(p) => {
                    let (x, y) = map(p);
                    PathCmd::LineTo(x, y)
                }
                PathEl::QuadTo(c, p) => {
                    let (cx, cy) = map(c);
                    let (x, y) = map(p);
                    PathCmd::QuadTo(cx, cy, x, y)
                }
                PathEl::CurveTo(c1, c2, p) => {
                    let (c1x, c1y) = map(c1);
                    let (c2x, c2y) = map(c2);
                    let (x, y) = map(p);
                    PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y)
                }
                PathEl::ClosePath => PathCmd::Close,
            };
            self.scratch.push(cmd);
        }
    }

    fn stroke_style(&mut self, s: &fixed::Stroke, transform: Affine) -> StrokeStyle {
        // 下面两条在 velato 0.11 上恒不成立(见模块头),留着是前向卫兵:
        // 上游哪天开始发 dash / 不同端帽,降级会立刻变得可观测而不是静默画错
        if !s.dash_pattern.is_empty() {
            self.stats.dashes_ignored += 1;
        }
        if s.start_cap != s.end_cap {
            self.stats.stroke_cap_mismatch += 1;
        }
        // 各向异性:`√|det A|` 是两个奇异值的**几何均值**,横竖缩放不同时
        // 它谁都不对。最坏的 squash-and-stretch(det ≈ 1)修正量恰好为零
        let (sx, sy) = affine_singular_values(transform);
        if sy <= 0.0 || sx > sy * ANISOTROPY_TOLERANCE {
            self.stats.stroke_width_approximated += 1;
        }
        StrokeStyle {
            // 变换烘焙进坐标之后描边宽度不再跟着缩放,这里手工补偿
            width: (s.width * affine_scale(transform)) as f32,
            cap: match s.start_cap {
                Cap::Butt => LineCap::Butt,
                Cap::Square => LineCap::Square,
                Cap::Round => LineCap::Round,
            },
            join: match s.join {
                Join::Miter => LineJoin::Miter,
                Join::Round => LineJoin::Round,
                Join::Bevel => LineJoin::Bevel,
            },
            miter_limit: s.miter_limit as f32,
        }
    }

    /// 画刷 → 纯色。`None` = 这条 draw 整个丢弃
    fn brush_color(&mut self, brush: &fixed::Brush) -> Option<Color> {
        match brush {
            Brush::Solid(c) => Some(rgba8_to_color(c.to_rgba8())),
            Brush::Gradient(g) => {
                // 降级:渐变拍成一种纯色。lottie-2 §0 裁决 4 明确"v1 两个后端
                // 降级到完全一致:渐变取平均色",换来"换后端画面不变"这条硬保证。
                // 真要保真需要 Painter 长出渐变画刷 —— 那不由 lottie 顺手带进来
                self.stats.gradient_fallbacks += 1;
                Some(average_stop_color(&g.stops.0))
            }
            Brush::Image(_) => {
                // velato README 自己就把"图片内嵌"列在不支持清单里,这里走到
                // 基本意味着资产超纲。丢弃整条 draw 而不是画个黑块
                self.stats.image_brushes_skipped += 1;
                None
            }
        }
    }
}

/// 兜底排空:`Renderer::append` 中途 panic 展开时,已经发出去的
/// `push_clip_rect` 靠这条补上 `pop_clip`。不这么做的话宿主 `Painter` 的
/// 裁剪栈会被永久污染(见 [`RenderStats::unclosed_layers`])
impl<S: PathSink + ?Sized> Drop for PainterSink<'_, S> {
    fn drop(&mut self) {
        self.drain();
    }
}

impl<S: PathSink + ?Sized> velato::RenderSink for PainterSink<'_, S> {
    fn push_layer(
        &mut self,
        _blend: impl Into<peniko::BlendMode>,
        alpha: f32,
        _transform: Affine,
        _shape: &impl Shape,
    ) {
        // velato 0.11 只在**轨道遮罩**分支发 push_layer,且 alpha 恒为 1.0
        // (`render.rs:113/126`),所以这条 alpha 通道今天实际是恒等的。
        // 保留累乘是为了"上游哪天开始发非 1.0 时我们不会静默画错"。
        //
        // 代价照记:blend 丢了、隔离层没了 —— 带轨道遮罩的资产会把**遮罩图层
        // 本身当成可见内容画出来**。velato 那边也还挂着
        // `todo: re-enable masking when it is more understood`
        self.stats.layers_flattened += 1;
        let alpha = self.cur_alpha() * alpha.clamp(0.0, 1.0);
        self.layers.push(LayerFrame {
            alpha,
            clipped: false,
        });
    }

    fn push_clip_layer(&mut self, transform: Affine, shape: &impl Shape) {
        self.flatten(shape, transform);
        // 轴对齐矩形特判:velato 的 `Renderer::append` 每帧开头必发一次覆盖整幅
        // 合成画布的 push_clip_layer(`render.rs:68`),它就是个矩形。把它落到
        // Painter 已有的矩形裁剪上,既正确又不要求 Painter 长出路径裁剪
        // (lottie-1 §6.2 / lottie-2 §2.4 都点名"根裁剪该特判走 push_clip")。
        //
        // **但特判对每一个 push_clip_layer 都生效,不只根裁剪**:图层遮罩
        // (`render.rs:129-133`)走的是同一个入口,而遮罩的 mode/opacity 已经被
        // 上游丢了。栈非空 = 这是遮罩不是根裁剪,单记一笔让它可观测
        let is_root = self.layers.is_empty();
        let rect = axis_aligned_rect(&self.scratch);
        match rect {
            Some((x, y, w, h)) => {
                self.stats.clips_applied += 1;
                if !is_root {
                    // Subtract / inverted 遮罩在这里会被画反 —— 见
                    // RenderStats::mask_clips_intersected 的文档
                    self.stats.mask_clips_intersected += 1;
                }
                self.out.push_clip_rect(x, y, w, h);
            }
            None => {
                // 已知缺口:非矩形裁剪(图层 masksProperties)整体失效,
                // 被遮住的部分会照常画出来。补它需要 Painter 长出路径裁剪
                self.stats.clips_ignored += 1;
            }
        }
        let alpha = self.cur_alpha();
        self.layers.push(LayerFrame {
            alpha,
            clipped: rect.is_some(),
        });
    }

    fn pop_layer(&mut self) {
        match self.layers.pop() {
            Some(f) => {
                if f.clipped {
                    self.out.pop_clip();
                }
            }
            // 栈失衡:不 panic、不给 sink 发多余的 pop_clip(那会把调用方的
            // 裁剪栈也带崩),只记一笔。R4「去 panic」红线
            None => self.stats.unbalanced_pops += 1,
        }
    }

    fn draw(
        &mut self,
        stroke: Option<&fixed::Stroke>,
        transform: Affine,
        brush: &fixed::Brush,
        shape: &impl Shape,
    ) {
        let Some(base) = self.brush_color(brush) else {
            return;
        };
        let color = multiply_alpha(base, self.cur_alpha());
        if color.a == 0 {
            self.stats.transparent_skipped += 1;
            return;
        }
        self.flatten(shape, transform);
        if self.scratch.is_empty() {
            self.stats.empty_paths_skipped += 1;
            return;
        }
        match stroke {
            Some(s) => {
                let style = self.stroke_style(s, transform);
                self.stats.strokes += 1;
                self.out.stroke_path(&self.scratch, &style, color);
            }
            None => {
                self.stats.fills += 1;
                // 恒 NonZero:RenderSink::draw 压根没有 fill rule 参数
                self.out.fill_path(&self.scratch, PathFill::NonZero, color);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// 仿射变换的等效均匀尺度 = √|det A|。各向同性时就是缩放倍数,
/// 各向异性时是两个奇异值的几何均值(见 [`affine_singular_values`])
pub(crate) fn affine_scale(t: Affine) -> f64 {
    let c = t.as_coeffs();
    (c[0] * c[3] - c[1] * c[2]).abs().sqrt()
}

/// 仿射线性部分的两个奇异值 `(sx, sy)`,`sx >= sy >= 0`。
///
/// 用的是 2×2 的闭式解(不需要迭代 SVD):把矩阵拆成"旋转 + 缩放"与
/// "旋转 + 切变"两半,`sx = q + r`、`sy = q - r`。
/// `sx / sy` 就是各向异性程度,`sx * sy = |det A|`(可用作自检)。
pub(crate) fn affine_singular_values(t: Affine) -> (f64, f64) {
    let [a, b, c, d, _, _] = t.as_coeffs();
    let e = (a + d) * 0.5;
    let f = (a - d) * 0.5;
    let g = (b + c) * 0.5;
    let h = (b - c) * 0.5;
    let q = (e * e + h * h).sqrt();
    let r = (f * f + g * g).sqrt();
    (q + r, (q - r).max(0.0))
}

fn rgba8_to_color(c: peniko::color::Rgba8) -> Color {
    Color::rgba(c.r, c.g, c.b, c.a)
}

fn multiply_alpha(c: Color, alpha: f32) -> Color {
    if alpha >= 1.0 {
        return c;
    }
    Color::rgba(c.r, c.g, c.b, (c.a as f32 * alpha.max(0.0)) as u8)
}

/// 渐变色标的均值色。
///
/// 取的是**色标沿 offset 的分段线性均值**(每段贡献 (c0+c1)/2 × 段长),
/// 不是色标的算术平均 —— 后者会让"0.0 红 / 0.9 红 / 1.0 蓝"这种色标表
/// 算出一个偏蓝三分之一的结果,而它肉眼上几乎全红。
///
/// 在 8bit sRGB 上平均是有意的近似:velato 的 README 自己就把
/// "Correct color stop handling" 列在缺失特性里,这里再讲究色彩空间没有意义。
fn average_stop_color(stops: &[peniko::ColorStop]) -> Color {
    let comps = |s: &peniko::ColorStop| {
        let c = s.color.to_alpha_color::<Srgb>().to_rgba8();
        [c.r as f64, c.g as f64, c.b as f64, c.a as f64]
    };
    match stops {
        [] => Color::rgba(0, 0, 0, 0),
        [only] => {
            let c = comps(only);
            Color::rgba(c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8)
        }
        _ => {
            let mut acc = [0.0f64; 4];
            let mut span = 0.0f64;
            for w in stops.windows(2) {
                let d = (w[1].offset - w[0].offset).max(0.0) as f64;
                if d <= 0.0 {
                    continue;
                }
                let (a, b) = (comps(&w[0]), comps(&w[1]));
                for i in 0..4 {
                    acc[i] += (a[i] + b[i]) * 0.5 * d;
                }
                span += d;
            }
            if span <= 0.0 {
                // 所有色标挤在同一个 offset 上(合法但退化):回落到算术平均
                for s in stops {
                    let c = comps(s);
                    for i in 0..4 {
                        acc[i] += c[i];
                    }
                }
                span = stops.len() as f64;
            }
            let px = |v: f64| (v / span).round().clamp(0.0, 255.0) as u8;
            Color::rgba(px(acc[0]), px(acc[1]), px(acc[2]), px(acc[3]))
        }
    }
}

/// 判定一条已烘焙的路径是不是轴对齐矩形;是就返回 (x, y, w, h)。
///
/// 只认"**单条子路径** + 4 个折点 + 每条边只变一个坐标"。对旋转过的矩形返回
/// None —— 宁可少裁也不能裁错(裁错 = 内容整块消失,比漏裁难查得多)。
///
/// 子路径必须只有一条:`MoveTo(0,0) LineTo(0,10) MoveTo(10,10) LineTo(10,0)`
/// 是**两条不相连的竖线**(裁剪它 = 什么都不剩),但它的四个点恰好落在
/// (0,0,10,10) 的四角上,只数点会把"裁掉一切"错判成"裁到整个矩形"。
fn axis_aligned_rect(path: &[PathCmd]) -> Option<(f32, f32, f32, f32)> {
    // 亚像素容差:变换乘完之后精确的 0 会变成 1e-13 量级的噪声
    const EPS: f32 = 1e-3;
    // 定长数组而不是 Vec:这个函数每帧至少跑一次(根裁剪),不值得为它分配
    let mut pts = [(0.0f32, 0.0f32); 5];
    let mut n = 0usize;
    let mut moves = 0usize;
    for c in path {
        match *c {
            PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => {
                if matches!(*c, PathCmd::MoveTo(..)) {
                    moves += 1;
                    // 第二条子路径 —— 四个点的包围盒不再代表被裁区域
                    if moves > 1 {
                        return None;
                    }
                }
                if n == pts.len() {
                    return None;
                }
                pts[n] = (x, y);
                n += 1;
            }
            PathCmd::Close => {}
            _ => return None,
        }
    }
    if moves != 1 {
        return None;
    }
    // 显式回到起点的写法(MoveTo..LineTo 回起点)与隐式闭合都要认
    if n == 5 && (pts[0].0 - pts[4].0).abs() <= EPS && (pts[0].1 - pts[4].1).abs() <= EPS {
        n = 4;
    }
    if n != 4 {
        return None;
    }
    for i in 0..4 {
        let a = pts[i];
        let b = pts[(i + 1) % 4];
        let dx = (a.0 - b.0).abs();
        let dy = (a.1 - b.1).abs();
        // 每条边必须恰好只在一个方向上有长度
        if !((dx <= EPS) ^ (dy <= EPS)) {
            return None;
        }
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    // 只看前 4 个:数组是定长的,第 5 格要么是起点的重复、要么是没写过的 0
    for (x, y) in &pts[..4] {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

// ---------------------------------------------------------------------------
// 单测:私有函数与"只能从 RenderSink 那一侧驱动"的降级路径
// ---------------------------------------------------------------------------
//
// 这些路径要么走不到(velato 0.11 结构上不发 Brush::Image / dash),要么
// 需要手搓一个畸形的 lottie 才能触发。集成测试够不到,但"降级可观测"这条
// 属性正是靠它们守着 —— 只断言计数器等于 0 是守不住的。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::{RecordingSink, SinkCmd};
    use peniko::kurbo::{BezPath, Rect, Stroke as KStroke};
    use peniko::{ColorStop, Gradient};
    use velato::RenderSink as _;

    fn stop(offset: f32, r: u8, g: u8, b: u8) -> ColorStop {
        ColorStop {
            offset,
            color: peniko::color::DynamicColor::from_alpha_color(
                peniko::color::AlphaColor::<Srgb>::from_rgba8(r, g, b, 255),
            ),
        }
    }

    // --- average_stop_color ------------------------------------------------

    #[test]
    fn gradient_average_is_weighted_by_offset_span() {
        // 0.0 红 / 0.9 红 / 1.0 蓝:肉眼上几乎全红,算术平均会给出偏蓝 1/3
        let c = average_stop_color(&[
            stop(0.0, 255, 0, 0),
            stop(0.9, 255, 0, 0),
            stop(1.0, 0, 0, 255),
        ]);
        assert_eq!((c.r, c.g, c.b, c.a), (242, 0, 13, 255), "实际 {c:?}");
        // 算术平均会是 (170, 0, 85) —— 明显偏蓝
        assert!(c.r > 200 && c.b < 40);
    }

    #[test]
    fn gradient_average_handles_degenerate_stop_tables() {
        // 空色标表:透明黑(而不是除零)
        assert_eq!(average_stop_color(&[]), Color::rgba(0, 0, 0, 0));
        // 单色标:原样
        let one = average_stop_color(&[stop(0.3, 10, 20, 30)]);
        assert_eq!((one.r, one.g, one.b, one.a), (10, 20, 30, 255));
        // 全部挤在同一个 offset(合法但退化):回落到算术平均,不是 NaN
        let same = average_stop_color(&[stop(0.5, 0, 0, 0), stop(0.5, 255, 255, 255)]);
        assert_eq!((same.r, same.g, same.b), (128, 128, 128), "实际 {same:?}");
    }

    // --- axis_aligned_rect -------------------------------------------------

    fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathCmd> {
        vec![
            PathCmd::MoveTo(x0, y0),
            PathCmd::LineTo(x1, y0),
            PathCmd::LineTo(x1, y1),
            PathCmd::LineTo(x0, y1),
            PathCmd::Close,
        ]
    }

    #[test]
    fn axis_aligned_rect_accepts_both_closing_styles() {
        assert_eq!(
            axis_aligned_rect(&rect_path(10.0, 20.0, 40.0, 60.0)),
            Some((10.0, 20.0, 30.0, 40.0))
        );
        // 显式回到起点的第 5 个点
        let mut explicit = rect_path(10.0, 20.0, 40.0, 60.0);
        explicit.insert(4, PathCmd::LineTo(10.0, 20.0));
        assert_eq!(
            axis_aligned_rect(&explicit),
            Some((10.0, 20.0, 30.0, 40.0)),
            "显式闭合也要认"
        );
    }

    #[test]
    fn axis_aligned_rect_rejects_non_rects() {
        // 曲线
        assert_eq!(
            axis_aligned_rect(&[
                PathCmd::MoveTo(0.0, 0.0),
                PathCmd::CubicTo(1.0, 1.0, 2.0, 2.0, 3.0, 3.0),
            ]),
            None
        );
        // 旋转过的矩形(每条边两个方向都在变)
        assert_eq!(
            axis_aligned_rect(&[
                PathCmd::MoveTo(0.0, 5.0),
                PathCmd::LineTo(5.0, 0.0),
                PathCmd::LineTo(10.0, 5.0),
                PathCmd::LineTo(5.0, 10.0),
                PathCmd::Close,
            ]),
            None
        );
        // 三角形
        assert_eq!(
            axis_aligned_rect(&[
                PathCmd::MoveTo(0.0, 0.0),
                PathCmd::LineTo(10.0, 0.0),
                PathCmd::LineTo(0.0, 10.0),
                PathCmd::Close,
            ]),
            None
        );
    }

    #[test]
    fn axis_aligned_rect_rejects_two_disjoint_subpaths() {
        // 两条不相连的竖线:裁剪它等于"什么都不剩",但四个点恰好落在
        // (0,0,10,10) 的四角上。只数点会把"裁掉一切"错判成"裁到整个矩形"
        let two_lines = [
            PathCmd::MoveTo(0.0, 0.0),
            PathCmd::LineTo(0.0, 10.0),
            PathCmd::MoveTo(10.0, 10.0),
            PathCmd::LineTo(10.0, 0.0),
        ];
        assert_eq!(axis_aligned_rect(&two_lines), None);
    }

    // --- 仿射尺度 ----------------------------------------------------------

    #[test]
    fn singular_values_expose_anisotropy_that_det_hides() {
        // squash-and-stretch:det = 1,√|det| 给出 1.0 的"零修正"
        let squash = Affine::scale_non_uniform(4.0, 0.25);
        assert!((affine_scale(squash) - 1.0).abs() < 1e-12, "det 看不出来");
        let (sx, sy) = affine_singular_values(squash);
        assert!(
            (sx - 4.0).abs() < 1e-9 && (sy - 0.25).abs() < 1e-9,
            "{sx} {sy}"
        );
        // 各向同性:两个奇异值相等
        let (sx, sy) = affine_singular_values(Affine::scale(3.0));
        assert!((sx - 3.0).abs() < 1e-9 && (sy - 3.0).abs() < 1e-9);
        // 旋转不改变奇异值
        let (sx, sy) = affine_singular_values(Affine::rotate(0.7) * Affine::scale(2.0));
        assert!((sx - 2.0).abs() < 1e-9 && (sy - 2.0).abs() < 1e-9);
        // 恒等式自检:sx * sy == |det|
        let t = Affine::rotate(0.3) * Affine::scale_non_uniform(5.0, 0.5);
        let (sx, sy) = affine_singular_values(t);
        assert!((sx * sy - affine_scale(t).powi(2)).abs() < 1e-9);
    }

    // --- 描边降级 ----------------------------------------------------------

    #[test]
    fn anisotropic_scale_flags_the_stroke_width_as_approximate() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let stroke = KStroke::new(4.0);
        // det = 1 → 补偿因子恰好 1.0 = 零修正,正解是横 16 / 纵 1
        let style = sink.stroke_style(&stroke, Affine::scale_non_uniform(4.0, 0.25));
        assert_eq!(style.width, 4.0, "宽度没被修正 —— 这正是要报出来的事");
        assert_eq!(sink.stats().stroke_width_approximated, 1);
    }

    #[test]
    fn isotropic_scale_is_exact_and_not_flagged() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let stroke = KStroke::new(3.0);
        let style = sink.stroke_style(&stroke, Affine::scale(3.0));
        assert_eq!(style.width, 9.0, "各向同性下补偿是精确的");
        assert_eq!(sink.stats().stroke_width_approximated, 0);
        // 旋转 + 均匀缩放同样是各向同性
        let style = sink.stroke_style(&stroke, Affine::rotate(1.1) * Affine::scale(2.0));
        assert_eq!(style.width, 6.0);
        assert_eq!(sink.stats().stroke_width_approximated, 0);
    }

    #[test]
    fn dash_guard_would_fire_if_upstream_ever_emitted_one() {
        // velato 0.11 恒不发 dash(runtime/model/animated.rs:414 只写
        // Stroke::new().with_caps().with_join()),所以这个计数器在真实资产上
        // 恒为 0。这条测试钉住的是"上游哪天开始发,我们会报出来"这个前向承诺
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let dashed = KStroke::new(2.0).with_dashes(0.0, [4.0, 2.0]);
        sink.stroke_style(&dashed, Affine::IDENTITY);
        assert_eq!(sink.stats().dashes_ignored, 1);
        assert_eq!(sink.stats().stroke_cap_mismatch, 0);
    }

    #[test]
    fn cap_mismatch_guard_would_fire_if_upstream_ever_emitted_one() {
        // 同上:with_caps 把 start/end 设成同一个值,恒不触发
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let mut s = KStroke::new(2.0);
        s.start_cap = Cap::Round;
        s.end_cap = Cap::Butt;
        let style = sink.stroke_style(&s, Affine::IDENTITY);
        assert_eq!(sink.stats().stroke_cap_mismatch, 1);
        assert_eq!(style.cap, LineCap::Round, "取 start_cap");
    }

    // --- 画刷降级 ----------------------------------------------------------

    #[test]
    fn image_brush_drops_the_whole_draw() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let img = peniko::ImageBrush::new(peniko::ImageData {
            data: peniko::Blob::new(std::sync::Arc::new(vec![0u8; 4])),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: 1,
            height: 1,
        });
        sink.draw(
            None,
            Affine::IDENTITY,
            &Brush::Image(img),
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let stats = sink.finish();
        assert_eq!(stats.image_brushes_skipped, 1);
        assert_eq!(stats.fills, 0, "整条 draw 丢弃,不画黑块");
        assert!(out.cmds.is_empty());
    }

    #[test]
    fn gradient_brush_is_flattened_to_one_solid_color() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let g = Gradient::new_linear((0.0, 0.0), (10.0, 0.0))
            .with_stops([stop(0.0, 255, 0, 0), stop(1.0, 0, 0, 255)].as_slice());
        sink.draw(
            None,
            Affine::IDENTITY,
            &Brush::Gradient(g),
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let stats = sink.finish();
        assert_eq!(stats.gradient_fallbacks, 1);
        assert_eq!(stats.fills, 1);
        match out.cmds.as_slice() {
            [SinkCmd::Fill { color, .. }] => {
                assert_eq!((color.r, color.g, color.b), (128, 0, 128), "红蓝各半");
            }
            other => panic!("期望一条纯色填充,实际 {other:?}"),
        }
    }

    // --- 图层栈 ------------------------------------------------------------

    #[test]
    fn extra_pop_layer_is_counted_not_forwarded() {
        // 多余的 pop 绝不能转发给 sink —— 那会把调用方自己的裁剪栈也带崩
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        sink.pop_layer();
        sink.pop_layer();
        let stats = sink.finish();
        assert_eq!(stats.unbalanced_pops, 2);
        assert_eq!(stats.unclosed_layers, 0);
        assert!(out.cmds.is_empty(), "一条 pop_clip 都不该发出去");
    }

    #[test]
    fn unclosed_clips_are_drained_on_finish() {
        // 少 pop:欠下的 pop_clip 必须补发,否则宿主 Painter 的裁剪栈被永久污染
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        sink.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 10.0, 10.0));
        sink.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 5.0, 5.0));
        let stats = sink.finish();
        assert_eq!(stats.unclosed_layers, 2);
        assert_eq!(
            out.cmds
                .iter()
                .filter(|c| matches!(c, SinkCmd::PopClip))
                .count(),
            2,
            "两次 push_clip_rect 必须换来两次 pop_clip:{:?}",
            out.cmds
        );
    }

    #[test]
    fn unclosed_clips_are_drained_on_drop_too() {
        // finish() 走不到的路径:append 中途 panic 展开。Drop 是兜底
        let mut out = RecordingSink::default();
        {
            let mut sink = PainterSink::new(&mut out);
            sink.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 10.0, 10.0));
            // 不调 finish,直接离开作用域
        }
        assert_eq!(
            out.cmds.last(),
            Some(&SinkCmd::PopClip),
            "Drop 也要补发 pop_clip:{:?}",
            out.cmds
        );
    }

    #[test]
    fn ignored_clip_does_not_emit_a_pop() {
        // 非矩形裁剪被忽略 → 这一格没发过 push_clip_rect,pop 时也不能发 pop_clip
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        let mut tri = BezPath::new();
        tri.move_to((0.0, 0.0));
        tri.line_to((10.0, 0.0));
        tri.line_to((0.0, 10.0));
        tri.close_path();
        sink.push_clip_layer(Affine::IDENTITY, &tri);
        sink.pop_layer();
        let stats = sink.finish();
        assert_eq!(stats.clips_ignored, 1);
        assert_eq!(stats.clips_applied, 0);
        assert!(out.cmds.is_empty(), "既没 push 也没 pop:{:?}", out.cmds);
    }

    #[test]
    fn nested_rect_clip_is_flagged_as_a_mask_with_unknown_mode() {
        // 根裁剪不算遮罩;它下面那层矩形裁剪是遮罩,mode 已被上游丢弃
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        sink.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 200.0, 100.0));
        sink.push_clip_layer(Affine::IDENTITY, &Rect::new(0.0, 0.0, 100.0, 100.0));
        sink.pop_layer();
        sink.pop_layer();
        let stats = sink.finish();
        assert_eq!(stats.clips_applied, 2);
        assert_eq!(stats.mask_clips_intersected, 1, "只有非根的那一次算遮罩");
        assert!(stats.degraded(), "带遮罩的一帧不该被报成零降级");
    }

    // --- alpha 与空路径 ----------------------------------------------------

    #[test]
    fn layer_alpha_multiplies_into_subsequent_colors() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        sink.push_layer(
            peniko::Mix::Normal,
            0.5,
            Affine::IDENTITY,
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        sink.draw(
            None,
            Affine::IDENTITY,
            &Brush::Solid(peniko::color::AlphaColor::<Srgb>::from_rgba8(9, 9, 9, 200)),
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let stats = sink.finish();
        assert_eq!(stats.layers_flattened, 1);
        match out.cmds.as_slice() {
            [SinkCmd::Fill { color, .. }] => assert_eq!(color.a, 100, "200 × 0.5"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fully_transparent_draw_is_skipped_before_flattening() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        sink.push_layer(
            peniko::Mix::Normal,
            0.0,
            Affine::IDENTITY,
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        sink.draw(
            None,
            Affine::IDENTITY,
            &Brush::Solid(peniko::color::AlphaColor::<Srgb>::from_rgba8(9, 9, 9, 255)),
            &Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let stats = sink.finish();
        assert_eq!(stats.transparent_skipped, 1);
        assert_eq!(stats.fills, 0);
        assert!(out.cmds.is_empty());
    }

    #[test]
    fn empty_path_draw_is_counted_not_silently_dropped() {
        let mut out = RecordingSink::default();
        let mut sink = PainterSink::new(&mut out);
        sink.draw(
            None,
            Affine::IDENTITY,
            &Brush::Solid(peniko::color::AlphaColor::<Srgb>::from_rgba8(1, 2, 3, 255)),
            &BezPath::new(),
        );
        let stats = sink.finish();
        assert_eq!(stats.empty_paths_skipped, 1);
        assert_eq!(stats.fills, 0);
        assert!(out.cmds.is_empty());
    }

    // --- 容差 --------------------------------------------------------------

    #[test]
    fn bogus_tolerance_falls_back_to_the_default() {
        let mut out = RecordingSink::default();
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let sink = PainterSink::with_tolerance(&mut out, bad);
            assert_eq!(sink.tolerance, DEFAULT_TOLERANCE, "{bad} 应当被消毒");
        }
        let sink = PainterSink::with_tolerance(&mut out, 2.5);
        assert_eq!(sink.tolerance, 2.5, "合法值原样保留");
    }

    #[test]
    fn a_coarser_tolerance_emits_fewer_path_commands() {
        // 容差只对解析形状(Circle/Rect)起作用,BezPath 原样吐出
        let circle = peniko::kurbo::Circle::new((50.0, 50.0), 40.0);
        let count = |tol: f64| {
            let mut out = RecordingSink::default();
            {
                let mut sink = PainterSink::with_tolerance(&mut out, tol);
                sink.draw(
                    None,
                    Affine::IDENTITY,
                    &Brush::Solid(peniko::color::AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255)),
                    &circle,
                );
                let _ = sink.finish();
            }
            match out.cmds.as_slice() {
                [SinkCmd::Fill { cmds, .. }] => *cmds,
                other => panic!("{other:?}"),
            }
        };
        let fine = count(0.001);
        let coarse = count(5.0);
        assert!(
            fine > coarse,
            "细容差 {fine} 应当比粗容差 {coarse} 出更多段"
        );
    }
}
