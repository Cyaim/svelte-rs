//! 路径动词词汇 —— **`sv_shell::Painter` 的镜像**。
//!
//! ## 为什么是"镜像"而不是直接 `use sv_shell::PathCmd`
//!
//! `sv-shell` 的 `mod paint` 是**私有模块**,而 `lib.rs` 的
//! `pub use paint::{...}` 只导出了 `Painter` / `PaintCmd` / `RecordingPainter` /
//! `TinySkiaPainter`,**没有导出 `PathCmd` / `PathFill` / `StrokeStyle` /
//! `LineCap` / `LineJoin`**。这五个类型于是"公开但不可命名"(rustc:
//! `E0603: module 'paint' is private ... enum 'PathCmd' is not publicly
//! re-exported`,实测),外部 crate 既无法写出它们的类型名,也无法构造它们的值
//! —— 连 `Painter::fill_path` 都因此调不动。
//!
//! 所以本模块把这套词汇按**同名同形**复制一份,并让 [`PathSink`] 的方法签名
//! 与 `Painter` 的路径动词逐字对齐。等 sv-shell 补上那一行 re-export,
//! 桥接就退化成一个纯搬运的 `for` 循环(README 里有原文)。
//!
//! 坐标口径同 `Painter`:**物理像素,变换已由调用方烘焙进坐标**
//! (`Painter` 的路径动词不收 transform 参数,这是 sv-shell 的既有不变量)。

use sv_ui::Color;

/// 路径命令。五个变体与 `sv_shell::PathCmd` 一一对应,
/// 也与 `kurbo::PathEl` 一一对应(圆弧/椭圆在 kurbo 侧就已经是三次贝塞尔)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// 二次贝塞尔:(控制点, 终点)
    QuadTo(f32, f32, f32, f32),
    /// 三次贝塞尔:(控制点 1, 控制点 2, 终点)。lottie 的主力曲线
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// 填充规则。
///
/// **lottie 这条线永远只会产出 `NonZero`**:velato 的 `RenderSink::draw`
/// 签名里根本没有 fill rule 参数,它自带的 `impl RenderSink for vello::Scene`
/// 也是硬编码 `Fill::NonZero`。`EvenOdd` 留在这里是为了与 sv-shell 的
/// `PathFill` 形状一致(它的价值在 SVG 图标那边:带孔图标)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PathFill {
    #[default]
    NonZero,
    EvenOdd,
}

/// 线端形状
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// 折点形状
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// 描边风格(打包成结构体,与 `sv_shell::StrokeStyle` 同形)
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    /// 斜接上限(超过退化为 Bevel);SVG/lottie 缺省 4.0
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

/// lottie 帧的出口。**四个动词是 `sv_shell::Painter` 的严格子集**:
/// `fill_path` / `stroke_path` 逐字相同,`push_clip_rect` / `pop_clip`
/// 对应 `Painter::push_clip(x, y, w, h, radius=0)` / `pop_clip`。
///
/// 裁剪两项给了默认空实现,理由:velato 每帧开头必发一次覆盖整幅合成画布的
/// `push_clip_layer`(`render.rs:68`),对"目标矩形就是合成画布"的常见用法
/// 是恒等操作,不该逼着每个实现者都处理它。
///
/// **但不实现它 = 图层遮罩全部失效**(不只是画布外那点边角):矩形的
/// `masksProperties` 也走 `push_clip_rect`。真实沿用请把这两个动词接到宿主的
/// 裁剪栈上,并保证**成对**;`tests/lottie.rs` 的 `PixmapSink` 是个 20 行的样板。
/// 这一条有对照测试守着(`a_clip_that_the_sink_honours_really_removes_pixels`:
/// 实现裁剪 ≈10000 像素,不实现 20000 像素)
pub trait PathSink {
    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color);
    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color);

    /// 压入矩形裁剪(轴对齐,物理像素)。默认忽略
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let _ = (x, y, w, h);
    }
    /// 弹出裁剪。默认忽略。**与 `push_clip_rect` 必须成对实现**
    fn pop_clip(&mut self) {}
}

/// 让 `&mut S` 也是 sink —— 调用方常见的是手里只有 `&mut dyn PathSink`
impl<S: PathSink + ?Sized> PathSink for &mut S {
    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        (**self).fill_path(path, fill, color);
    }
    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        (**self).stroke_path(path, style, color);
    }
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        (**self).push_clip_rect(x, y, w, h);
    }
    fn pop_clip(&mut self) {
        (**self).pop_clip();
    }
}

// ---------------------------------------------------------------------------
// 记录型 sink:金样测试用
// ---------------------------------------------------------------------------

/// 简化命令。口径**刻意抄 `sv_shell::PaintCmd`**:只记条数 + 规则 + 颜色 +
/// 取整包围盒。逐点记录会让金样长到没人看得懂(一帧几千个点),而且 velato
/// 打磨一条缓动曲线就会让金样全红;包围盒足以抓住"画错位置/整层丢了"
#[derive(Clone, PartialEq, Debug)]
pub enum SinkCmd {
    Fill {
        cmds: usize,
        fill: PathFill,
        color: Color,
        bbox: (i32, i32, i32, i32),
    },
    Stroke {
        cmds: usize,
        width: i32,
        cap: LineCap,
        join: LineJoin,
        color: Color,
        bbox: (i32, i32, i32, i32),
    },
    PushClip {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    PopClip,
}

/// 把一帧记成可比对的命令流(零像素、零后端)
#[derive(Default, Debug)]
pub struct RecordingSink {
    pub cmds: Vec<SinkCmd>,
}

impl PathSink for RecordingSink {
    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        self.cmds.push(SinkCmd::Fill {
            cmds: path.len(),
            fill,
            color,
            bbox: path_bbox_i32(path),
        });
    }

    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        self.cmds.push(SinkCmd::Stroke {
            cmds: path.len(),
            width: style.width as i32,
            cap: style.cap,
            join: style.join,
            color,
            bbox: path_bbox_i32(path),
        });
    }

    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.cmds.push(SinkCmd::PushClip {
            x: x as i32,
            y: y as i32,
            w: w as i32,
            h: h as i32,
        });
    }

    fn pop_clip(&mut self) {
        self.cmds.push(SinkCmd::PopClip);
    }
}

/// 路径包围盒(取整)。控制点也计入 —— 曲线不会超出控制点凸包,
/// 所以这是个**保守**包围盒。与 sv-shell `path_bbox_i32` 同口径。
///
/// 空路径(以及只有 `Close` 的路径)返回 `(0, 0, 0, 0)`,与"原点处一个点"
/// 无法区分。这只影响金样可读性:适配器在 `draw` 里就把空路径挡掉并计进
/// [`crate::RenderStats::empty_paths_skipped`] 了,不会走到这里
pub(crate) fn path_bbox_i32(path: &[PathCmd]) -> (i32, i32, i32, i32) {
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
