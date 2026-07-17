//! 布局 + 绘制(CPU 自绘原型)
//!
//! 布局是极简的行/列堆叠(padding + gap + 固定宽高覆盖),TODO 换 taffy。
//! 绘制走 tiny-skia(矢量)+ fontdue(字形光栅),TODO 换 vello + parley。
//! 逻辑坐标布局、物理坐标绘制(乘 scale),保证 HiDPI 下文字清晰。

use std::collections::HashMap;

use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, TextStyle};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Transform};

use sv_ui::{Color, Direction, Doc, DocumentInner, ElementKind, ViewId};

use crate::font::ui_font;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// 一次布局的产物:绘制顺序排列(父先子后),rect 为逻辑坐标
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub id: ViewId,
    pub rect: Rect,
}

pub fn measure_text(font: &Font, text: &str, px: f32) -> (f32, f32) {
    if text.is_empty() {
        return (0.0, px * 1.25);
    }
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.append(&[font], &TextStyle::new(text, px, 0));
    let w = layout
        .glyphs()
        .iter()
        .map(|g| g.x + g.width as f32)
        .fold(0.0f32, f32::max);
    (w, layout.height())
}

fn measure(
    inner: &DocumentInner,
    font: &Font,
    id: ViewId,
    cache: &mut HashMap<ViewId, (f32, f32)>,
) -> (f32, f32) {
    if let Some(sz) = cache.get(&id) {
        return *sz;
    }
    let n = &inner.nodes[id];
    let s = &n.style;
    let (mut w, mut h) = match n.kind {
        ElementKind::Text | ElementKind::Button => {
            let (tw, th) = measure_text(font, &n.text, s.font_size);
            (tw + s.padding * 2.0, th + s.padding * 2.0)
        }
        ElementKind::View => {
            let mut main = 0.0f32;
            let mut cross = 0.0f32;
            let mut count = 0usize;
            for c in &n.children {
                let (cw, ch) = measure(inner, font, *c, cache);
                match s.direction {
                    Direction::Row => {
                        main += cw;
                        cross = cross.max(ch);
                    }
                    Direction::Column => {
                        main += ch;
                        cross = cross.max(cw);
                    }
                }
                count += 1;
            }
            if count > 1 {
                main += s.gap * (count as f32 - 1.0);
            }
            match s.direction {
                Direction::Row => (main + s.padding * 2.0, cross + s.padding * 2.0),
                Direction::Column => (cross + s.padding * 2.0, main + s.padding * 2.0),
            }
        }
    };
    if let Some(fw) = s.width {
        w = fw;
    }
    if let Some(fh) = s.height {
        h = fh;
    }
    cache.insert(id, (w, h));
    (w, h)
}

fn place(
    inner: &DocumentInner,
    font: &Font,
    cache: &mut HashMap<ViewId, (f32, f32)>,
    id: ViewId,
    x: f32,
    y: f32,
    forced: Option<(f32, f32)>,
    out: &mut Vec<Placed>,
) {
    let (w, h) = forced.unwrap_or_else(|| measure(inner, font, id, cache));
    out.push(Placed { id, rect: Rect { x, y, w, h } });
    let n = &inner.nodes[id];
    if n.kind != ElementKind::View {
        return;
    }
    let s = n.style.clone();
    let mut cx = x + s.padding;
    let mut cy = y + s.padding;
    for c in &n.children {
        let (cw, ch) = measure(inner, font, *c, cache);
        place(inner, font, cache, *c, cx, cy, None, out);
        match s.direction {
            Direction::Row => cx += cw + s.gap,
            Direction::Column => cy += ch + s.gap,
        }
    }
}

/// 布局整棵树。root 强制占满窗口逻辑尺寸
pub fn layout_tree(doc: &Doc, logical_w: f32, logical_h: f32) -> Vec<Placed> {
    let font = ui_font();
    doc.read(|inner| {
        let mut cache = HashMap::new();
        let mut out = Vec::new();
        place(
            inner,
            font,
            &mut cache,
            inner.root,
            0.0,
            0.0,
            Some((logical_w, logical_h)),
            &mut out,
        );
        out
    })
}

fn skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn rounded_rect(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32, r: f32) {
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

fn draw_text(pixmap: &mut Pixmap, font: &Font, text: &str, px: f32, color: Color, ox: f32, oy: f32) {
    if text.is_empty() {
        return;
    }
    let (pw, ph) = (pixmap.width(), pixmap.height());
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.append(&[font], &TextStyle::new(text, px, 0));
    // glyphs() 返回引用,先收集 key 再逐个光栅,避免与 font 借用打架
    let glyphs: Vec<_> = layout.glyphs().to_vec();
    let data = pixmap.pixels_mut();
    for g in glyphs {
        if g.width == 0 {
            continue;
        }
        let (metrics, coverage) = font.rasterize_config(g.key);
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
                    (ox + g.x).round() as i32 + xx as i32,
                    (oy + g.y).round() as i32 + yy as i32,
                    color,
                    cov,
                );
            }
        }
    }
}

/// 渲染一帧:布局(逻辑坐标)+ 绘制(物理坐标)。返回像素与命中测试用的布局
pub fn render_frame(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> (Pixmap, Vec<Placed>) {
    let font = ui_font();
    let logical_w = phys_w as f32 / scale;
    let logical_h = phys_h as f32 / scale;
    let placed = layout_tree(doc, logical_w, logical_h);

    let mut pixmap = Pixmap::new(phys_w.max(1), phys_h.max(1)).expect("sv-shell: 创建 pixmap 失败");
    pixmap.fill(skia_color(Color::WHITE));

    doc.read(|inner| {
        for p in &placed {
            let Some(n) = inner.nodes.get(p.id) else { continue };
            let s = &n.style;
            let (x, y, w, h) = (p.rect.x * scale, p.rect.y * scale, p.rect.w * scale, p.rect.h * scale);

            if let Some(bg) = s.bg {
                let mut pb = PathBuilder::new();
                rounded_rect(&mut pb, x, y, w, h, s.corner_radius * scale);
                if let Some(path) = pb.finish() {
                    let mut paint = Paint::default();
                    paint.set_color(skia_color(bg));
                    paint.anti_alias = true;
                    pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
                }
            }

            match n.kind {
                ElementKind::Text => {
                    let fg = s.fg.unwrap_or(Color::BLACK);
                    draw_text(
                        &mut pixmap,
                        font,
                        &n.text,
                        s.font_size * scale,
                        fg,
                        x + s.padding * scale,
                        y + s.padding * scale,
                    );
                }
                ElementKind::Button => {
                    let fg = s.fg.unwrap_or(Color::WHITE);
                    // 按钮文本居中
                    let (tw, th) = measure_text(font, &n.text, s.font_size * scale);
                    draw_text(
                        &mut pixmap,
                        font,
                        &n.text,
                        s.font_size * scale,
                        fg,
                        x + (w - tw) / 2.0,
                        y + (h - th) / 2.0,
                    );
                }
                ElementKind::View => {}
            }
        }
    });

    (pixmap, placed)
}

/// 命中测试(逻辑坐标),返回最上层可点击节点
pub fn hit_click_target(doc: &Doc, placed: &[Placed], x: f32, y: f32) -> Option<ViewId> {
    placed
        .iter()
        .rev()
        .find(|p| p.rect.contains(x, y) && doc.click_handler(p.id).is_some())
        .map(|p| p.id)
}
