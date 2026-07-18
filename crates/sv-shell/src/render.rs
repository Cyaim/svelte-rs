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
use tiny_skia::Pixmap;

use sv_ui::{Color, Direction, Doc, DocumentInner, ElementKind, ViewId};

use crate::font::ui_font;
use crate::paint::{GlyphPos, Painter, TinySkiaPainter};

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

/// 把节点的不透明度乘进颜色 alpha
fn with_opacity(c: Color, o: f32) -> Color {
    Color::rgba(c.r, c.g, c.b, (c.a as f32 * o.clamp(0.0, 1.0)) as u8)
}

/// 有效不透明度 = 自身 × 祖先链乘积(近似组透明,v0 无合成层;
/// 换 vello 后由 push_layer/pop_layer 天然正确)
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

/// shaping:文本 → 已定位字形(物理坐标)。painter 只拿 glyph run
fn shape_text(font: &Font, text: &str, px: f32, ox: f32, oy: f32) -> Vec<GlyphPos> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.append(&[font], &TextStyle::new(text, px, 0));
    layout
        .glyphs()
        .iter()
        .filter(|g| g.width > 0)
        .map(|g| {
            // 反推基线原点:fontdue 给位图左上角,GPU 后端要 pen origin
            // (bitmap_left = pen_x + xmin;bitmap_top = baseline - height - ymin)
            let m = font.metrics_indexed(g.key.glyph_index, px);
            GlyphPos {
                key: g.key,
                x: ox + g.x,
                y: oy + g.y,
                id: g.key.glyph_index,
                ox: ox + g.x - m.xmin as f32,
                oy: oy + g.y + m.height as f32 + m.ymin as f32,
            }
        })
        .collect()
}

/// 共享绘制遍历:对任意 Painter 后端发出同一命令流。
/// 这是"可切换渲染后端"的支点(调研 14):后端只实现 Painter 三个动词
pub fn paint_tree(doc: &Doc, placed: &[Placed], painter: &mut dyn Painter, scale: f32) {
    let font = ui_font();
    doc.read(|inner| {
        for p in placed {
            let Some(n) = inner.nodes.get(p.id) else { continue };
            let s = &n.style;
            let op = effective_opacity(inner, p.id);
            let fs = resolve_font_size(inner, p.id) * scale;
            let bw = s.border.map(|b| b.width).unwrap_or(0.0);
            let (x, y, w, h) = (p.rect.x * scale, p.rect.y * scale, p.rect.w * scale, p.rect.h * scale);
            let inset = (s.padding.left + bw) * scale;
            let inset_top = (s.padding.top + bw) * scale;

            if let Some(bg) = s.bg {
                painter.fill_rounded_rect(x, y, w, h, s.corner_radius * scale, with_opacity(bg, op));
            }
            if let Some(b) = s.border {
                painter.stroke_rounded_rect(
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
                    let run = shape_text(font, &n.text, fs, x + inset, y + inset_top);
                    painter.glyph_run(font, &run, fg);
                }
                ElementKind::Button => {
                    let fg = with_opacity(s.fg.unwrap_or(Color::WHITE), op);
                    let (tw, th) = measure_text(font, &n.text, fs);
                    let run = shape_text(font, &n.text, fs, x + (w - tw) / 2.0, y + (h - th) / 2.0);
                    painter.glyph_run(font, &run, fg);
                }
                ElementKind::Checkbox => {
                    let boxc = with_opacity(s.bg.unwrap_or(Color::rgb(221, 221, 234)), op);
                    let r = if s.corner_radius > 0.0 { s.corner_radius } else { 4.0 };
                    painter.fill_rounded_rect(x, y, w, h, r * scale, boxc);
                    if n.checked {
                        let accent = with_opacity(s.fg.unwrap_or(Color::rgb(255, 62, 0)), op);
                        let ins = w * 0.25;
                        painter.fill_rounded_rect(
                            x + ins,
                            y + ins,
                            w - ins * 2.0,
                            h - ins * 2.0,
                            2.0 * scale,
                            accent,
                        );
                    }
                }
                ElementKind::View => {}
            }
        }
    });
}

/// 渲染一帧:布局(逻辑坐标)+ 绘制(物理坐标)。返回像素与命中测试用的布局
pub fn render_frame(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> (Pixmap, Vec<Placed>) {
    let logical_w = phys_w as f32 / scale;
    let logical_h = phys_h as f32 / scale;
    let placed = layout_tree(doc, logical_w, logical_h);

    let mut pixmap = Pixmap::new(phys_w.max(1), phys_h.max(1)).expect("sv-shell: 创建 pixmap 失败");
    pixmap.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    let mut painter = TinySkiaPainter { pixmap: &mut pixmap };
    paint_tree(doc, &placed, &mut painter, scale);

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
