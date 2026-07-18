//! 布局 + 绘制(CPU 自绘原型)
//!
//! 布局:行/列堆叠 + CSS 盒模型最小集(四方向 padding/margin、border、
//! 固定宽高覆盖;缺省即 border-box 语义)。TODO 换 taffy。
//! **继承**:`fg=None` / `font_size=NAN` 沿父链解析(color/font-size 白名单,
//! ADR-8 C1),根 fallback BLACK/16。measure 自顶向下携带解析值,
//! paint 对平铺列表做 O(depth) 父链回溯。
//! 绘制走 tiny-skia + swash;逻辑坐标布局、物理坐标绘制(乘 scale)。
//! 文本 shaping 为简化线性排版:charmap 逐字映射 + advance 推进(无 kerning/
//! 连字;能力与原 fontdue 持平,M2 换 Parley/HarfRust)。

use std::collections::HashMap;

use swash::FontRef;
use tiny_skia::Pixmap;

use sv_ui::{Color, Direction, Doc, DocumentInner, ElementKind, ViewId};

use crate::font::ui_font;
use crate::paint::{GlyphKey, GlyphPos, Painter, TinySkiaPainter};

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

/// 行度量:(基线距行顶, 行高)。font.metrics 按 px 缩放(ascent/descent/leading)
fn line_metrics(font: &FontRef, px: f32) -> (f32, f32) {
    let m = font.metrics(&[]).scale(px);
    (m.ascent, m.ascent + m.descent + m.leading)
}

pub fn measure_text(font: &FontRef, text: &str, px: f32) -> (f32, f32) {
    let (_, line_h) = line_metrics(font, px);
    if text.is_empty() {
        return (0.0, line_h);
    }
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    let w: f32 = text.chars().map(|c| gm.advance_width(charmap.map(c))).sum();
    (w, line_h)
}

/// 返回 border-box 尺寸(不含 margin;margin 由父容器计入间距)。
/// `inherited_font`:父链解析到本节点的字号(自身未设时生效)
fn measure(
    inner: &DocumentInner,
    font: &FontRef,
    id: ViewId,
    cache: &mut HashMap<ViewId, (f32, f32)>,
    inherited_font: f32,
) -> (f32, f32) {
    if let Some(sz) = cache.get(&id) {
        return *sz;
    }
    let n = &inner.nodes[id];
    let s = &n.style;
    let fs = if s.font_size.is_nan() {
        inherited_font
    } else {
        s.font_size
    };
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
        ElementKind::TextInput => {
            // 宽不随内容变(业界一致);默认 200 逻辑 px,style.width 覆盖
            let (_, line_h) = line_metrics(font, fs);
            (
                200.0 + s.padding.horizontal() + bw * 2.0,
                line_h + s.padding.vertical() + bw * 2.0,
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
    font: &FontRef,
    cache: &mut HashMap<ViewId, (f32, f32)>,
    id: ViewId,
    x: f32,
    y: f32,
    forced: Option<(f32, f32)>,
    inherited_font: f32,
    out: &mut Vec<Placed>,
) {
    let (w, h) = forced.unwrap_or_else(|| measure(inner, font, id, cache, inherited_font));
    out.push(Placed {
        id,
        rect: Rect { x, y, w, h },
    });
    let n = &inner.nodes[id];
    if n.kind != ElementKind::View {
        return;
    }
    let s = n.style.clone();
    let fs = if s.font_size.is_nan() {
        inherited_font
    } else {
        s.font_size
    };
    let bw = s.border.map(|b| b.width).unwrap_or(0.0);
    let mut cx = x + s.padding.left + bw;
    let mut cy = y + s.padding.top + bw;
    for c in &n.children {
        let (cw, ch) = measure(inner, font, *c, cache, fs);
        let m = inner.nodes[*c].style.margin;
        match s.direction {
            Direction::Row => {
                place(
                    inner,
                    font,
                    cache,
                    *c,
                    cx + m.left,
                    cy + m.top,
                    None,
                    fs,
                    out,
                );
                cx += cw + m.horizontal() + s.gap;
            }
            Direction::Column => {
                place(
                    inner,
                    font,
                    cache,
                    *c,
                    cx + m.left,
                    cy + m.top,
                    None,
                    fs,
                    out,
                );
                cy += ch + m.vertical() + s.gap;
            }
        }
    }
}

/// 版本键控布局缓存:同一 Doc、同版本、同尺寸 → 直接复用上次布局。
/// 静止帧的 O(n) measure/place 归零(细粒度更新模型下,静止是常态)
pub fn layout_tree_cached(doc: &Doc, logical_w: f32, logical_h: f32) -> Vec<Placed> {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Option<(usize, u64, u32, u32, Vec<Placed>)>> =
            const { RefCell::new(None) };
    }
    let key = (
        doc.identity(),
        doc.version(),
        logical_w.to_bits(),
        logical_h.to_bits(),
    );
    CACHE.with(|c| {
        let mut slot = c.borrow_mut();
        if let Some((id, ver, w, h, placed)) = slot.as_ref()
            && (*id, *ver, *w, *h) == key
        {
            return placed.clone();
        }
        let placed = layout_tree(doc, logical_w, logical_h);
        *slot = Some((key.0, key.1, key.2, key.3, placed.clone()));
        placed
    })
}

/// 布局整棵树。root 强制占满窗口逻辑尺寸
pub fn layout_tree(doc: &Doc, logical_w: f32, logical_h: f32) -> Vec<Placed> {
    let font = ui_font();
    doc.read(|inner| {
        let mut cache = HashMap::new();
        let mut out = Vec::new();
        place(
            inner,
            &font,
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

/// shaping:文本 → 已定位字形(物理坐标)。painter 只拿 glyph run。
/// 简化线性排版:charmap 逐字映射 + advance 推进(无 kerning/连字)。
/// `oy` 是文本框顶,基线 = oy + ascent;x/y 与 ox/oy 都是基线原点
/// (CPU 端由光栅 Placement 换算位图左上角,GPU 端直接喂 draw_glyphs)
fn shape_text(font: &FontRef, text: &str, px: f32, ox: f32, oy: f32) -> Vec<GlyphPos> {
    if text.is_empty() {
        return Vec::new();
    }
    let (ascent, _) = line_metrics(font, px);
    let baseline = oy + ascent;
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    let mut pen = ox;
    let mut out = Vec::new();
    for c in text.chars() {
        let id = charmap.map(c);
        let adv = gm.advance_width(id);
        // 空白字符只推进 pen,不产出字形(与原 fontdue 过滤零宽位图语义一致)
        if !c.is_whitespace() {
            out.push(GlyphPos {
                key: GlyphKey::new(id, px),
                x: pen,
                y: baseline,
                id,
                ox: pen,
                oy: baseline,
            });
        }
        pen += adv;
    }
    out
}

/// 光标 x 偏移(逻辑 px,相对文本起点):`byte_idx` 前所有字符的 advance 和。
/// 与 [`shape_text`] 同一 advance 逻辑——保证"画的"和"点的"一致
pub fn caret_x(font: &FontRef, text: &str, px: f32, byte_idx: usize) -> f32 {
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    text[..byte_idx.min(text.len())]
        .chars()
        .map(|c| gm.advance_width(charmap.map(c)))
        .sum()
}

/// 点击 x 坐标(相对文本起点)→ 最近 char 边界的字节偏移(与 caret_x 互逆)
pub fn caret_index_at(font: &FontRef, text: &str, px: f32, x: f32) -> usize {
    let charmap = font.charmap();
    let gm = font.glyph_metrics(&[]).scale(px);
    let mut pen = 0.0f32;
    for (i, c) in text.char_indices() {
        let adv = gm.advance_width(charmap.map(c));
        if x < pen + adv / 2.0 {
            return i;
        }
        pen += adv;
    }
    text.len()
}

/// 共享绘制遍历:对任意 Painter 后端发出同一命令流。
/// 这是"可切换渲染后端"的支点(调研 14):后端只实现 Painter 三个动词
pub fn paint_tree(doc: &Doc, placed: &[Placed], painter: &mut dyn Painter, scale: f32) {
    let font = ui_font();
    doc.read(|inner| {
        for p in placed {
            let Some(n) = inner.nodes.get(p.id) else {
                continue;
            };
            let s = &n.style;
            let op = effective_opacity(inner, p.id);
            let fs = resolve_font_size(inner, p.id) * scale;
            let bw = s.border.map(|b| b.width).unwrap_or(0.0);
            let (x, y, w, h) = (
                p.rect.x * scale,
                p.rect.y * scale,
                p.rect.w * scale,
                p.rect.h * scale,
            );
            let inset = (s.padding.left + bw) * scale;
            let inset_top = (s.padding.top + bw) * scale;

            if let Some(bg) = s.bg {
                painter.fill_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    s.corner_radius * scale,
                    with_opacity(bg, op),
                );
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
                    let run = shape_text(&font, &n.text, fs, x + inset, y + inset_top);
                    painter.glyph_run(&run, fg);
                }
                ElementKind::Button => {
                    let fg = with_opacity(s.fg.unwrap_or(Color::WHITE), op);
                    let (tw, th) = measure_text(&font, &n.text, fs);
                    let run =
                        shape_text(&font, &n.text, fs, x + (w - tw) / 2.0, y + (h - th) / 2.0);
                    painter.glyph_run(&run, fg);
                }
                ElementKind::Checkbox => {
                    let boxc = with_opacity(s.bg.unwrap_or(Color::rgb(221, 221, 234)), op);
                    let r = if s.corner_radius > 0.0 {
                        s.corner_radius
                    } else {
                        4.0
                    };
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
                ElementKind::TextInput => {
                    let Some(input) = n.input.as_deref() else {
                        continue;
                    };
                    let focused = inner.focused == Some(p.id);
                    // 默认底/边(style 设了 bg/border 则上面已统一画过,不重复)
                    let radius = if s.corner_radius > 0.0 {
                        s.corner_radius
                    } else {
                        4.0
                    };
                    if s.bg.is_none() {
                        painter.fill_rounded_rect(
                            x,
                            y,
                            w,
                            h,
                            radius * scale,
                            with_opacity(Color::rgb(248, 248, 252), op),
                        );
                    }
                    if s.border.is_none() {
                        painter.stroke_rounded_rect(
                            x,
                            y,
                            w,
                            h,
                            radius * scale,
                            1.0 * scale,
                            with_opacity(Color::rgb(200, 200, 212), op),
                        );
                    }

                    let content_x = x + inset;
                    let content_y = y + inset_top;
                    let content_w = w - (s.padding.horizontal() + bw * 2.0) * scale;
                    let content_h = h - (s.padding.vertical() + bw * 2.0) * scale;

                    // 显示串 = value[..cursor] + 预编辑 + value[cursor..]
                    // (仅绘制层拼接,ViewNode.text 不含半成品组合文本)
                    let value = &n.text;
                    let (display, caret_byte, preedit_range) =
                        sv_ui::input::display_text(value, input);

                    // 光标跟随:每帧无状态计算横向滚移(fs 已含 scale,均物理 px)
                    let caret_px = caret_x(&font, &display, fs, caret_byte);
                    let scroll = (caret_px - (content_w - 2.0 * scale)).max(0.0);
                    let text_x = content_x - scroll;

                    painter.push_clip(content_x - scale, y, content_w + 2.0 * scale, h);

                    // 选区高亮(组合中隐藏选区,IME 惯例)
                    if focused && input.preedit.is_none() && input.cursor != input.anchor {
                        let lo = caret_x(&font, value, fs, input.cursor.min(input.anchor));
                        let hi = caret_x(&font, value, fs, input.cursor.max(input.anchor));
                        painter.fill_rounded_rect(
                            text_x + lo,
                            content_y,
                            hi - lo,
                            content_h,
                            0.0,
                            with_opacity(Color::rgba(60, 120, 255, 80), op),
                        );
                    }

                    // 文本 / placeholder
                    if display.is_empty() {
                        if !input.placeholder.is_empty() {
                            let run = shape_text(&font, &input.placeholder, fs, text_x, content_y);
                            painter.glyph_run(&run, with_opacity(Color::rgb(152, 152, 166), op));
                        }
                    } else {
                        let fg = with_opacity(resolve_fg(inner, p.id), op);
                        let run = shape_text(&font, &display, fs, text_x, content_y);
                        painter.glyph_run(&run, fg);
                    }

                    // 预编辑整段 2px 下划线(over-the-spot,候选窗是输入法自己的)
                    if let Some((lo, hi)) = preedit_range {
                        let x0 = caret_x(&font, &display, fs, lo);
                        let x1 = caret_x(&font, &display, fs, hi);
                        painter.fill_rounded_rect(
                            text_x + x0,
                            content_y + content_h - 2.0 * scale,
                            x1 - x0,
                            2.0 * scale,
                            0.0,
                            with_opacity(resolve_fg(inner, p.id), op),
                        );
                    }

                    // 光标竖线(仅焦点时)
                    if focused {
                        painter.fill_rounded_rect(
                            text_x + caret_px,
                            content_y,
                            (1.5 * scale).max(1.0),
                            content_h,
                            0.0,
                            with_opacity(Color::rgb(255, 62, 0), op),
                        );
                    }

                    painter.pop_clip();
                }
                ElementKind::View => {}
            }
        }

        // 默认焦点环(调研 20:stroke 外扩 2px,宽 2px,accent 定色;
        // 画在所有节点之后 = 永远在最上层;Painter 零新动词)
        if let Some(fid) = inner.focused
            && let Some(p) = placed.iter().find(|p| p.id == fid)
        {
            let m = 2.0 * scale;
            let radius = inner
                .nodes
                .get(fid)
                .map(|n| n.style.corner_radius)
                .unwrap_or(0.0);
            painter.stroke_rounded_rect(
                p.rect.x * scale - m,
                p.rect.y * scale - m,
                p.rect.w * scale + m * 2.0,
                p.rect.h * scale + m * 2.0,
                (radius + 2.0) * scale,
                2.0 * scale,
                Color::rgb(255, 62, 0),
            );
        }
    });
}

/// 渲染一帧:布局(逻辑坐标)+ 绘制(物理坐标)。返回像素与命中测试用的布局
pub fn render_frame(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> (Pixmap, Vec<Placed>) {
    let logical_w = phys_w as f32 / scale;
    let logical_h = phys_h as f32 / scale;
    let placed = layout_tree_cached(doc, logical_w, logical_h);

    let mut pixmap = Pixmap::new(phys_w.max(1), phys_h.max(1)).expect("sv-shell: 创建 pixmap 失败");
    pixmap.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 255));
    let mut painter = TinySkiaPainter::new(&mut pixmap);
    paint_tree(doc, &placed, &mut painter, scale);

    (pixmap, placed)
}

/// 点击命中 TextInput 时:窗口逻辑 x → 值内字节偏移(含 padding/border 内缩)。
/// v0 忽略溢出滚移(光标跟随滚动是绘制层每帧无状态计算,点击场景多为未溢出)
pub fn input_caret_at(doc: &Doc, p: &Placed, lx: f32) -> usize {
    let font = ui_font();
    doc.read(|inner| {
        let Some(n) = inner.nodes.get(p.id) else {
            return 0;
        };
        let fs = resolve_font_size(inner, p.id);
        let bw = n.style.border.map(|b| b.width).unwrap_or(0.0);
        let text_x = p.rect.x + n.style.padding.left + bw;
        caret_index_at(&font, &n.text, fs, lx - text_x)
    })
}

/// 焦点输入框的光标矩形(物理 px;IME 候选窗定位用)。
/// 与绘制层同一 display/caret/scroll 计算——"画的"与"报的"一致
pub fn ime_caret_rect(doc: &Doc, placed: &[Placed], scale: f32) -> Option<(f32, f32, f32, f32)> {
    let font = ui_font();
    doc.read(|inner| {
        let id = inner.focused?;
        let n = inner.nodes.get(id)?;
        let input = n.input.as_deref()?;
        let p = placed.iter().find(|p| p.id == id)?;
        let fs = resolve_font_size(inner, id) * scale;
        let bw = n.style.border.map(|b| b.width).unwrap_or(0.0);
        let s = &n.style;
        let (x, y, w, h) = (
            p.rect.x * scale,
            p.rect.y * scale,
            p.rect.w * scale,
            p.rect.h * scale,
        );
        let content_x = x + (s.padding.left + bw) * scale;
        let content_y = y + (s.padding.top + bw) * scale;
        let content_w = w - (s.padding.horizontal() + bw * 2.0) * scale;
        let content_h = h - (s.padding.vertical() + bw * 2.0) * scale;
        let (display, caret_byte, _) = sv_ui::input::display_text(&n.text, input);
        let caret_px = caret_x(&font, &display, fs, caret_byte);
        let scroll = (caret_px - (content_w - 2.0 * scale)).max(0.0);
        Some((
            content_x - scroll + caret_px,
            content_y,
            (1.5 * scale).max(1.0),
            content_h,
        ))
    })
}

/// 命中测试(逻辑坐标),返回最上层可点击节点
pub fn hit_click_target(doc: &Doc, placed: &[Placed], x: f32, y: f32) -> Option<ViewId> {
    placed
        .iter()
        .rev()
        .find(|p| p.rect.contains(x, y) && doc.click_handler(p.id).is_some())
        .map(|p| p.id)
}
