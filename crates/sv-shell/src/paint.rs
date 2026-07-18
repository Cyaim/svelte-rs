//! Painter 抽象 —— 可切换渲染后端的边界(调研 14 裁决落地)。
//!
//! 设计要点:
//! - **trait 即时调用**为接口形态;[`RecordingPainter`] 是它的显示列表实现,
//!   免费获得金样测试(零像素、零 GPU),未来可升级为帧间 diff 载体;
//! - 词汇对齐 vello `Scene` 的动词(fill/stroke/glyph run/layer),M2 接
//!   vello 时 1:1 映射;
//! - **文本走定位好的 glyph run**:shaping 在上层(text 门面),光栅在
//!   backend 内(CPU 端按字形 id 光栅,GPU 端走 atlas)——painter 不拿
//!   字符串也不拿位图(Slint 软件渲染器与 GPU 灾难的双重教训);
//! - `dyn` 只存在于 sv-shell 边界内,严禁类型参数上浮到 sv-ui/编译器产物
//!   (tachys 泛型爆炸的教训;这里每帧低千级动态调用 ≈ 个位数 µs)。
//!
//! 坐标:物理像素(调用方已乘 scale)。

use fontdue::Font;
use fontdue::layout::GlyphRasterConfig;
use sv_ui::Color;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

/// 一枚已定位字形(物理坐标)。
/// CPU 路径用 (key, x, y):位图左上角 + 光栅配置;
/// GPU 路径用 (id, ox, oy):字形 id + **基线原点**(vello draw_glyphs 语义)
#[derive(Clone, Copy, Debug)]
pub struct GlyphPos {
    pub key: GlyphRasterConfig,
    /// 位图左上角 x(CPU 光栅路径)
    pub x: f32,
    /// 位图左上角 y(CPU 光栅路径)
    pub y: f32,
    /// 字形 id(GPU 路径)
    pub id: u16,
    /// 基线原点 x(GPU 路径)
    pub ox: f32,
    /// 基线原点 y(GPU 路径)
    pub oy: f32,
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
    /// 一段已定位字形(shaping 已完成;backend 只负责光栅/上屏)
    fn glyph_run(&mut self, font: &Font, glyphs: &[GlyphPos], color: Color);
}

// ---------------------------------------------------------------------------
// 记录型后端:命令快照(金样测试 / 未来缓存载体)
// ---------------------------------------------------------------------------

/// 简化命令(数值取整,快照稳定;字形只记数量与颜色)
#[derive(Clone, PartialEq, Debug)]
pub enum PaintCmd {
    FillRect { x: i32, y: i32, w: i32, h: i32, radius: i32, color: Color },
    StrokeRect { x: i32, y: i32, w: i32, h: i32, width: i32, color: Color },
    Glyphs { count: usize, color: Color },
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

    fn stroke_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32, color: Color) {
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

    fn glyph_run(&mut self, _font: &Font, glyphs: &[GlyphPos], color: Color) {
        self.cmds.push(PaintCmd::Glyphs { count: glyphs.len(), color });
    }
}

// ---------------------------------------------------------------------------
// tiny-skia CPU 后端(首个真实实现;能力冻结,定位过渡与测试基准)
// ---------------------------------------------------------------------------

pub struct TinySkiaPainter<'a> {
    pub pixmap: &'a mut Pixmap,
}

/// 字形覆盖度 LRU(线程级):同一字形同字号只光栅一次。
/// 上限按条目数粗控(每条 ≈ 字号² 字节;2048 条 @16px ≈ 1.3MB)
mod glyph_cache {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use fontdue::Metrics;
    use fontdue::layout::GlyphRasterConfig;

    const CAP: usize = 2048;

    thread_local! {
        static CACHE: RefCell<HashMap<GlyphRasterConfig, (Metrics, Vec<u8>)>> =
            RefCell::new(HashMap::new());
    }

    pub fn with<R>(
        font: &fontdue::Font,
        key: GlyphRasterConfig,
        f: impl FnOnce(&Metrics, &[u8]) -> R,
    ) -> R {
        CACHE.with(|c| {
            let mut cache = c.borrow_mut();
            if !cache.contains_key(&key) {
                // 简化淘汰:超限整体清空(避免逐条 LRU 记账;
                // 稳态 UI 的工作集远小于上限,清空是罕见事件)
                if cache.len() >= CAP {
                    cache.clear();
                }
                cache.insert(key, font.rasterize_config(key));
            }
            let (m, cov) = &cache[&key];
            f(m, cov)
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
    pb.cubic_to(x + w, y + h - r + K * r, x + w - r + K * r, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - K * r, y + h, x, y + h - r + K * r, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - K * r, x + r - K * r, y, x + r, y);
    pb.close();
}

fn blend_pixel(data: &mut [PremultipliedColorU8], pw: u32, ph: u32, x: i32, y: i32, c: Color, cov: u8) {
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
            self.pixmap
                .fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    fn stroke_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32, color: Color) {
        // 沿边框中心线描边(内缩半宽),视觉贴合 border-box
        let half = width / 2.0;
        let mut pb = PathBuilder::new();
        rounded_rect_path(&mut pb, x + half, y + half, w - width, h - width, (radius - half).max(0.0));
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color(skia_color(color));
            paint.anti_alias = true;
            let stroke = Stroke { width, ..Stroke::default() };
            self.pixmap
                .stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    fn glyph_run(&mut self, font: &Font, glyphs: &[GlyphPos], color: Color) {
        let (pw, ph) = (self.pixmap.width(), self.pixmap.height());
        let data = self.pixmap.pixels_mut();
        for g in glyphs {
            glyph_cache::with(font, g.key, |metrics, coverage| {
                for yy in 0..metrics.height {
                    for xx in 0..metrics.width {
                        let cov = coverage[yy * metrics.width + xx];
                        if cov == 0 {
                            continue;
                        }
                        blend_pixel(
                            data,
                            pw,
                            ph,
                            g.x.round() as i32 + xx as i32,
                            g.y.round() as i32 + yy as i32,
                            color,
                            cov,
                        );
                    }
                }
            });
        }
    }
}
