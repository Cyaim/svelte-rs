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
//! - TextInput 仍走旧 swash 线性路径(key=0):编辑几何(caret_x)与显示必须
//!   同源,P3 由 PlainEditor 整体外包后一并切换。

use std::cell::RefCell;
use std::collections::HashMap;

use parley::{AlignmentOptions, FontContext, LayoutContext, PositionedLayoutItem, StyleProperty};
use swash::FontRef;

use crate::font::FontHandle;
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

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine {
        fcx: FontContext::new(),
        lcx: LayoutContext::new(),
    });
    static FONTS: RefCell<HashMap<u64, RegisteredFont>> = RefCell::new(HashMap::new());
}

/// 注册(或取回)一个 parley 字体的身份句柄
fn register(font: &parley::FontData) -> FontHandle {
    // Blob id 进程内唯一但**从 0 起**——必须避开内置字体的保留键 0
    // (撞键 = Segoe 的字形 id 被雅黑光栅,Latin 全员错字;实测踩过):
    // 最高位恒置 1,id 左移 16 容纳 TTC index,三段无碰撞
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

/// 按身份键取 swash FontRef(CPU 光栅;key=0 的内置字体走 font.rs)
pub(crate) fn font_ref_of(key: u64) -> Option<FontRef<'static>> {
    FONTS.with(|f| f.borrow().get(&key).map(|r| r.fref))
}

/// 按身份键取原始字节 + index(GPU 端建 FontData)
pub(crate) fn font_bytes_of(key: u64) -> Option<(&'static [u8], u32)> {
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
        let m = crate::font::ui_font().metrics(&[]).scale(px);
        return (0.0, m.ascent + m.descent + m.leading);
    }
    thread_local! {
        static HOT: RefCell<HashMap<(u64, u32, u32), (f32, f32)>> = RefCell::new(HashMap::new());
        static COLD: RefCell<HashMap<(u64, u32, u32), (f32, f32)>> = RefCell::new(HashMap::new());
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
