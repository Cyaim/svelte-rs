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

/// 字形光栅键:字形 id + 字号(f32 以位模式存储,保 Hash/Eq)。
/// 单字体前提下这两项唯一决定一张覆盖度位图(HiDPI 已把 scale 乘进 px)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphKey {
    /// 字形 id(swash charmap 映射;TTC 内 collection index 0 的字体)
    pub id: u16,
    /// 字号的 f32 位模式(`f32::to_bits`)
    pub px_bits: u32,
}

impl GlyphKey {
    pub fn new(id: u16, px: f32) -> Self {
        Self {
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
    /// 一段已定位字形(shaping 已完成;backend 只负责光栅/上屏。
    /// 字体是全局单字体 UI 字体——`font::ui_font()` / vello FontData 各自持有)
    fn glyph_run(&mut self, glyphs: &[GlyphPos], color: Color);
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

    fn glyph_run(&mut self, glyphs: &[GlyphPos], color: Color) {
        self.cmds.push(PaintCmd::Glyphs {
            count: glyphs.len(),
            color,
        });
    }
}

// ---------------------------------------------------------------------------
// tiny-skia CPU 后端(首个真实实现;能力冻结,定位过渡与测试基准)
// ---------------------------------------------------------------------------

pub struct TinySkiaPainter<'a> {
    pub pixmap: &'a mut Pixmap,
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
            let mut scaler = ctx
                .builder(crate::font::ui_font())
                .size(key.px())
                .hint(false)
                .build();
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

    fn glyph_run(&mut self, glyphs: &[GlyphPos], color: Color) {
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
                        blend_pixel(data, pw, ph, x0 + xx as i32, y0 + yy as i32, color, cov);
                    }
                }
            });
        }
    }
}
