//! Arco Design 设计令牌层(`sv-arco` 组件库的地基,调研 26 §4 的 A0 波次)。
//!
//! 两部分内容,均可独立使用:
//!
//! 1. **色板算法**([`palette`]):任一基准色 → 10 档梯度色板,亮/暗双模式。
//!    移植自 `@arco-design/color` 0.4.0(MIT),数值行为与上游逐字节对拍
//!    (见 `tests/golden.rs`,金样为上游仓库自带的 jest 断言原文)。
//! 2. **全局令牌**(本模块顶层常量,由 `src/generated.rs` 提供):圆角/字号/
//!    尺寸/间距/阴影/亮暗语义色 + 14 组预置色板,转译自
//!    `@arco-design/web-react` 2.66.16 的 `global.less` / `colors.less`。
//!    双出口:Rust 常量(给 `view!` 宏与运行时逻辑)与 [`CSS_ROOT_LIGHT`] /
//!    [`CSS_ROOT_DARK`](`:root { --x }` 文本,给 `.svelte` 的 `<style>` 块,
//!    走 sv-compiler 已落地的编译期 `var()` 代入)。
//!
//! # 转译而非手抄
//!
//! `src/generated.rs` 由 `cargo run -p sv-arco-tokens --bin gen_tokens` 从
//! `assets/*.less`(vendored,来源版本见文件头)重新生成,`tests/sync.rs`
//! 强制产物与生成器一致 —— arco 升版时更新 assets、重跑、diff 审查即可。
//!
//! # 许可与署名
//!
//! 视觉规范与 design token 派生自 ByteDance Arco Design(MIT),许可证原文
//! 见仓内 `LICENSE-ARCO`。本 crate 为非官方实现,与 ByteDance 无关联、未获
//! 其背书("unofficial, not affiliated with or endorsed by ByteDance")。

pub mod palette;

#[doc(hidden)]
pub mod generator;

// skip:generated.rs 与生成器逐字符对拍(tests/sync.rs),不能被 fmt 重排
#[rustfmt::skip]
mod generated;
pub use generated::*;

/// 不透明 RGB 颜色(sRGB,8 位/通道)。
///
/// 刻意不依赖 sv-ui 的颜色类型 —— 令牌层零依赖,转换由消费方(sv-arco)做。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// 大写十六进制形式,如 `#FFECE8`(与上游 `@arco-design/color` 的
    /// hex 输出逐字符一致,金样测试直接比这个字符串)。
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// 解析 `#RRGGBB`(可省 `#`,大小写不敏感);其余形状返回 `None`。
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        Some(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }

    /// 附上 alpha 变成 [`Rgba`]。
    pub const fn with_alpha(self, a: u8) -> Rgba {
        Rgba {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }
}

/// 带 alpha 的 RGB 颜色(非预乘)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// 大写十六进制形式:不透明时 `#RRGGBB`,否则 `#RRGGBBAA`
    /// (sv-compiler 的 CSS 子集支持 hex-alpha)。
    pub fn hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

/// 亮/暗主题模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Light,
    Dark,
}

/// 14 组预置色板的名字。判别值即 [`PALETTES_LIGHT`] / [`PALETTES_DARK`] 的行下标。
///
/// 顺序与 `@arco-design/color` 的 `colorList` 一致(gray 排最后;gray 不走
/// 梯度算法,是 arco 手调的字面值)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Palette {
    Red = 0,
    Orangered = 1,
    Orange = 2,
    Gold = 3,
    Yellow = 4,
    Lime = 5,
    Green = 6,
    Cyan = 7,
    Blue = 8,
    Arcoblue = 9,
    Purple = 10,
    Pinkpurple = 11,
    Magenta = 12,
    Gray = 13,
}

impl Palette {
    pub const ALL: [Palette; 14] = [
        Palette::Red,
        Palette::Orangered,
        Palette::Orange,
        Palette::Gold,
        Palette::Yellow,
        Palette::Lime,
        Palette::Green,
        Palette::Cyan,
        Palette::Blue,
        Palette::Arcoblue,
        Palette::Purple,
        Palette::Pinkpurple,
        Palette::Magenta,
        Palette::Gray,
    ];

    /// arco 的色板名(小写,与 CSS 变量前缀一致,如 `arcoblue`)。
    pub const fn name(self) -> &'static str {
        match self {
            Palette::Red => "red",
            Palette::Orangered => "orangered",
            Palette::Orange => "orange",
            Palette::Gold => "gold",
            Palette::Yellow => "yellow",
            Palette::Lime => "lime",
            Palette::Green => "green",
            Palette::Cyan => "cyan",
            Palette::Blue => "blue",
            Palette::Arcoblue => "arcoblue",
            Palette::Purple => "purple",
            Palette::Pinkpurple => "pinkpurple",
            Palette::Magenta => "magenta",
            Palette::Gray => "gray",
        }
    }
}

/// 功能色 → 色板的接线(colors.less:`@primary-*` 引 arcoblue,下同)。
pub const PRIMARY: Palette = Palette::Arcoblue;
pub const SUCCESS: Palette = Palette::Green;
pub const WARNING: Palette = Palette::Orange;
pub const DANGER: Palette = Palette::Red;
pub const LINK: Palette = Palette::Arcoblue;

/// 取一组色板(10 档,`[0]` = 第 1 档最浅,`[5]` = 第 6 档基准)。
pub fn palette_of(p: Palette, mode: Mode) -> &'static [Rgb; 10] {
    match mode {
        Mode::Light => &PALETTES_LIGHT[p as usize],
        Mode::Dark => &PALETTES_DARK[p as usize],
    }
}

/// 功能色便捷取值:`step` 为 arco 档位 1..=10(6 = 基准)。
///
/// # Panics
///
/// `step` 不在 1..=10 时 panic(与 [`palette::light`] 同一契约)。
pub fn functional(p: Palette, step: u8, mode: Mode) -> Rgb {
    assert!((1..=10).contains(&step), "色板档位须在 1..=10,拿到 {step}");
    palette_of(p, mode)[(step - 1) as usize]
}

/// 单层阴影(arco 的 box-shadow 全系单层高斯:偏移 + blur + 低 alpha 黑)。
///
/// 目前只是**数据**:box-shadow 渲染动词尚未落地(CSS-SUPPORT ⏳ 等 vello
/// 消费 `blur:true`),先把参数空间钉死,渲染侧接上时零改动。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub color: Rgba,
}

/// 一档阴影的九个方向(arco:`shadow{1,2,3}-{center,up,down,...}`)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSet {
    pub center: Shadow,
    pub up: Shadow,
    pub down: Shadow,
    pub left: Shadow,
    pub right: Shadow,
    pub left_up: Shadow,
    pub left_down: Shadow,
    pub right_up: Shadow,
    pub right_down: Shadow,
}

/// 一套主题模式下的语义色(文本四层/填充四层/边框四层/背景五层)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticColors {
    /// `color-text-1..4`:标题正文 → 禁用,依次变浅。
    pub text: [Rgba; 4],
    /// `color-fill-1..4`:填充,依次变深。
    pub fill: [Rgba; 4],
    /// `color-border-1..4`:浅 → 深。
    pub border_levels: [Rgba; 4],
    /// `color-border`:默认边框色。
    pub border: Rgba,
    /// `color-bg-1..5`:整体背景 → 各级容器背景。
    pub bg: [Rgba; 5],
    /// `color-bg-white`:亮色模式下"白色控件底"在两种模式里的对应色。
    pub bg_white: Rgba,
    pub white: Rgba,
    pub black: Rgba,
}

/// 按模式取语义色。
pub fn semantic(mode: Mode) -> &'static SemanticColors {
    match mode {
        Mode::Light => &SEMANTIC_LIGHT,
        Mode::Dark => &SEMANTIC_DARK,
    }
}

/// 按模式取 `:root` CSS 变量块。
pub fn css_root(mode: Mode) -> &'static str {
    match mode {
        Mode::Light => CSS_ROOT_LIGHT,
        Mode::Dark => CSS_ROOT_DARK,
    }
}
