//! 系统字体加载(原型:各平台按路径探测,优先 CJK 字体)
//!
//! TODO:换成 Parley/fontique 之后由字体系统接管 fallback 链与字形缺失。

use std::sync::OnceLock;

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

static FONT: OnceLock<Font> = OnceLock::new();

/// 加载(并缓存)系统 UI 字体
pub fn ui_font() -> &'static Font {
    FONT.get_or_init(|| {
        for path in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                let settings = FontSettings { collection_index: 0, ..FontSettings::default() };
                if let Ok(font) = Font::from_bytes(bytes, settings) {
                    return font;
                }
            }
        }
        panic!("sv-shell: 未找到可用的系统字体,尝试过 {CANDIDATES:?}");
    })
}
