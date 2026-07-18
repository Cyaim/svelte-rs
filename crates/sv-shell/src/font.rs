//! 系统字体加载(原型:各平台按路径探测,优先 CJK 字体)
//!
//! swash 懒解析:字节读进 `Arc`,[`FontRef`] 只定位表偏移、按需读表——
//! **零拷贝、无全量预解析**(fontdue 急切解析 CJK 字体 ~173MB 常驻内存的
//! 教训,见调研 16/17)。
//!
//! TODO:换成 Parley/fontique 之后由字体系统接管 fallback 链与字形缺失。

use std::sync::{Arc, OnceLock};

use swash::FontRef;

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
static FONT: OnceLock<FontRef<'static>> = OnceLock::new();

/// 原始字体字节 + collection index(swash 与 vello/peniko 共用同一份数据)
fn font_data() -> &'static (Arc<Vec<u8>>, u32) {
    FONT_DATA.get_or_init(|| {
        for path in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                // 校验 swash 能定位字体目录,才认定该候选可用(保持逐候选回退语义)
                if FontRef::from_index(&bytes, 0).is_some() {
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

/// 加载(并缓存)系统 UI 字体引用。
///
/// `FontRef` 是 Copy 的零拷贝视图(data 指针 + 偏移 + 缓存键);缓存一份
/// 保证 CacheKey 稳定,swash ScaleContext 的内部缓存才能命中。
/// TTC 的 collection index 0 与 vello 端 FontData 的 index 语义一致。
pub fn ui_font() -> FontRef<'static> {
    *FONT.get_or_init(|| {
        let (bytes, index) = font_data();
        FontRef::from_index(bytes.as_slice(), *index as usize)
            .expect("sv-shell: 字体字节已校验可解析,构建 FontRef 不应失败")
    })
}

/// 字体身份句柄(调研 24 P0"载体扩宽"):glyph run 从此带字体身份,
/// 光栅缓存/GPU FontData 都按 `key` 索引。单字体阶段恒为内置 UI 字体
/// (key=0);fontique 接管 fallback 后同帧可出现多个 key,载体无需再动
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FontHandle {
    pub key: u64,
}

impl FontHandle {
    /// 按 key 解析 swash FontRef(CPU 光栅/度量用)。
    /// key=0 是内置 UI 字体(TextInput 旧线性路径,P3 随 PlainEditor 切换);
    /// 其余键出自 TextEngine 的 fontique 注册表(调研 24 P1)
    pub fn font_ref(&self) -> FontRef<'static> {
        if self.key == 0 {
            return ui_font();
        }
        crate::text::font_ref_of(self.key).expect("sv-shell: 未注册的字体键")
    }

    /// 原始字节 + collection index(GPU 端构造 peniko::FontData 用)
    #[cfg_attr(not(feature = "backend-vello"), allow(dead_code))]
    pub fn data(&self) -> (&'static [u8], u32) {
        if self.key == 0 {
            return ui_font_data();
        }
        crate::text::font_bytes_of(self.key).expect("sv-shell: 未注册的字体键")
    }
}

/// 内置 UI 字体的句柄(单字体阶段所有 run 共用)
pub fn ui_font_handle() -> FontHandle {
    FontHandle { key: 0 }
}
