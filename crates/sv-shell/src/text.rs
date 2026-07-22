//! TextEngine 门面(调研 24 P1):fontique 字体发现 + Parley 排版接管 shaping。
//!
//! - **门面即防波堤**:parley 0.x 一年 5 个 breaking minor(0.8 坐标系翻转),
//!   全仓只有本文件 import parley;锁 minor 0.11,季度集中升级。
//! - 排版在**逻辑 px** 做(quantize=false 保连续),字形坐标乘 scale 出物理
//!   ——画/量同源,HiDPI 下不会各断各的(与旧折行门面同一纪律)。
//! - fallback:fontique 按 script 选字体,同帧多字体 run 由 P0 载体
//!   (FontHandle/GlyphKey.font_key)承载;字体注册表按 Blob id 建键,
//!   每个唯一字体**有意泄漏一次** Blob 句柄(Arc 壳,零拷贝)换取
//!   `FontRef<'static>` 的稳定 CacheKey(swash ScaleContext 缓存命中前提)。
//! - **P3(2026-07-22)**:TextInput 的光标/选区/命中几何也搬到本门面
//!   ([`caret_x`] / [`caret_index_at`] / [`selection_rects`]),旧 swash 线性
//!   路径(逐 char advance 求和)与 `font.rs` 一并退役——全仓再无第二套排版。

use std::cell::RefCell;
use std::collections::HashMap;

use parley::{
    Affinity, AlignmentOptions, Cursor, FontContext, LayoutContext, PositionedLayoutItem,
    Selection, StyleProperty,
};
use swash::FontRef;

use crate::paint::{GlyphKey, GlyphPos};

struct Engine {
    fcx: FontContext,
    lcx: LayoutContext<[u8; 4]>,
}

struct RegisteredFont {
    fref: FontRef<'static>,
    bytes: &'static [u8],
    index: u32,
}

/// 字体身份句柄(调研 24 P0"载体扩宽"):glyph run 带字体身份,
/// 光栅缓存/GPU FontData 都按 `key` 索引。键由 [`register`] 按 fontique
/// Blob id 生成,同帧多字体 fallback 混排即多个 key
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FontHandle {
    pub key: u64,
}

impl FontHandle {
    /// 按 key 解析 swash FontRef(CPU 光栅/度量用)
    pub fn font_ref(&self) -> FontRef<'static> {
        font_ref_of(self.key).expect("sv-shell: 未注册的字体键")
    }

    /// 原始字节 + collection index(GPU 端构造 peniko::FontData 用)
    #[cfg_attr(not(feature = "backend-vello"), allow(dead_code))]
    pub fn data(&self) -> (&'static [u8], u32) {
        font_bytes_of(self.key).expect("sv-shell: 未注册的字体键")
    }
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine {
        fcx: FontContext::new(),
        lcx: LayoutContext::new(),
    });
    static FONTS: RefCell<HashMap<u64, RegisteredFont>> = RefCell::new(HashMap::new());
}

/// 注册(或取回)一个 parley 字体的身份句柄
fn register(font: &parley::FontData) -> FontHandle {
    // Blob id 进程内唯一但**从 0 起**:最高位恒置 1(P1 期为避开内置字体的
    // 保留键 0——撞键 = Segoe 的字形 id 被雅黑光栅,Latin 全员错字,实测踩过;
    // P3 内置字体退役后保留此编码,键值形态不变),id 左移 16 容纳 TTC index
    let key = (1u64 << 63) | (font.data.id() << 16) | (font.index as u64 & 0xFFFF);
    FONTS.with(|f| {
        f.borrow_mut().entry(key).or_insert_with(|| {
            // 有意泄漏 Blob 壳(Arc 克隆,零拷贝):FontRef<'static> 的
            // CacheKey 必须稳定,ScaleContext 光栅缓存才能命中
            let blob: &'static parley::fontique::Blob<u8> = Box::leak(Box::new(font.data.clone()));
            let bytes: &'static [u8] = blob.as_ref();
            let fref = FontRef::from_index(bytes, font.index as usize)
                .expect("sv-shell: fontique 给出的字体应能被 swash 解析");
            RegisteredFont {
                fref,
                bytes,
                index: font.index,
            }
        });
    });
    FontHandle { key }
}

/// 按身份键取 swash FontRef(CPU 光栅)
fn font_ref_of(key: u64) -> Option<FontRef<'static>> {
    FONTS.with(|f| f.borrow().get(&key).map(|r| r.fref))
}

/// 按身份键取原始字节 + index(GPU 端建 FontData)
fn font_bytes_of(key: u64) -> Option<(&'static [u8], u32)> {
    FONTS.with(|f| f.borrow().get(&key).map(|r| (r.bytes, r.index)))
}

/// 一段 shaping 产物:同字体连续字形
pub struct ShapedRun {
    pub font: FontHandle,
    pub glyphs: Vec<GlyphPos>,
}

/// 默认 locale:zh-Hans(解析一次缓存;解析失败回退 None = 系统默认)
fn zh_hans() -> Option<parley::Language> {
    thread_local! {
        static LOCALE: std::cell::OnceCell<Option<parley::Language>> =
            const { std::cell::OnceCell::new() };
    }
    LOCALE.with(|l| *l.get_or_init(|| "zh-Hans".parse().ok()))
}

fn with_layout<R>(
    text: &str,
    px: f32,
    wrap_w: Option<f32>,
    align: sv_ui::TextAlign,
    f: impl FnOnce(&parley::Layout<[u8; 4]>) -> R,
) -> R {
    ENGINE.with(|e| {
        let e = &mut *e.borrow_mut();
        let mut b = e.lcx.ranged_builder(&mut e.fcx, text, 1.0, false);
        b.push_default(StyleProperty::FontSize(px));
        // 超长不可断段(长 URL)按 CSS overflow-wrap: anywhere 应急强断,
        // 与旧折行门面语义一致(不撑破容器)
        b.push_default(StyleProperty::OverflowWrap(parley::OverflowWrap::Anywhere));
        // locale 注入(调研 24 §3.1):Han 统一汉字按 zh-Hans 消歧
        // (否则可能选中日式字形/触发 ja 分词模型缺失告警)
        b.push_default(StyleProperty::Locale(zh_hans()));
        let mut layout = b.build(text);
        layout.break_all_lines(wrap_w);
        let alignment = match align {
            sv_ui::TextAlign::Left => parley::Alignment::Left,
            sv_ui::TextAlign::Center => parley::Alignment::Center,
            sv_ui::TextAlign::Right => parley::Alignment::Right,
        };
        layout.align(alignment, AlignmentOptions::default());
        f(&layout)
    })
}

/// 度量(逻辑 px):`wrap_w=None` 单行固有宽。空串保持一行高(旧语义)。
/// 两代淘汰缓存:taffy measure fn 每叶多趟询问,parley 布局不便宜
/// (与 glyph_cache 同款分代策略)
pub fn measure(text: &str, px: f32, wrap_w: Option<f32>) -> (f32, f32) {
    if text.is_empty() {
        // 空串保持一行高:借代表字形的行度量。P3 前这里取内置 swash 字体的
        // metrics,与 parley 实际选中的族可能不是同一个字体(空/非空节点行高
        // 会差几个 px);现在同源了
        return (0.0, line_height(px));
    }
    /// 键 =(文本哈希, 字号位, 折行宽位)→ 值 =(宽, 高)
    type MeasureCache = HashMap<(u64, u32, u32), (f32, f32)>;
    thread_local! {
        static HOT: RefCell<MeasureCache> = RefCell::new(HashMap::new());
        static COLD: RefCell<MeasureCache> = RefCell::new(HashMap::new());
    }
    const CAP: usize = 4096;
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        (
            h.finish(),
            px.to_bits(),
            wrap_w.map_or(u32::MAX, f32::to_bits),
        )
    };
    if let Some(hit) = HOT.with(|c| c.borrow().get(&key).copied()) {
        return hit;
    }
    if let Some(hit) = COLD.with(|c| c.borrow_mut().remove(&key)) {
        HOT.with(|c| c.borrow_mut().insert(key, hit));
        return hit;
    }
    let result = with_layout(text, px, wrap_w, sv_ui::TextAlign::Left, |l| {
        (l.width(), l.height())
    });
    HOT.with(|c| {
        let mut hot = c.borrow_mut();
        if hot.len() >= CAP {
            let demoted = std::mem::take(&mut *hot);
            COLD.with(|cold| *cold.borrow_mut() = demoted);
        }
        hot.insert(key, result);
    });
    result
}

/// 单行行高(逻辑 px):空文本节点与 TextInput 的固有高。
/// 取代表字形 `x` 的行度量——与非空文本同一字体系统,不再各断各的
pub fn line_height(px: f32) -> f32 {
    measure("x", px, None).1
}

// ---------------------------------------------------------------------------
// 光标 / 选区几何(调研 24 P3)
//
// **裁决(修订调研 24 §3.3 的 PlainEditor 编辑器池方案)**:parley 只出几何,
// 不接管编辑内核。sv-ui 的 `InputState`(字节光标/锚点/预编辑)仍是唯一编辑
// 真源,于是既不需要 `HashMap<ViewId, PlainEditor>` 第二真源,也不需要
// `Generation` 回声抑制;`bind:value`/IME/剪贴板全链路一行不改。换来的是
// parley `Cursor`/`Selection` 的全部好处:kerning/连字后的真实光标位置、
// fallback 混排下的命中、BiDi 选区多矩形——旧线性 advance 求和全部作废。
//
// 全部取 `wrap_w=None` + 左对齐的单行 Layout,与绘制层 [`shape`] 同参构建
// ——"画的"与"点的"必须出自同一次排版。
// ---------------------------------------------------------------------------

fn with_line_layout<R>(text: &str, px: f32, f: impl FnOnce(&parley::Layout<[u8; 4]>) -> R) -> R {
    with_layout(text, px, None, sv_ui::TextAlign::Left, f)
}

/// 光标 x(逻辑 px,相对文本起点);`byte` 非 char 边界时向下取所在簇起点
pub fn caret_x(text: &str, px: f32, byte: usize) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    with_line_layout(text, px, |l| {
        Cursor::from_byte_index(l, byte.min(text.len()), Affinity::Downstream)
            .geometry(l, 0.0)
            .x0 as f32
    })
}

/// 点击 x(逻辑 px,相对文本起点)→ 最近簇边界的字节偏移(与 [`caret_x`] 互逆)
pub fn caret_index_at(text: &str, px: f32, x: f32) -> usize {
    if text.is_empty() {
        return 0;
    }
    with_line_layout(text, px, |l| Cursor::from_point(l, x, 0.0).index())
}

/// 选区矩形(逻辑 px,相对文本起点):`(x, y, w, h)` 序列。
/// 单行下通常一个矩形,BiDi 混排会分段——所以返回的是序列而不是一对 x
pub fn selection_rects(text: &str, px: f32, lo: usize, hi: usize) -> Vec<(f32, f32, f32, f32)> {
    if text.is_empty() || lo >= hi {
        return Vec::new();
    }
    with_line_layout(text, px, |l| {
        let sel = Selection::new(
            Cursor::from_byte_index(l, lo.min(text.len()), Affinity::Downstream),
            Cursor::from_byte_index(l, hi.min(text.len()), Affinity::Downstream),
        );
        sel.geometry(l)
            .into_iter()
            .map(|(r, _line)| {
                (
                    r.x0 as f32,
                    r.y0 as f32,
                    (r.x1 - r.x0) as f32,
                    (r.y1 - r.y0) as f32,
                )
            })
            .collect()
    })
}

/// 行字节区间(测试探针:断行行为的最小观察面)
#[cfg(test)]
pub fn line_ranges(text: &str, px: f32, wrap_w: Option<f32>) -> Vec<std::ops::Range<usize>> {
    with_layout(text, px, wrap_w, sv_ui::TextAlign::Left, |l| {
        l.lines().map(|line| line.text_range()).collect()
    })
}

/// shaping:逻辑 px 排版(含 fallback 多字体 run 与对齐),
/// 字形坐标乘 `scale` 平移 `(ox, oy)`(物理 px)后产出
pub fn shape(
    text: &str,
    px: f32,
    wrap_w: Option<f32>,
    align: sv_ui::TextAlign,
    ox: f32,
    oy: f32,
    scale: f32,
) -> Vec<ShapedRun> {
    if text.is_empty() {
        return Vec::new();
    }
    with_layout(text, px, wrap_w, align, |layout| {
        let mut out: Vec<ShapedRun> = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(grun) = item else {
                    continue;
                };
                let font = register(grun.run().font());
                let px_phys = grun.run().font_size() * scale;
                let glyphs: Vec<GlyphPos> = grun
                    .positioned_glyphs()
                    .map(|g| {
                        let (x, y) = (ox + g.x * scale, oy + g.y * scale);
                        GlyphPos {
                            key: GlyphKey::new(font, g.id as u16, px_phys),
                            x,
                            y,
                            id: g.id as u16,
                            ox: x,
                            oy: y,
                        }
                    })
                    .collect();
                if glyphs.is_empty() {
                    continue;
                }
                // 相邻同字体 run 合并(减少 Painter 调用)
                if let Some(last) = out.last_mut()
                    && last.font == font
                {
                    last.glyphs.extend(glyphs);
                } else {
                    out.push(ShapedRun { font, glyphs });
                }
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn shaped_ids_match_raster_font_charmap() {
        // shaping 出的 glyph id 必须与光栅所用 FontRef 的 charmap 一致
        for run in super::shape("R2sv", 16.0, None, sv_ui::TextAlign::Left, 0.0, 0.0, 1.0) {
            let fref = super::font_ref_of(run.font.key).unwrap();
            let charmap = fref.charmap();
            let expect: Vec<u16> = "R2sv".chars().map(|c| charmap.map(c)).collect();
            let got: Vec<u16> = run.glyphs.iter().map(|g| g.id).collect();
            eprintln!("[probe] expect(charmap)={expect:?} got(shaped)={got:?}");
            assert_eq!(got, expect, "shaping 字形 id 与光栅字体 charmap 不一致");
        }
    }
}
