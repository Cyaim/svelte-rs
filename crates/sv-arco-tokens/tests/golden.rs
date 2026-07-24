//! 金样对拍:上游 `@arco-design/color` 仓库**自带的 jest 断言原文**(vendored
//! 于 `assets/`,经 `tools/extract-golden.mjs` 机械提取为 `fixtures/golden_data.rs`)
//! vs 本移植。13 组 × 亮/暗 × 10 档全部逐字符串比对 —— 舍入差一个字节都过不了。

use sv_arco_tokens::{Mode, Palette, Rgb, palette, palette_of};

include!("fixtures/golden_data.rs");

#[test]
fn chromatic_palettes_match_upstream() {
    assert_eq!(GOLDEN_CHROMATIC.len(), 13, "金样应有 13 组");
    for (name, base_hex, light, dark) in GOLDEN_CHROMATIC {
        let base = Rgb::from_hex(base_hex).expect("金样基准色应可解析");
        for i in 0..10u8 {
            assert_eq!(
                palette::light(base, i + 1).hex(),
                light[i as usize],
                "{name} 亮色第 {} 档",
                i + 1
            );
            assert_eq!(
                palette::dark(base, i + 1).hex(),
                dark[i as usize],
                "{name} 暗色第 {} 档",
                i + 1
            );
        }
    }
}

/// 生成的静态表也钉在金样上(防转译层拿错基准色/串行)。
#[test]
fn generated_tables_match_upstream() {
    for (row, (name, _, light, dark)) in GOLDEN_CHROMATIC.iter().enumerate() {
        let p = Palette::ALL[row];
        assert_eq!(p.name(), *name, "Palette 枚举顺序应与金样一致");
        for i in 0..10 {
            assert_eq!(
                palette_of(p, Mode::Light)[i].hex(),
                light[i],
                "{name} 亮 {}",
                i + 1
            );
            assert_eq!(
                palette_of(p, Mode::Dark)[i].hex(),
                dark[i],
                "{name} 暗 {}",
                i + 1
            );
        }
    }
    for i in 0..10 {
        assert_eq!(
            palette_of(Palette::Gray, Mode::Light)[i].hex(),
            GOLDEN_GRAY_LIGHT[i]
        );
        assert_eq!(
            palette_of(Palette::Gray, Mode::Dark)[i].hex(),
            GOLDEN_GRAY_DARK[i]
        );
    }
}

/// 上游 `generate single color` it 块的四条单点断言。
#[test]
fn single_color_spot_checks() {
    let red = Rgb::from_hex("#F53F3F").unwrap();
    assert_eq!(palette::light(red, 1).hex(), "#FFECE8");
    assert_eq!(palette::dark(red, 1).hex(), "#4D000A");
    assert_eq!(palette::light(red, 10).hex(), "#4D000A");
    assert_eq!(palette::dark(red, 10).hex(), "#FFF0EC");
}
