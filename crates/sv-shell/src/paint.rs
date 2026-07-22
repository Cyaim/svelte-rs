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

use sv_ui::Color;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

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
    /// 能否合成外部 wgpu 纹理(`<surface3d>` 的前置;CPU 后端恒 false)
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
