//! 布局 + 绘制(CPU 自绘原型)
//!
//! 布局:行/列堆叠 + CSS 盒模型最小集(四方向 padding/margin、border、
//! 固定宽高覆盖;缺省即 border-box 语义)。TODO 换 taffy。
//! **继承**:`fg=None` / `font_size=NAN` 沿父链解析(color/font-size 白名单,
//! ADR-8 C1),根 fallback BLACK/16。measure 自顶向下携带解析值,
//! paint 对平铺列表做 O(depth) 父链回溯。
//! 绘制走 tiny-skia + fontdue;逻辑坐标布局、物理坐标绘制(乘 scale)。

use std::collections::HashMap;

use fontdue::Font;
use fontdue::layout::{CoordinateSystem, Layout, TextStyle};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

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

/// 一次布局的产物:绘制顺序排列(父先子后),rect 为逻辑坐标 border-box
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub id: ViewId,
    pub rect: Rect,
}

const ROOT_FONT_SIZE: f32 = 16.0;

/// 继承解析:自身 NAN → 父链向上,根 fallback
fn resolve_font_size(inner: &DocumentInner, id: ViewId) -> f32 {
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(n) = inner.nodes.get(c) else { break };
        if !n.style.font_size.is_nan() {
            return n.style.font_size;
        }
        cur = n.parent;
    }
    ROOT_FONT_SIZE
}

fn resolve_fg(inner: &DocumentInner, id: ViewId) -> Color {
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(n) = inner.nodes.get(c) else { break };
        if let Some(fg) = n.style.fg {
            return fg;
        }
        cur = n.parent;
    }
    Color::BLACK
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

/// 返回 border-box 尺寸(不含 margin;margin 由父容器计入间距)。
/// `inherited_font`:父链解析到本节点的字号(自身未设时生效)
fn measure(
    inner: &DocumentInner,
    font: &Font,
    id: ViewId,
    cache: &mut HashMap<ViewId, (f32, f32)>,
    inherited_font: f32,
) -> (f32, f32) {
    if let Some(sz) = cache.get(&id) {
        return *sz;
    }
    let n = &inner.nodes[id];
    let s = &n.style;
    let fs = if s.font_size.is_nan() { inherited_font } else { s.font_size };
    let bw = s.border.map(|b| b.width).unwrap_or(0.0);
    let (mut w, mut h) = match n.kind {
        ElementKind::Text | ElementKind::Button => {
            let (tw, th) = measure_text(font, &n.text, fs);
            (
                tw + s.padding.horizontal() + bw * 2.0,
                th + s.padding.vertical() + bw * 2.0,
            )
        }
        ElementKind::Checkbox => {
            let side = fs.max(14.0);
            (
                side + s.padding.horizontal() + bw * 2.0,
                side + s.padding.vertical() + bw * 2.0,
            )
        }
        ElementKind::View => {
            let mut main = 0.0f32;
            let mut cross = 0.0f32;
            let mut count = 0usize;
            for c in &n.children {
                let (cw, ch) = measure(inner, font, *c, cache, fs);
                let m = inner.nodes[*c].style.margin;
                match s.direction {
                    Direction::Row => {
                        main += cw + m.horizontal();
                        cross = cross.max(ch + m.vertical());
                    }
                    Direction::Column => {
                        main += ch + m.vertical();
                        cross = cross.max(cw + m.horizontal());
                    }
                }
                count += 1;
            }
            if count > 1 {
                main += s.gap * (count as f32 - 1.0);
            }
            match s.direction {
                Direction::Row => (
                    main + s.padding.horizontal() + bw * 2.0,
                    cross + s.padding.vertical() + bw * 2.0,
                ),
                Direction::Column => (
                    cross + s.padding.horizontal() + bw * 2.0,
                    main + s.padding.vertical() + bw * 2.0,
                ),
            }
        }
    };
    // 显式宽高 = border-box 覆盖(桌面直觉,即 CSS box-sizing: border-box)
    if let Some(fw) = s.width {
        w = fw;
    }
    if let Some(fh) = s.height {
        h = fh;
    }
    cache.insert(id, (w, h));
    (w, h)
}

#[allow(clippy::too_many_arguments)]
fn place(
    inner: &DocumentInner,
    font: &Font,
    cache: &mut HashMap<ViewId, (f32, f32)>,
    id: ViewId,
    x: f32,
    y: f32,
    forced: Option<(f32, f32)>,
    inherited_font: f32,
    out: &mut Vec<Placed>,
) {
    let (w, h) = forced.unwrap_or_else(|| measure(inner, font, id, cache, inherited_font));
    out.push(Placed { id, rect: Rect { x, y, w, h } });
    let n = &inner.nodes[id];
    if n.kind != ElementKind::View {
        return;
    }
    let s = n.style.clone();
    let fs = if s.font_size.is_nan() { inherited_font } else { s.font_size };
    let bw = s.border.map(|b| b.width).unwrap_or(0.0);
    let mut cx = x + s.padding.left + bw;
    let mut cy = y + s.padding.top + bw;
    for c in &n.children {
        let (cw, ch) = measure(inner, font, *c, cache, fs);
        let m = inner.nodes[*c].style.margin;
        match s.direction {
            Direction::Row => {
                place(inner, font, cache, *c, cx + m.left, cy + m.top, None, fs, out);
                cx += cw + m.horizontal() + s.gap;
            }
            Direction::Column => {
                place(inner, font, cache, *c, cx + m.left, cy + m.top, None, fs, out);
                cy += ch + m.vertical() + s.gap;
            }
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
            ROOT_FONT_SIZE,
            &mut out,
        );
        out
    })
}

fn skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// 把节点的不透明度乘进颜色 alpha
fn with_opacity(c: Color, o: f32) -> Color {
    Color::rgba(c.r, c.g, c.b, (c.a as f32 * o.clamp(0.0, 1.0)) as u8)
}

/// 有效不透明度 = 自身 × 祖先链乘积(近似组透明,v0 无合成层)
fn effective_opacity(inner: &DocumentInner, id: ViewId) -> f32 {
    let mut o = 1.0f32;
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(n) = inner.nodes.get(c) else { break };
        o *= n.style.opacity;
        cur = n.parent;
    }
    o
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

fn fill_rounded(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
    let mut pb = PathBuilder::new();
    rounded_rect(&mut pb, x, y, w, h, r);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(skia_color(c));
        paint.anti_alias = true;
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn stroke_rounded(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Color) {
    // 沿边框中心线描边(内缩半宽),视觉上贴合 border-box
    let half = width / 2.0;
    let mut pb = PathBuilder::new();
    rounded_rect(&mut pb, x + half, y + half, w - width, h - width, (r - half).max(0.0));
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(skia_color(c));
        paint.anti_alias = true;
        let stroke = Stroke { width, ..Stroke::default() };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
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
            let op = effective_opacity(inner, p.id);
            let fs = resolve_font_size(inner, p.id) * scale;
            let bw = s.border.map(|b| b.width).unwrap_or(0.0);
            let (x, y, w, h) = (p.rect.x * scale, p.rect.y * scale, p.rect.w * scale, p.rect.h * scale);
            let inset = (s.padding.left + bw) * scale;
            let inset_top = (s.padding.top + bw) * scale;

            if let Some(bg) = s.bg {
                fill_rounded(&mut pixmap, x, y, w, h, s.corner_radius * scale, with_opacity(bg, op));
            }
            if let Some(b) = s.border {
                stroke_rounded(
                    &mut pixmap,
                    x,
                    y,
                    w,
                    h,
                    s.corner_radius * scale,
                    b.width * scale,
                    with_opacity(b.color, op),
                );
            }

            match n.kind {
                ElementKind::Text => {
                    let fg = with_opacity(resolve_fg(inner, p.id), op);
                    draw_text(&mut pixmap, font, &n.text, fs, fg, x + inset, y + inset_top);
                }
                ElementKind::Button => {
                    // 按钮文本默认白(未显式设置且不走继承——按钮底色语境)
                    let fg = with_opacity(s.fg.unwrap_or(Color::WHITE), op);
                    let (tw, th) = measure_text(font, &n.text, fs);
                    draw_text(&mut pixmap, font, &n.text, fs, fg, x + (w - tw) / 2.0, y + (h - th) / 2.0);
                }
                ElementKind::Checkbox => {
                    let boxc = with_opacity(s.bg.unwrap_or(Color::rgb(221, 221, 234)), op);
                    let r = if s.corner_radius > 0.0 { s.corner_radius } else { 4.0 };
                    fill_rounded(&mut pixmap, x, y, w, h, r * scale, boxc);
                    if n.checked {
                        let accent = with_opacity(s.fg.unwrap_or(Color::rgb(255, 62, 0)), op);
                        let ins = w * 0.25;
                        fill_rounded(&mut pixmap, x + ins, y + ins, w - ins * 2.0, h - ins * 2.0, 2.0 * scale, accent);
                    }
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
