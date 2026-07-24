//! Arco 色板梯度算法 —— `@arco-design/color` 0.4.0(commit d882db3e3e25,MIT)
//! 的 `palette.js` / `palette-dark.js` 逐式移植。
//!
//! # 与 JS 的逐位一致性
//!
//! 上游算法跑在 `color@3`(内部是 `color-convert@1.9`)之上,数值行为里有
//! 四个不显眼但影响末位舍入的细节,移植时逐一对齐(金样见 `tests/golden.rs`,
//! 断言字符串直接取自上游仓库自带的 jest 测试原文):
//!
//! 1. **`.hue()` 走 HSL 路由**:`color@3` 的 `hue` getter 定义在
//!    `['hsl', 'hsv', ...]` 上,对 rgb 模型取 `model[0]` = **hsl** ——
//!    HSL/HSV 的色相数学上相等,但浮点运算顺序不同,末位可能差 1 ulp,
//!    所以这里的 `js_hue` 抄的是 `color-convert` 的 **rgb→hsl** 公式。
//!    `.saturationv()` / `.value()` 才走 rgb→hsv(`js_sv`)。
//! 2. **构造钳制**:`Color({h,s,v})` 构造时按通道钳制(h∈\[0,360\]、
//!    s/v∈\[0,100\],`maxfn` 是 clamp 不是取模)。
//! 3. **JS `Math.round`**:半数向 +∞(`floor(x+0.5)`),与 Rust 的
//!    `f64::round`(半数远离零)在正数域一致,但为杜绝歧义仍显式实现。
//! 4. **暗色板经过 hex 舍入**:`palette-dark.js` 先拿亮色板的 **hex 字符串**
//!    再重新解析 —— 亮档结果先量化到 u8 再进暗色计算,不能用未舍入的浮点续算。
//!
//! 所有表达式的括号结构与上游源码保持一致,f64 全程,不做任何"等价化简"。

use crate::Rgb;

/// 亮色板:`base` 为第 6 档基准色,`index` ∈ 1..=10(1 最浅、10 最深)。
///
/// # Panics
///
/// `index` 不在 1..=10 时 panic。
pub fn light(base: Rgb, index: u8) -> Rgb {
    assert!(
        (1..=10).contains(&index),
        "色板档位须在 1..=10,拿到 {index}"
    );
    if index == 6 {
        return base; // palette.js:i === 6 直接返回原色
    }
    let h = js_hue(base);
    let (s, v) = js_sv(base);

    // palette.js 的常数:hueStep=2,饱和度目标 9(亮端)/100(暗端),
    // 明度目标 100(亮端)/30(暗端)
    let hue_step = 2.0_f64;
    let max_saturation_step = 100.0_f64;
    let min_saturation_step = 9.0_f64;
    let max_value = 100.0_f64;
    let min_value = 30.0_f64;

    let is_light = index < 6;
    let i = if is_light { 6 - index } else { index - 6 } as f64;

    // getNewHue:h∈[60,240] 亮端左旋、暗端右旋;其余色域反向。单次 wrap 后取整。
    let new_h = {
        let mut hue = if (60.0..=240.0).contains(&h) {
            if is_light {
                h - hue_step * i
            } else {
                h + hue_step * i
            }
        } else if is_light {
            h + hue_step * i
        } else {
            h - hue_step * i
        };
        if hue < 0.0 {
            hue += 360.0;
        } else if hue >= 360.0 {
            hue -= 360.0;
        }
        js_round(hue)
    };

    // getNewSaturation:亮端向 9 递减(已低于 9 则保持),暗端向 100 递增
    let new_s = if is_light {
        if s <= min_saturation_step {
            s
        } else {
            s - ((s - min_saturation_step) / 5.0) * i
        }
    } else {
        s + ((max_saturation_step - s) / 4.0) * i
    };

    // getNewValue:亮端向 100 递增,暗端向 30 递减(已低于 30 则保持)
    let new_v = if is_light {
        v + ((max_value - v) / 5.0) * i
    } else if v <= min_value {
        v
    } else {
        v - ((v - min_value) / 4.0) * i
    };

    hsv_to_rgb_js(clamp_h(new_h), clamp_pct(new_s), clamp_pct(new_v))
}

/// 暗色板:取亮色板**镜像档位**(`10 - index + 1`)的色相与明度,饱和度按
/// 基准色相分三段修正后线性展开。
///
/// # Panics
///
/// `index` 不在 1..=10 时 panic。
pub fn dark(base: Rgb, index: u8) -> Rgb {
    assert!(
        (1..=10).contains(&index),
        "色板档位须在 1..=10,拿到 {index}"
    );

    // palette-dark.js:Color(colorPalette(originColor, 10 - i + 1))
    // —— 亮档先量化成 hex 再解析(细节 4)
    let light_color = light(base, 10 - index + 1);
    let light_h = js_hue(light_color);
    let (_, light_v) = js_sv(light_color);

    let origin_h = js_hue(base);
    let (origin_s, origin_v) = js_sv(base);
    let _ = origin_v; // baseColor 的 v 只进构造,不参与后续计算

    // getNewSaturation(6):基准饱和度按色相带修正(用的是**原始**饱和度)
    let s6_raw = if (0.0..50.0).contains(&origin_h) {
        origin_s - 15.0
    } else if (50.0..191.0).contains(&origin_h) {
        origin_s - 20.0
    } else {
        origin_s - 15.0 // h ∈ [191, 360]
    };
    // baseSaturation = baseColor.saturationv():读回的是构造钳制后的值(细节 2)
    let base_saturation = clamp_pct(s6_raw);

    let step = ((base_saturation - 9.0) / 4.0).ceil();
    let step1to5 = ((100.0 - base_saturation) / 5.0).ceil();

    let new_s = if index < 6 {
        base_saturation + (6 - index) as f64 * step1to5
    } else if index == 6 {
        s6_raw // ==6 分支返回未钳制的原式,钳制发生在最终构造
    } else {
        base_saturation - step * (index - 6) as f64
    };

    hsv_to_rgb_js(clamp_h(light_h), clamp_pct(new_s), clamp_pct(light_v))
}

/// 整组亮色板(下标 0 = 第 1 档)。
pub fn light10(base: Rgb) -> [Rgb; 10] {
    core::array::from_fn(|i| light(base, (i + 1) as u8))
}

/// 整组暗色板(下标 0 = 第 1 档)。
pub fn dark10(base: Rgb) -> [Rgb; 10] {
    core::array::from_fn(|i| dark(base, (i + 1) as u8))
}

/// JS `Math.round` 语义:半数向 +∞。仅用于正数域(色相、RGB 通道)。
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

fn clamp_h(h: f64) -> f64 {
    h.clamp(0.0, 360.0)
}

fn clamp_pct(x: f64) -> f64 {
    x.clamp(0.0, 100.0)
}

/// `color-convert` rgb→hsl 的色相通道(`color@3` 的 `.hue()` 实际路由,细节 1)。
fn js_hue(c: Rgb) -> f64 {
    let r = c.r as f64 / 255.0;
    let g = c.g as f64 / 255.0;
    let b = c.b as f64 / 255.0;
    let min = r.min(g).min(b);
    let max = r.max(g).max(b);
    let delta = max - min;
    let mut h = if max == min {
        0.0
    } else if r == max {
        (g - b) / delta
    } else if g == max {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    h = (h * 60.0).min(360.0);
    if h < 0.0 {
        h += 360.0;
    }
    h
}

/// `color-convert` rgb→hsv 的饱和度/明度通道(0..=100 浮点)。
fn js_sv(c: Rgb) -> (f64, f64) {
    let r = c.r as f64 / 255.0;
    let g = c.g as f64 / 255.0;
    let b = c.b as f64 / 255.0;
    let v = r.max(g).max(b);
    let diff = v - r.min(g).min(b);
    let s = if diff == 0.0 { 0.0 } else { diff / v };
    (s * 100.0, v * 100.0)
}

/// `color-convert` hsv→rgb + `color@3` `.round()`(JS `Math.round` 每通道)。
fn hsv_to_rgb_js(h: f64, s: f64, v: f64) -> Rgb {
    let h = h / 60.0;
    let s = s / 100.0;
    let v = v / 100.0;
    let hi = (h.floor() as i64).rem_euclid(6); // h∈[0,6],floor 后 %6 只把 6 折回 0

    let f = h - h.floor();
    let p = 255.0 * v * (1.0 - s);
    let q = 255.0 * v * (1.0 - (s * f));
    let t = 255.0 * v * (1.0 - (s * (1.0 - f)));
    let v = 255.0 * v;

    let (r, g, b) = match hi {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Rgb::new(js_round(r) as u8, js_round(g) as u8, js_round(b) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 极端输入不 panic、通道不越界(算法只保证 1..=10 档位契约)。
    #[test]
    fn edge_inputs_do_not_panic() {
        for base in [
            Rgb::new(0, 0, 0),       // 黑:s=0, v=0
            Rgb::new(255, 255, 255), // 白:s=0, v=100
            Rgb::new(128, 128, 128), // 中灰:diff=0 分支
            Rgb::new(255, 0, 4),     // h≈359:暗端旋转触发 wrap
            Rgb::new(10, 5, 200),    // 低亮度高饱和
        ] {
            for i in 1..=10 {
                let _ = light(base, i);
                let _ = dark(base, i);
            }
        }
    }

    #[test]
    fn index_6_is_identity_in_light_mode() {
        let base = Rgb::new(22, 93, 255);
        assert_eq!(light(base, 6), base);
    }

    #[test]
    #[should_panic(expected = "1..=10")]
    fn index_0_panics() {
        let _ = light(Rgb::new(1, 2, 3), 0);
    }

    #[test]
    #[should_panic(expected = "1..=10")]
    fn index_11_panics_dark() {
        let _ = dark(Rgb::new(1, 2, 3), 11);
    }
}
