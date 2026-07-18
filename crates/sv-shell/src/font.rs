//! 系统字体加载(原型:各平台按路径探测,优先 CJK 字体)
//!
//! TODO:换成 Parley/fontique 之后由字体系统接管 fallback 链与字形缺失。

use std::sync::{Arc, OnceLock};

use fontdue::{Font, FontSettings};

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\msyh.ttc",    // 微软雅黑(CJK)
    "C:\\Windows\\Fonts\\segoeui.ttf", // Segoe UI
    "C:\\Windows\\Fonts\\arial.ttf",
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/PingFang.ttc",
    "/System/Library/Fonts/Helvetica.ttc",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
];

#[cfg(all(unix, not(target_os = "macos")))]
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];

static FONT_DATA: OnceLock<(Arc<Vec<u8>>, u32)> = OnceLock::new();
static FONT: OnceLock<Font> = OnceLock::new();

/// 原始字体字节 + collection index(fontdue 与 vello/peniko 共用同一份数据)
fn font_data() -> &'static (Arc<Vec<u8>>, u32) {
    FONT_DATA.get_or_init(|| {
        for path in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                // 校验 fontdue 可解析,才认定该候选可用(保持逐候选回退语义)
                let settings = FontSettings { collection_index: 0, ..FontSettings::default() };
                if Font::from_bytes(bytes.as_slice(), settings).is_ok() {
                    return (Arc::new(bytes), 0);
                }
            }
        }
        panic!("sv-shell: 未找到可用的系统字体,尝试过 {CANDIDATES:?}");
    })
}

/// 暴露原始字体字节与 collection index(GPU 后端构造 peniko::FontData 用)
#[cfg_attr(not(feature = "backend-vello"), allow(dead_code))]
pub fn ui_font_data() -> (&'static [u8], u32) {
    let (bytes, index) = font_data();
    (bytes.as_slice(), *index)
}

/// 加载(并缓存)系统 UI 字体
pub fn ui_font() -> &'static Font {
    FONT.get_or_init(|| {
        // SV_FONT_SUBST=0 关闭 OpenType 替换表加载(内存实验;调研 15/16)
        let load_subst = std::env::var("SV_FONT_SUBST").map(|v| v != "0").unwrap_or(true);
        let (bytes, index) = font_data();
        let settings = FontSettings {
            collection_index: *index,
            load_substitutions: load_subst,
            ..FontSettings::default()
        };
        Font::from_bytes(bytes.as_slice(), settings)
            .expect("sv-shell: 字体字节已校验可解析,构建 Font 不应失败")
    })
}
