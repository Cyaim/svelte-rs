//! 转译**正确性**抽查:与 `tests/sync.rs`(生成器↔产物一致)互补,这里的
//! 期望值是从 global.less / colors.less 原文人工读出的独立样本 —— 解析器
//! 拿错行、单位算错、语义接线接反,在这里现形。

use sv_arco_tokens::*;

#[test]
fn scalar_tokens_match_less_source() {
    // global.less:radius 2/4/8,size 4px 等差到 200,spacing 非均匀阶梯
    assert_eq!(RADIUS_NONE, 0.0);
    assert_eq!(RADIUS_SMALL, 2.0);
    assert_eq!(RADIUS_MEDIUM, 4.0);
    assert_eq!(RADIUS_LARGE, 8.0);
    assert_eq!(SIZE[1], 4.0);
    assert_eq!(SIZE[8], 32.0);
    assert_eq!(SIZE[50], 200.0);
    assert_eq!(SIZE_MINI, 24.0); // @size-mini: @size-6(别名解引用)
    assert_eq!(SIZE_DEFAULT, 32.0); // @size-default: @size-8
    assert_eq!(SPACING[1], 2.0);
    assert_eq!(SPACING[7], 16.0);
    assert_eq!(SPACING[22], 120.0);
    assert_eq!(BORDER_WIDTH[0], 0.0);
    assert_eq!(BORDER_WIDTH[2], 2.0);
    assert_eq!(OPACITY[6], 0.6);
    assert_eq!(FONT_SIZE_BODY_3, 14.0);
    assert_eq!(FONT_SIZE_TITLE_2, 20.0);
    assert_eq!(FONT_SIZE_DISPLAY_3, 56.0);
    assert_eq!(FONT_SIZE_CAPTION, 12.0);
}

#[test]
fn shadows_match_less_source() {
    // @shadow2-down: 0 4px 10px rgba(0, 0, 0, 0.1);@shadow-special 的 alpha 是 0.3
    let s = SHADOW2.down;
    assert_eq!((s.dx, s.dy, s.blur), (0.0, 4.0, 10.0));
    assert_eq!(s.color, Rgba::new(0, 0, 0, 26)); // 0.1 × 255 四舍五入
    assert_eq!(SHADOW_SPECIAL.blur, 1.0);
    assert_eq!(SHADOW_SPECIAL.color.a, 77); // 0.3 × 255
    assert_eq!(SHADOW1.left_up.dx, -2.0);
    assert_eq!(SHADOW3.right_down.blur, 20.0);
}

#[test]
fn semantic_colors_match_less_source() {
    // 亮:text-1 = neutral-10 = gray-10 = #1d2129;border = gray-3 = #e5e6eb
    assert_eq!(SEMANTIC_LIGHT.text[0].hex(), "#1D2129");
    assert_eq!(SEMANTIC_LIGHT.text[3].hex(), "#C9CDD4");
    assert_eq!(SEMANTIC_LIGHT.fill[1].hex(), "#F2F3F5");
    assert_eq!(SEMANTIC_LIGHT.border.hex(), "#E5E6EB");
    assert_eq!(SEMANTIC_LIGHT.bg[0].hex(), "#FFFFFF");
    // 暗:bg-1 #17171a,border #333335,text-1 = fade(#fff, 90%)
    assert_eq!(SEMANTIC_DARK.bg[0].hex(), "#17171A");
    assert_eq!(SEMANTIC_DARK.border.hex(), "#333335");
    assert_eq!(SEMANTIC_DARK.text[0], Rgba::new(255, 255, 255, 230));
    assert_eq!(SEMANTIC_DARK.fill[0], Rgba::new(255, 255, 255, 10)); // fade 4%
    assert_eq!(semantic(Mode::Light).white.hex(), "#FFFFFF");
}

#[test]
fn functional_wiring_matches_colors_less() {
    // colors.less:primary/link → arcoblue(基准 #165DFF),success → green,
    // warning → orange,danger → red
    assert_eq!(functional(PRIMARY, 6, Mode::Light).hex(), "#165DFF");
    assert_eq!(functional(SUCCESS, 6, Mode::Light).hex(), "#00B42A");
    assert_eq!(functional(WARNING, 6, Mode::Light).hex(), "#FF7D00");
    assert_eq!(functional(DANGER, 6, Mode::Light).hex(), "#F53F3F");
    assert_eq!(functional(LINK, 6, Mode::Light).hex(), "#165DFF");
}

#[test]
fn css_root_blocks_contain_expected_vars() {
    let light = css_root(Mode::Light);
    assert!(light.starts_with(":root {"), "应是 :root 块");
    assert!(light.contains("--arcoblue-6: #165DFF;"));
    assert!(
        light.contains("--primary-6: #165DFF;"),
        "功能色应解析为具体值"
    );
    assert!(light.contains("--color-text-1: #1D2129;"));
    assert!(light.contains("--border-radius-medium: 4px;"));
    assert!(light.contains("--spacing-7: 16px;"));
    assert!(light.contains("--size-default: 32px;"));
    let dark = css_root(Mode::Dark);
    assert!(dark.contains("--color-bg-1: #17171A;"));
    assert!(
        dark.contains("--color-text-1: #FFFFFFE6;"),
        "暗色文本应是 hex-alpha"
    );
    assert!(dark.contains("--gray-1: #17171A;"));
    // 两块的变量名集合应完全一致(暗色是整块替换)
    let names = |s: &str| {
        s.lines()
            .filter_map(|l| l.trim().split_once(':').map(|(n, _)| n.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(names(light), names(dark), "亮暗两块的变量名与顺序应一致");
}
