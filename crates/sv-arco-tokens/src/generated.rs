//! 机器生成 —— `cargo run -p sv-arco-tokens --bin gen_tokens` 重写,勿手改。
//! 来源:`@arco-design/web-react` 2.66.16(commit fbf2ec0a8cc2)的
//! `global.less` / `colors.less`(vendored 于 `assets/`);色板由
//! `crate::palette` 现算。与生成器的一致性由 `tests/sync.rs` 守护。
//!
//! rustfmt 经由 lib.rs 里 `mod generated` 上的 `#[rustfmt::skip]` 跳过
//! 本文件:同步测试比对生成器的逐字符输出,fmt 一重排就永久对不上。

use crate::{Rgb, Rgba, SemanticColors, Shadow, ShadowSet};

/// 亮色模式 14 组色板 × 10 档;行下标 = `Palette` 判别值,列下标 = 档位-1。
pub static PALETTES_LIGHT: [[Rgb; 10]; 14] = [
    // red(基准 #F53F3F)
    [Rgb::new(255, 236, 232), Rgb::new(253, 205, 197), Rgb::new(251, 172, 163), Rgb::new(249, 137, 129), Rgb::new(247, 101, 96), Rgb::new(245, 63, 63), Rgb::new(203, 39, 45), Rgb::new(161, 21, 30), Rgb::new(119, 8, 19), Rgb::new(77, 0, 10)],
    // orangered(基准 #F77234)
    [Rgb::new(255, 243, 232), Rgb::new(253, 221, 195), Rgb::new(252, 197, 159), Rgb::new(250, 172, 123), Rgb::new(249, 144, 87), Rgb::new(247, 114, 52), Rgb::new(204, 81, 32), Rgb::new(162, 53, 17), Rgb::new(119, 31, 6), Rgb::new(77, 14, 0)],
    // orange(基准 #FF7D00)
    [Rgb::new(255, 247, 232), Rgb::new(255, 228, 186), Rgb::new(255, 207, 139), Rgb::new(255, 182, 93), Rgb::new(255, 154, 46), Rgb::new(255, 125, 0), Rgb::new(210, 95, 0), Rgb::new(166, 69, 0), Rgb::new(121, 46, 0), Rgb::new(77, 27, 0)],
    // gold(基准 #F7BA1E)
    [Rgb::new(255, 252, 232), Rgb::new(253, 244, 191), Rgb::new(252, 233, 150), Rgb::new(250, 220, 109), Rgb::new(249, 204, 69), Rgb::new(247, 186, 30), Rgb::new(204, 146, 19), Rgb::new(162, 109, 10), Rgb::new(119, 75, 4), Rgb::new(77, 45, 0)],
    // yellow(基准 #FADC19)
    [Rgb::new(254, 255, 232), Rgb::new(254, 254, 190), Rgb::new(253, 250, 148), Rgb::new(252, 242, 107), Rgb::new(251, 232, 66), Rgb::new(250, 220, 25), Rgb::new(207, 175, 15), Rgb::new(163, 132, 8), Rgb::new(120, 93, 3), Rgb::new(77, 56, 0)],
    // lime(基准 #9FDB1D)
    [Rgb::new(252, 255, 232), Rgb::new(237, 248, 187), Rgb::new(220, 241, 144), Rgb::new(201, 233, 104), Rgb::new(181, 226, 65), Rgb::new(159, 219, 29), Rgb::new(126, 183, 18), Rgb::new(95, 148, 10), Rgb::new(67, 112, 4), Rgb::new(42, 77, 0)],
    // green(基准 #00B42A)
    [Rgb::new(232, 255, 234), Rgb::new(175, 240, 181), Rgb::new(123, 225, 136), Rgb::new(76, 210, 99), Rgb::new(35, 195, 67), Rgb::new(0, 180, 42), Rgb::new(0, 154, 41), Rgb::new(0, 128, 38), Rgb::new(0, 102, 34), Rgb::new(0, 77, 28)],
    // cyan(基准 #14C9C9)
    [Rgb::new(232, 255, 251), Rgb::new(183, 244, 236), Rgb::new(137, 233, 224), Rgb::new(94, 223, 214), Rgb::new(55, 212, 207), Rgb::new(20, 201, 201), Rgb::new(13, 165, 170), Rgb::new(7, 130, 139), Rgb::new(3, 97, 108), Rgb::new(0, 66, 77)],
    // blue(基准 #3491FA)
    [Rgb::new(232, 247, 255), Rgb::new(195, 231, 254), Rgb::new(159, 212, 253), Rgb::new(123, 192, 252), Rgb::new(87, 169, 251), Rgb::new(52, 145, 250), Rgb::new(32, 108, 207), Rgb::new(17, 75, 163), Rgb::new(6, 48, 120), Rgb::new(0, 26, 77)],
    // arcoblue(基准 #165DFF)
    [Rgb::new(232, 243, 255), Rgb::new(190, 218, 255), Rgb::new(148, 191, 255), Rgb::new(106, 161, 255), Rgb::new(64, 128, 255), Rgb::new(22, 93, 255), Rgb::new(14, 66, 210), Rgb::new(7, 44, 166), Rgb::new(3, 26, 121), Rgb::new(0, 13, 77)],
    // purple(基准 #722ED1)
    [Rgb::new(245, 232, 255), Rgb::new(221, 190, 246), Rgb::new(195, 150, 237), Rgb::new(168, 113, 227), Rgb::new(141, 78, 218), Rgb::new(114, 46, 209), Rgb::new(85, 29, 176), Rgb::new(60, 16, 143), Rgb::new(39, 6, 110), Rgb::new(22, 0, 77)],
    // pinkpurple(基准 #D91AD9)
    [Rgb::new(255, 232, 251), Rgb::new(247, 186, 239), Rgb::new(240, 142, 230), Rgb::new(232, 101, 223), Rgb::new(225, 62, 219), Rgb::new(217, 26, 217), Rgb::new(176, 16, 182), Rgb::new(138, 9, 147), Rgb::new(101, 3, 112), Rgb::new(66, 0, 77)],
    // magenta(基准 #F5319D)
    [Rgb::new(255, 232, 241), Rgb::new(253, 194, 219), Rgb::new(251, 157, 199), Rgb::new(249, 121, 183), Rgb::new(247, 84, 168), Rgb::new(245, 49, 157), Rgb::new(203, 30, 131), Rgb::new(161, 16, 105), Rgb::new(119, 6, 79), Rgb::new(77, 0, 52)],
    // gray(arco 手调字面值,不走算法)
    [Rgb::new(247, 248, 250), Rgb::new(242, 243, 245), Rgb::new(229, 230, 235), Rgb::new(201, 205, 212), Rgb::new(169, 174, 184), Rgb::new(134, 144, 156), Rgb::new(107, 119, 133), Rgb::new(78, 89, 105), Rgb::new(39, 46, 59), Rgb::new(29, 33, 41)],
];

/// 暗色模式 14 组色板 × 10 档;行下标 = `Palette` 判别值,列下标 = 档位-1。
pub static PALETTES_DARK: [[Rgb; 10]; 14] = [
    // red(基准 #F53F3F)
    [Rgb::new(77, 0, 10), Rgb::new(119, 6, 17), Rgb::new(161, 22, 31), Rgb::new(203, 46, 52), Rgb::new(245, 78, 78), Rgb::new(247, 105, 101), Rgb::new(249, 141, 134), Rgb::new(251, 176, 167), Rgb::new(253, 209, 202), Rgb::new(255, 240, 236)],
    // orangered(基准 #F77234)
    [Rgb::new(77, 14, 0), Rgb::new(119, 30, 5), Rgb::new(162, 55, 20), Rgb::new(204, 87, 41), Rgb::new(247, 126, 69), Rgb::new(249, 146, 90), Rgb::new(250, 173, 125), Rgb::new(252, 198, 161), Rgb::new(253, 222, 197), Rgb::new(255, 244, 235)],
    // orange(基准 #FF7D00)
    [Rgb::new(77, 27, 0), Rgb::new(121, 48, 4), Rgb::new(166, 75, 10), Rgb::new(210, 105, 19), Rgb::new(255, 141, 31), Rgb::new(255, 150, 38), Rgb::new(255, 179, 87), Rgb::new(255, 205, 135), Rgb::new(255, 227, 184), Rgb::new(255, 247, 232)],
    // gold(基准 #F7BA1E)
    [Rgb::new(77, 45, 0), Rgb::new(119, 75, 4), Rgb::new(162, 111, 15), Rgb::new(204, 150, 31), Rgb::new(247, 192, 52), Rgb::new(249, 204, 68), Rgb::new(250, 220, 108), Rgb::new(252, 233, 149), Rgb::new(253, 244, 190), Rgb::new(255, 252, 232)],
    // yellow(基准 #FADC19)
    [Rgb::new(77, 56, 0), Rgb::new(120, 94, 7), Rgb::new(163, 134, 20), Rgb::new(207, 179, 37), Rgb::new(250, 225, 60), Rgb::new(251, 233, 75), Rgb::new(252, 243, 116), Rgb::new(253, 250, 157), Rgb::new(254, 254, 198), Rgb::new(254, 255, 240)],
    // lime(基准 #9FDB1D)
    [Rgb::new(42, 77, 0), Rgb::new(68, 112, 6), Rgb::new(98, 148, 18), Rgb::new(132, 183, 35), Rgb::new(168, 219, 57), Rgb::new(184, 226, 75), Rgb::new(203, 233, 112), Rgb::new(222, 241, 152), Rgb::new(238, 248, 194), Rgb::new(253, 255, 238)],
    // green(基准 #00B42A)
    [Rgb::new(0, 77, 28), Rgb::new(4, 102, 37), Rgb::new(10, 128, 45), Rgb::new(18, 154, 55), Rgb::new(29, 180, 64), Rgb::new(39, 195, 70), Rgb::new(80, 210, 102), Rgb::new(126, 225, 139), Rgb::new(178, 240, 183), Rgb::new(235, 255, 236)],
    // cyan(基准 #14C9C9)
    [Rgb::new(0, 66, 77), Rgb::new(6, 97, 108), Rgb::new(17, 131, 139), Rgb::new(31, 166, 170), Rgb::new(48, 201, 201), Rgb::new(63, 212, 207), Rgb::new(102, 223, 215), Rgb::new(144, 233, 225), Rgb::new(190, 244, 237), Rgb::new(240, 255, 252)],
    // blue(基准 #3491FA)
    [Rgb::new(0, 26, 77), Rgb::new(5, 47, 120), Rgb::new(19, 76, 163), Rgb::new(41, 113, 207), Rgb::new(70, 154, 250), Rgb::new(90, 170, 251), Rgb::new(125, 193, 252), Rgb::new(161, 213, 253), Rgb::new(198, 232, 254), Rgb::new(234, 248, 255)],
    // arcoblue(基准 #165DFF)
    [Rgb::new(0, 13, 77), Rgb::new(4, 27, 121), Rgb::new(14, 50, 166), Rgb::new(29, 77, 210), Rgb::new(48, 111, 255), Rgb::new(60, 126, 255), Rgb::new(104, 159, 255), Rgb::new(147, 190, 255), Rgb::new(190, 218, 255), Rgb::new(234, 244, 255)],
    // purple(基准 #722ED1)
    [Rgb::new(22, 0, 77), Rgb::new(39, 6, 110), Rgb::new(62, 19, 143), Rgb::new(90, 37, 176), Rgb::new(123, 61, 209), Rgb::new(142, 81, 218), Rgb::new(169, 116, 227), Rgb::new(197, 154, 237), Rgb::new(223, 194, 246), Rgb::new(247, 237, 255)],
    // pinkpurple(基准 #D91AD9)
    [Rgb::new(66, 0, 77), Rgb::new(101, 3, 112), Rgb::new(138, 13, 147), Rgb::new(176, 27, 182), Rgb::new(217, 46, 217), Rgb::new(225, 61, 219), Rgb::new(232, 102, 223), Rgb::new(240, 146, 230), Rgb::new(247, 193, 240), Rgb::new(255, 242, 253)],
    // magenta(基准 #F5319D)
    [Rgb::new(77, 0, 52), Rgb::new(119, 8, 80), Rgb::new(161, 23, 108), Rgb::new(203, 43, 136), Rgb::new(245, 69, 166), Rgb::new(247, 86, 169), Rgb::new(249, 122, 184), Rgb::new(251, 158, 200), Rgb::new(253, 195, 219), Rgb::new(255, 232, 241)],
    // gray(arco 手调字面值,不走算法)
    [Rgb::new(23, 23, 26), Rgb::new(46, 46, 48), Rgb::new(72, 72, 73), Rgb::new(95, 95, 96), Rgb::new(120, 120, 122), Rgb::new(146, 146, 147), Rgb::new(171, 171, 172), Rgb::new(197, 197, 197), Rgb::new(223, 223, 223), Rgb::new(246, 246, 246)],
];

/// 亮色模式语义色。
pub static SEMANTIC_LIGHT: SemanticColors = SemanticColors {
    text: [Rgba::new(29, 33, 41, 255), Rgba::new(78, 89, 105, 255), Rgba::new(134, 144, 156, 255), Rgba::new(201, 205, 212, 255)],
    fill: [Rgba::new(247, 248, 250, 255), Rgba::new(242, 243, 245, 255), Rgba::new(229, 230, 235, 255), Rgba::new(201, 205, 212, 255)],
    border_levels: [Rgba::new(242, 243, 245, 255), Rgba::new(229, 230, 235, 255), Rgba::new(201, 205, 212, 255), Rgba::new(134, 144, 156, 255)],
    border: Rgba::new(229, 230, 235, 255),
    bg: [Rgba::new(255, 255, 255, 255), Rgba::new(255, 255, 255, 255), Rgba::new(255, 255, 255, 255), Rgba::new(255, 255, 255, 255), Rgba::new(255, 255, 255, 255)],
    bg_white: Rgba::new(255, 255, 255, 255),
    white: Rgba::new(255, 255, 255, 255),
    black: Rgba::new(0, 0, 0, 255),
};

/// 暗色模式语义色。
pub static SEMANTIC_DARK: SemanticColors = SemanticColors {
    text: [Rgba::new(255, 255, 255, 230), Rgba::new(255, 255, 255, 179), Rgba::new(255, 255, 255, 128), Rgba::new(255, 255, 255, 77)],
    fill: [Rgba::new(255, 255, 255, 10), Rgba::new(255, 255, 255, 20), Rgba::new(255, 255, 255, 31), Rgba::new(255, 255, 255, 41)],
    border_levels: [Rgba::new(46, 46, 48, 255), Rgba::new(72, 72, 73, 255), Rgba::new(95, 95, 96, 255), Rgba::new(146, 146, 147, 255)],
    border: Rgba::new(51, 51, 53, 255),
    bg: [Rgba::new(23, 23, 26, 255), Rgba::new(35, 35, 36, 255), Rgba::new(42, 42, 43, 255), Rgba::new(49, 49, 50, 255), Rgba::new(55, 55, 57, 255)],
    bg_white: Rgba::new(246, 246, 246, 255),
    white: Rgba::new(255, 255, 255, 230),
    black: Rgba::new(0, 0, 0, 255),
};

/// 边框宽度 `border-none` + `border-1..5`(px)。
pub const BORDER_WIDTH: [f32; 6] = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];

/// 圆角 `border-radius-none`(px;`circle` 档是 50%,不适用桌面场景未编)。
pub const RADIUS_NONE: f32 = 0.0;
/// 圆角 `border-radius-small`(px;`circle` 档是 50%,不适用桌面场景未编)。
pub const RADIUS_SMALL: f32 = 2.0;
/// 圆角 `border-radius-medium`(px;`circle` 档是 50%,不适用桌面场景未编)。
pub const RADIUS_MEDIUM: f32 = 4.0;
/// 圆角 `border-radius-large`(px;`circle` 档是 50%,不适用桌面场景未编)。
pub const RADIUS_LARGE: f32 = 8.0;

/// 尺寸阶梯 `size-none` + `size-1..50`(px,4px 等差;下标即档位)。
pub const SIZE: [f32; 51] = [0.0, 4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 28.0, 32.0, 36.0, 40.0, 44.0, 48.0, 52.0, 56.0, 60.0, 64.0, 68.0, 72.0, 76.0, 80.0, 84.0, 88.0, 92.0, 96.0, 100.0, 104.0, 108.0, 112.0, 116.0, 120.0, 124.0, 128.0, 132.0, 136.0, 140.0, 144.0, 148.0, 152.0, 156.0, 160.0, 164.0, 168.0, 172.0, 176.0, 180.0, 184.0, 188.0, 192.0, 196.0, 200.0];

/// 控件高度档 `size-mini`。
pub const SIZE_MINI: f32 = 24.0;
/// 控件高度档 `size-small`。
pub const SIZE_SMALL: f32 = 28.0;
/// 控件高度档 `size-default`。
pub const SIZE_DEFAULT: f32 = 32.0;
/// 控件高度档 `size-large`。
pub const SIZE_LARGE: f32 = 36.0;

/// 间距阶梯 `spacing-none` + `spacing-1..22`(px,非均匀;下标即档位)。
pub const SPACING: [f32; 23] = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 20.0, 24.0, 32.0, 36.0, 40.0, 48.0, 56.0, 60.0, 64.0, 72.0, 80.0, 84.0, 96.0, 100.0, 120.0];

/// 不透明度阶梯 `opacity-none` + `opacity-1..10`(0.0..=1.0;下标即档位)。
pub const OPACITY: [f32; 11] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

/// 字号 `font-size-body-1`(px)。
pub const FONT_SIZE_BODY_1: f32 = 12.0;
/// 字号 `font-size-body-2`(px)。
pub const FONT_SIZE_BODY_2: f32 = 13.0;
/// 字号 `font-size-body-3`(px)。
pub const FONT_SIZE_BODY_3: f32 = 14.0;
/// 字号 `font-size-title-1`(px)。
pub const FONT_SIZE_TITLE_1: f32 = 16.0;
/// 字号 `font-size-title-2`(px)。
pub const FONT_SIZE_TITLE_2: f32 = 20.0;
/// 字号 `font-size-title-3`(px)。
pub const FONT_SIZE_TITLE_3: f32 = 24.0;
/// 字号 `font-size-display-1`(px)。
pub const FONT_SIZE_DISPLAY_1: f32 = 36.0;
/// 字号 `font-size-display-2`(px)。
pub const FONT_SIZE_DISPLAY_2: f32 = 48.0;
/// 字号 `font-size-display-3`(px)。
pub const FONT_SIZE_DISPLAY_3: f32 = 56.0;
/// 字号 `font-size-caption`(px)。
pub const FONT_SIZE_CAPTION: f32 = 12.0;

/// 特殊阴影 `shadow-special`(0 0 1px 黑 30%)。
pub const SHADOW_SPECIAL: Shadow = Shadow { dx: 0.0, dy: 0.0, blur: 1.0, color: Rgba::new(0, 0, 0, 77) };

/// 阴影第 1 档 `shadow1-*` 全九向。
pub const SHADOW1: ShadowSet = ShadowSet {
    center: Shadow { dx: 0.0, dy: 0.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    up: Shadow { dx: 0.0, dy: -2.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    down: Shadow { dx: 0.0, dy: 2.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    left: Shadow { dx: -2.0, dy: 0.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    right: Shadow { dx: 2.0, dy: 0.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    left_up: Shadow { dx: -2.0, dy: -2.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    left_down: Shadow { dx: -2.0, dy: 2.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    right_up: Shadow { dx: 2.0, dy: -2.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
    right_down: Shadow { dx: 2.0, dy: 2.0, blur: 5.0, color: Rgba::new(0, 0, 0, 26) },
};

/// 阴影第 2 档 `shadow2-*` 全九向。
pub const SHADOW2: ShadowSet = ShadowSet {
    center: Shadow { dx: 0.0, dy: 0.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    up: Shadow { dx: 0.0, dy: -4.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    down: Shadow { dx: 0.0, dy: 4.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    left: Shadow { dx: -4.0, dy: 0.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    right: Shadow { dx: 4.0, dy: 0.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    left_up: Shadow { dx: -4.0, dy: -4.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    left_down: Shadow { dx: -4.0, dy: 4.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    right_up: Shadow { dx: 4.0, dy: -4.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
    right_down: Shadow { dx: 4.0, dy: 4.0, blur: 10.0, color: Rgba::new(0, 0, 0, 26) },
};

/// 阴影第 3 档 `shadow3-*` 全九向。
pub const SHADOW3: ShadowSet = ShadowSet {
    center: Shadow { dx: 0.0, dy: 0.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    up: Shadow { dx: 0.0, dy: -8.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    down: Shadow { dx: 0.0, dy: 8.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    left: Shadow { dx: -8.0, dy: 0.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    right: Shadow { dx: 8.0, dy: 0.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    left_up: Shadow { dx: -8.0, dy: -8.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    left_down: Shadow { dx: -8.0, dy: 8.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    right_up: Shadow { dx: 8.0, dy: -8.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
    right_down: Shadow { dx: 8.0, dy: 8.0, blur: 20.0, color: Rgba::new(0, 0, 0, 26) },
};

/// 亮色模式全部令牌的 `:root` CSS 变量块(`.svelte` 的 `<style>` 用)。
pub const CSS_ROOT_LIGHT: &str = r#":root {
  --red-1: #FFECE8;
  --red-2: #FDCDC5;
  --red-3: #FBACA3;
  --red-4: #F98981;
  --red-5: #F76560;
  --red-6: #F53F3F;
  --red-7: #CB272D;
  --red-8: #A1151E;
  --red-9: #770813;
  --red-10: #4D000A;
  --orangered-1: #FFF3E8;
  --orangered-2: #FDDDC3;
  --orangered-3: #FCC59F;
  --orangered-4: #FAAC7B;
  --orangered-5: #F99057;
  --orangered-6: #F77234;
  --orangered-7: #CC5120;
  --orangered-8: #A23511;
  --orangered-9: #771F06;
  --orangered-10: #4D0E00;
  --orange-1: #FFF7E8;
  --orange-2: #FFE4BA;
  --orange-3: #FFCF8B;
  --orange-4: #FFB65D;
  --orange-5: #FF9A2E;
  --orange-6: #FF7D00;
  --orange-7: #D25F00;
  --orange-8: #A64500;
  --orange-9: #792E00;
  --orange-10: #4D1B00;
  --gold-1: #FFFCE8;
  --gold-2: #FDF4BF;
  --gold-3: #FCE996;
  --gold-4: #FADC6D;
  --gold-5: #F9CC45;
  --gold-6: #F7BA1E;
  --gold-7: #CC9213;
  --gold-8: #A26D0A;
  --gold-9: #774B04;
  --gold-10: #4D2D00;
  --yellow-1: #FEFFE8;
  --yellow-2: #FEFEBE;
  --yellow-3: #FDFA94;
  --yellow-4: #FCF26B;
  --yellow-5: #FBE842;
  --yellow-6: #FADC19;
  --yellow-7: #CFAF0F;
  --yellow-8: #A38408;
  --yellow-9: #785D03;
  --yellow-10: #4D3800;
  --lime-1: #FCFFE8;
  --lime-2: #EDF8BB;
  --lime-3: #DCF190;
  --lime-4: #C9E968;
  --lime-5: #B5E241;
  --lime-6: #9FDB1D;
  --lime-7: #7EB712;
  --lime-8: #5F940A;
  --lime-9: #437004;
  --lime-10: #2A4D00;
  --green-1: #E8FFEA;
  --green-2: #AFF0B5;
  --green-3: #7BE188;
  --green-4: #4CD263;
  --green-5: #23C343;
  --green-6: #00B42A;
  --green-7: #009A29;
  --green-8: #008026;
  --green-9: #006622;
  --green-10: #004D1C;
  --cyan-1: #E8FFFB;
  --cyan-2: #B7F4EC;
  --cyan-3: #89E9E0;
  --cyan-4: #5EDFD6;
  --cyan-5: #37D4CF;
  --cyan-6: #14C9C9;
  --cyan-7: #0DA5AA;
  --cyan-8: #07828B;
  --cyan-9: #03616C;
  --cyan-10: #00424D;
  --blue-1: #E8F7FF;
  --blue-2: #C3E7FE;
  --blue-3: #9FD4FD;
  --blue-4: #7BC0FC;
  --blue-5: #57A9FB;
  --blue-6: #3491FA;
  --blue-7: #206CCF;
  --blue-8: #114BA3;
  --blue-9: #063078;
  --blue-10: #001A4D;
  --arcoblue-1: #E8F3FF;
  --arcoblue-2: #BEDAFF;
  --arcoblue-3: #94BFFF;
  --arcoblue-4: #6AA1FF;
  --arcoblue-5: #4080FF;
  --arcoblue-6: #165DFF;
  --arcoblue-7: #0E42D2;
  --arcoblue-8: #072CA6;
  --arcoblue-9: #031A79;
  --arcoblue-10: #000D4D;
  --purple-1: #F5E8FF;
  --purple-2: #DDBEF6;
  --purple-3: #C396ED;
  --purple-4: #A871E3;
  --purple-5: #8D4EDA;
  --purple-6: #722ED1;
  --purple-7: #551DB0;
  --purple-8: #3C108F;
  --purple-9: #27066E;
  --purple-10: #16004D;
  --pinkpurple-1: #FFE8FB;
  --pinkpurple-2: #F7BAEF;
  --pinkpurple-3: #F08EE6;
  --pinkpurple-4: #E865DF;
  --pinkpurple-5: #E13EDB;
  --pinkpurple-6: #D91AD9;
  --pinkpurple-7: #B010B6;
  --pinkpurple-8: #8A0993;
  --pinkpurple-9: #650370;
  --pinkpurple-10: #42004D;
  --magenta-1: #FFE8F1;
  --magenta-2: #FDC2DB;
  --magenta-3: #FB9DC7;
  --magenta-4: #F979B7;
  --magenta-5: #F754A8;
  --magenta-6: #F5319D;
  --magenta-7: #CB1E83;
  --magenta-8: #A11069;
  --magenta-9: #77064F;
  --magenta-10: #4D0034;
  --gray-1: #F7F8FA;
  --gray-2: #F2F3F5;
  --gray-3: #E5E6EB;
  --gray-4: #C9CDD4;
  --gray-5: #A9AEB8;
  --gray-6: #86909C;
  --gray-7: #6B7785;
  --gray-8: #4E5969;
  --gray-9: #272E3B;
  --gray-10: #1D2129;
  --primary-1: #E8F3FF;
  --primary-2: #BEDAFF;
  --primary-3: #94BFFF;
  --primary-4: #6AA1FF;
  --primary-5: #4080FF;
  --primary-6: #165DFF;
  --primary-7: #0E42D2;
  --primary-8: #072CA6;
  --primary-9: #031A79;
  --primary-10: #000D4D;
  --success-1: #E8FFEA;
  --success-2: #AFF0B5;
  --success-3: #7BE188;
  --success-4: #4CD263;
  --success-5: #23C343;
  --success-6: #00B42A;
  --success-7: #009A29;
  --success-8: #008026;
  --success-9: #006622;
  --success-10: #004D1C;
  --warning-1: #FFF7E8;
  --warning-2: #FFE4BA;
  --warning-3: #FFCF8B;
  --warning-4: #FFB65D;
  --warning-5: #FF9A2E;
  --warning-6: #FF7D00;
  --warning-7: #D25F00;
  --warning-8: #A64500;
  --warning-9: #792E00;
  --warning-10: #4D1B00;
  --danger-1: #FFECE8;
  --danger-2: #FDCDC5;
  --danger-3: #FBACA3;
  --danger-4: #F98981;
  --danger-5: #F76560;
  --danger-6: #F53F3F;
  --danger-7: #CB272D;
  --danger-8: #A1151E;
  --danger-9: #770813;
  --danger-10: #4D000A;
  --link-1: #E8F3FF;
  --link-2: #BEDAFF;
  --link-3: #94BFFF;
  --link-4: #6AA1FF;
  --link-5: #4080FF;
  --link-6: #165DFF;
  --link-7: #0E42D2;
  --link-8: #072CA6;
  --link-9: #031A79;
  --link-10: #000D4D;
  --color-text-1: #1D2129;
  --color-text-2: #4E5969;
  --color-text-3: #86909C;
  --color-text-4: #C9CDD4;
  --color-fill-1: #F7F8FA;
  --color-fill-2: #F2F3F5;
  --color-fill-3: #E5E6EB;
  --color-fill-4: #C9CDD4;
  --color-border-1: #F2F3F5;
  --color-border-2: #E5E6EB;
  --color-border-3: #C9CDD4;
  --color-border-4: #86909C;
  --color-border: #E5E6EB;
  --color-bg-1: #FFFFFF;
  --color-bg-2: #FFFFFF;
  --color-bg-3: #FFFFFF;
  --color-bg-4: #FFFFFF;
  --color-bg-5: #FFFFFF;
  --color-bg-white: #FFFFFF;
  --color-white: #FFFFFF;
  --color-black: #000000;
  --border-1: 1px;
  --border-2: 2px;
  --border-3: 3px;
  --border-4: 4px;
  --border-5: 5px;
  --border-radius-none: 0;
  --border-radius-small: 2px;
  --border-radius-medium: 4px;
  --border-radius-large: 8px;
  --font-size-body-1: 12px;
  --font-size-body-2: 13px;
  --font-size-body-3: 14px;
  --font-size-title-1: 16px;
  --font-size-title-2: 20px;
  --font-size-title-3: 24px;
  --font-size-display-1: 36px;
  --font-size-display-2: 48px;
  --font-size-display-3: 56px;
  --font-size-caption: 12px;
  --spacing-1: 2px;
  --spacing-2: 4px;
  --spacing-3: 6px;
  --spacing-4: 8px;
  --spacing-5: 10px;
  --spacing-6: 12px;
  --spacing-7: 16px;
  --spacing-8: 20px;
  --spacing-9: 24px;
  --spacing-10: 32px;
  --spacing-11: 36px;
  --spacing-12: 40px;
  --spacing-13: 48px;
  --spacing-14: 56px;
  --spacing-15: 60px;
  --spacing-16: 64px;
  --spacing-17: 72px;
  --spacing-18: 80px;
  --spacing-19: 84px;
  --spacing-20: 96px;
  --spacing-21: 100px;
  --spacing-22: 120px;
  --size-1: 4px;
  --size-2: 8px;
  --size-3: 12px;
  --size-4: 16px;
  --size-5: 20px;
  --size-6: 24px;
  --size-7: 28px;
  --size-8: 32px;
  --size-9: 36px;
  --size-10: 40px;
  --size-11: 44px;
  --size-12: 48px;
  --size-13: 52px;
  --size-14: 56px;
  --size-15: 60px;
  --size-16: 64px;
  --size-17: 68px;
  --size-18: 72px;
  --size-19: 76px;
  --size-20: 80px;
  --size-21: 84px;
  --size-22: 88px;
  --size-23: 92px;
  --size-24: 96px;
  --size-25: 100px;
  --size-26: 104px;
  --size-27: 108px;
  --size-28: 112px;
  --size-29: 116px;
  --size-30: 120px;
  --size-31: 124px;
  --size-32: 128px;
  --size-33: 132px;
  --size-34: 136px;
  --size-35: 140px;
  --size-36: 144px;
  --size-37: 148px;
  --size-38: 152px;
  --size-39: 156px;
  --size-40: 160px;
  --size-41: 164px;
  --size-42: 168px;
  --size-43: 172px;
  --size-44: 176px;
  --size-45: 180px;
  --size-46: 184px;
  --size-47: 188px;
  --size-48: 192px;
  --size-49: 196px;
  --size-50: 200px;
  --size-mini: 24px;
  --size-small: 28px;
  --size-default: 32px;
  --size-large: 36px;
}"#;

/// 暗色模式全部令牌的 `:root` CSS 变量块(与亮色同名,整块替换)。
pub const CSS_ROOT_DARK: &str = r#":root {
  --red-1: #4D000A;
  --red-2: #770611;
  --red-3: #A1161F;
  --red-4: #CB2E34;
  --red-5: #F54E4E;
  --red-6: #F76965;
  --red-7: #F98D86;
  --red-8: #FBB0A7;
  --red-9: #FDD1CA;
  --red-10: #FFF0EC;
  --orangered-1: #4D0E00;
  --orangered-2: #771E05;
  --orangered-3: #A23714;
  --orangered-4: #CC5729;
  --orangered-5: #F77E45;
  --orangered-6: #F9925A;
  --orangered-7: #FAAD7D;
  --orangered-8: #FCC6A1;
  --orangered-9: #FDDEC5;
  --orangered-10: #FFF4EB;
  --orange-1: #4D1B00;
  --orange-2: #793004;
  --orange-3: #A64B0A;
  --orange-4: #D26913;
  --orange-5: #FF8D1F;
  --orange-6: #FF9626;
  --orange-7: #FFB357;
  --orange-8: #FFCD87;
  --orange-9: #FFE3B8;
  --orange-10: #FFF7E8;
  --gold-1: #4D2D00;
  --gold-2: #774B04;
  --gold-3: #A26F0F;
  --gold-4: #CC961F;
  --gold-5: #F7C034;
  --gold-6: #F9CC44;
  --gold-7: #FADC6C;
  --gold-8: #FCE995;
  --gold-9: #FDF4BE;
  --gold-10: #FFFCE8;
  --yellow-1: #4D3800;
  --yellow-2: #785E07;
  --yellow-3: #A38614;
  --yellow-4: #CFB325;
  --yellow-5: #FAE13C;
  --yellow-6: #FBE94B;
  --yellow-7: #FCF374;
  --yellow-8: #FDFA9D;
  --yellow-9: #FEFEC6;
  --yellow-10: #FEFFF0;
  --lime-1: #2A4D00;
  --lime-2: #447006;
  --lime-3: #629412;
  --lime-4: #84B723;
  --lime-5: #A8DB39;
  --lime-6: #B8E24B;
  --lime-7: #CBE970;
  --lime-8: #DEF198;
  --lime-9: #EEF8C2;
  --lime-10: #FDFFEE;
  --green-1: #004D1C;
  --green-2: #046625;
  --green-3: #0A802D;
  --green-4: #129A37;
  --green-5: #1DB440;
  --green-6: #27C346;
  --green-7: #50D266;
  --green-8: #7EE18B;
  --green-9: #B2F0B7;
  --green-10: #EBFFEC;
  --cyan-1: #00424D;
  --cyan-2: #06616C;
  --cyan-3: #11838B;
  --cyan-4: #1FA6AA;
  --cyan-5: #30C9C9;
  --cyan-6: #3FD4CF;
  --cyan-7: #66DFD7;
  --cyan-8: #90E9E1;
  --cyan-9: #BEF4ED;
  --cyan-10: #F0FFFC;
  --blue-1: #001A4D;
  --blue-2: #052F78;
  --blue-3: #134CA3;
  --blue-4: #2971CF;
  --blue-5: #469AFA;
  --blue-6: #5AAAFB;
  --blue-7: #7DC1FC;
  --blue-8: #A1D5FD;
  --blue-9: #C6E8FE;
  --blue-10: #EAF8FF;
  --arcoblue-1: #000D4D;
  --arcoblue-2: #041B79;
  --arcoblue-3: #0E32A6;
  --arcoblue-4: #1D4DD2;
  --arcoblue-5: #306FFF;
  --arcoblue-6: #3C7EFF;
  --arcoblue-7: #689FFF;
  --arcoblue-8: #93BEFF;
  --arcoblue-9: #BEDAFF;
  --arcoblue-10: #EAF4FF;
  --purple-1: #16004D;
  --purple-2: #27066E;
  --purple-3: #3E138F;
  --purple-4: #5A25B0;
  --purple-5: #7B3DD1;
  --purple-6: #8E51DA;
  --purple-7: #A974E3;
  --purple-8: #C59AED;
  --purple-9: #DFC2F6;
  --purple-10: #F7EDFF;
  --pinkpurple-1: #42004D;
  --pinkpurple-2: #650370;
  --pinkpurple-3: #8A0D93;
  --pinkpurple-4: #B01BB6;
  --pinkpurple-5: #D92ED9;
  --pinkpurple-6: #E13DDB;
  --pinkpurple-7: #E866DF;
  --pinkpurple-8: #F092E6;
  --pinkpurple-9: #F7C1F0;
  --pinkpurple-10: #FFF2FD;
  --magenta-1: #4D0034;
  --magenta-2: #770850;
  --magenta-3: #A1176C;
  --magenta-4: #CB2B88;
  --magenta-5: #F545A6;
  --magenta-6: #F756A9;
  --magenta-7: #F97AB8;
  --magenta-8: #FB9EC8;
  --magenta-9: #FDC3DB;
  --magenta-10: #FFE8F1;
  --gray-1: #17171A;
  --gray-2: #2E2E30;
  --gray-3: #484849;
  --gray-4: #5F5F60;
  --gray-5: #78787A;
  --gray-6: #929293;
  --gray-7: #ABABAC;
  --gray-8: #C5C5C5;
  --gray-9: #DFDFDF;
  --gray-10: #F6F6F6;
  --primary-1: #000D4D;
  --primary-2: #041B79;
  --primary-3: #0E32A6;
  --primary-4: #1D4DD2;
  --primary-5: #306FFF;
  --primary-6: #3C7EFF;
  --primary-7: #689FFF;
  --primary-8: #93BEFF;
  --primary-9: #BEDAFF;
  --primary-10: #EAF4FF;
  --success-1: #004D1C;
  --success-2: #046625;
  --success-3: #0A802D;
  --success-4: #129A37;
  --success-5: #1DB440;
  --success-6: #27C346;
  --success-7: #50D266;
  --success-8: #7EE18B;
  --success-9: #B2F0B7;
  --success-10: #EBFFEC;
  --warning-1: #4D1B00;
  --warning-2: #793004;
  --warning-3: #A64B0A;
  --warning-4: #D26913;
  --warning-5: #FF8D1F;
  --warning-6: #FF9626;
  --warning-7: #FFB357;
  --warning-8: #FFCD87;
  --warning-9: #FFE3B8;
  --warning-10: #FFF7E8;
  --danger-1: #4D000A;
  --danger-2: #770611;
  --danger-3: #A1161F;
  --danger-4: #CB2E34;
  --danger-5: #F54E4E;
  --danger-6: #F76965;
  --danger-7: #F98D86;
  --danger-8: #FBB0A7;
  --danger-9: #FDD1CA;
  --danger-10: #FFF0EC;
  --link-1: #000D4D;
  --link-2: #041B79;
  --link-3: #0E32A6;
  --link-4: #1D4DD2;
  --link-5: #306FFF;
  --link-6: #3C7EFF;
  --link-7: #689FFF;
  --link-8: #93BEFF;
  --link-9: #BEDAFF;
  --link-10: #EAF4FF;
  --color-text-1: #FFFFFFE6;
  --color-text-2: #FFFFFFB3;
  --color-text-3: #FFFFFF80;
  --color-text-4: #FFFFFF4D;
  --color-fill-1: #FFFFFF0A;
  --color-fill-2: #FFFFFF14;
  --color-fill-3: #FFFFFF1F;
  --color-fill-4: #FFFFFF29;
  --color-border-1: #2E2E30;
  --color-border-2: #484849;
  --color-border-3: #5F5F60;
  --color-border-4: #929293;
  --color-border: #333335;
  --color-bg-1: #17171A;
  --color-bg-2: #232324;
  --color-bg-3: #2A2A2B;
  --color-bg-4: #313132;
  --color-bg-5: #373739;
  --color-bg-white: #F6F6F6;
  --color-white: #FFFFFFE6;
  --color-black: #000000;
  --border-1: 1px;
  --border-2: 2px;
  --border-3: 3px;
  --border-4: 4px;
  --border-5: 5px;
  --border-radius-none: 0;
  --border-radius-small: 2px;
  --border-radius-medium: 4px;
  --border-radius-large: 8px;
  --font-size-body-1: 12px;
  --font-size-body-2: 13px;
  --font-size-body-3: 14px;
  --font-size-title-1: 16px;
  --font-size-title-2: 20px;
  --font-size-title-3: 24px;
  --font-size-display-1: 36px;
  --font-size-display-2: 48px;
  --font-size-display-3: 56px;
  --font-size-caption: 12px;
  --spacing-1: 2px;
  --spacing-2: 4px;
  --spacing-3: 6px;
  --spacing-4: 8px;
  --spacing-5: 10px;
  --spacing-6: 12px;
  --spacing-7: 16px;
  --spacing-8: 20px;
  --spacing-9: 24px;
  --spacing-10: 32px;
  --spacing-11: 36px;
  --spacing-12: 40px;
  --spacing-13: 48px;
  --spacing-14: 56px;
  --spacing-15: 60px;
  --spacing-16: 64px;
  --spacing-17: 72px;
  --spacing-18: 80px;
  --spacing-19: 84px;
  --spacing-20: 96px;
  --spacing-21: 100px;
  --spacing-22: 120px;
  --size-1: 4px;
  --size-2: 8px;
  --size-3: 12px;
  --size-4: 16px;
  --size-5: 20px;
  --size-6: 24px;
  --size-7: 28px;
  --size-8: 32px;
  --size-9: 36px;
  --size-10: 40px;
  --size-11: 44px;
  --size-12: 48px;
  --size-13: 52px;
  --size-14: 56px;
  --size-15: 60px;
  --size-16: 64px;
  --size-17: 68px;
  --size-18: 72px;
  --size-19: 76px;
  --size-20: 80px;
  --size-21: 84px;
  --size-22: 88px;
  --size-23: 92px;
  --size-24: 96px;
  --size-25: 100px;
  --size-26: 104px;
  --size-27: 108px;
  --size-28: 112px;
  --size-29: 116px;
  --size-30: 120px;
  --size-31: 124px;
  --size-32: 128px;
  --size-33: 132px;
  --size-34: 136px;
  --size-35: 140px;
  --size-36: 144px;
  --size-37: 148px;
  --size-38: 152px;
  --size-39: 156px;
  --size-40: 160px;
  --size-41: 164px;
  --size-42: 168px;
  --size-43: 172px;
  --size-44: 176px;
  --size-45: 180px;
  --size-46: 184px;
  --size-47: 188px;
  --size-48: 192px;
  --size-49: 196px;
  --size-50: 200px;
  --size-mini: 24px;
  --size-small: 28px;
  --size-default: 32px;
  --size-large: 36px;
}"#;
