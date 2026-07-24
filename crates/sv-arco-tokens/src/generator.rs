//! `src/generated.rs` 的生成器(`#[doc(hidden)]`:只服务 `bin/gen_tokens`
//! 与 `tests/sync.rs`,不属于公开 API)。
//!
//! 输入:`assets/global.less` + `assets/colors.less`(vendored 自
//! `@arco-design/web-react` 2.66.16,commit fbf2ec0a8cc2);色板经
//! [`crate::palette`] 现算。输出:`src/generated.rs` 全文。
//!
//! 数值表(尺寸/间距/字号/阴影/色板基准/灰阶/暗色语义)一律**解析**自 less
//! 原文,不手抄;唯独"功能色→色板"与"语义色→灰阶"两张接线表是 less 里的
//! `rgb(var(...))` 间接层,无法机械解析,按 colors.less/global.less 对应行
//! 手工对照写死(行号见各处注释),arco 升版时人工复核这两张表。

use crate::{Rgb, Rgba, palette};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const GLOBAL_LESS: &str = include_str!("../assets/global.less");
const COLORS_LESS: &str = include_str!("../assets/colors.less");

/// 13 组算法色板的名字(顺序 = `@arco-design/color` colorList = `Palette` 判别值)。
const CHROMATIC: [&str; 13] = [
    "red",
    "orangered",
    "orange",
    "gold",
    "yellow",
    "lime",
    "green",
    "cyan",
    "blue",
    "arcoblue",
    "purple",
    "pinkpurple",
    "magenta",
];

/// 生成 `src/generated.rs` 的完整内容。
pub fn generate() -> String {
    let vars = parse_vars(&[GLOBAL_LESS, COLORS_LESS]);
    let v = Vars(&vars);

    // ---- 色板 ----------------------------------------------------------
    let bases: Vec<Rgb> = CHROMATIC.iter().map(|n| v.hex(&format!("{n}-6"))).collect();
    let gray_light: [Rgb; 10] = core::array::from_fn(|i| v.hex(&format!("gray-{}", i + 1)));
    let gray_dark: [Rgb; 10] = core::array::from_fn(|i| v.hex(&format!("dark-gray-{}", i + 1)));

    let mut palettes_light: Vec<[Rgb; 10]> = bases.iter().map(|b| palette::light10(*b)).collect();
    let mut palettes_dark: Vec<[Rgb; 10]> = bases.iter().map(|b| palette::dark10(*b)).collect();
    palettes_light.push(gray_light);
    palettes_dark.push(gray_dark);

    // ---- 语义色 ---------------------------------------------------------
    // 接线表(global.less L297-316):text-1..4 = neutral-10/8/6/4,
    // fill-1..4 = neutral-1..4,border-1..4 = neutral-2/3/4/6,border = gray-3;
    // 暗色模式 neutral-N 即 dark-gray-N,text/fill 则是 fade(#fff) 系(L362-369)。
    let solid = |c: Rgb| c.with_alpha(255);
    let light_sem = Semantic {
        text: [gray_light[9], gray_light[7], gray_light[5], gray_light[3]].map(solid),
        fill: [gray_light[0], gray_light[1], gray_light[2], gray_light[3]].map(solid),
        border_levels: [gray_light[1], gray_light[2], gray_light[3], gray_light[5]].map(solid),
        border: solid(gray_light[2]),
        bg: core::array::from_fn(|i| solid(v.hex(&format!("color-bg-{}", i + 1)))),
        bg_white: solid(v.hex("color-bg-white")),
        white: solid(v.hex("color-white")),
        black: solid(v.hex("color-black")),
    };
    let dark_sem = Semantic {
        text: core::array::from_fn(|i| v.fade(&format!("dark-color-text-{}", i + 1))),
        fill: core::array::from_fn(|i| v.fade(&format!("dark-color-fill-{}", i + 1))),
        border_levels: [gray_dark[1], gray_dark[2], gray_dark[3], gray_dark[5]].map(solid),
        border: solid(v.hex("dark-color-border")),
        bg: core::array::from_fn(|i| solid(v.hex(&format!("dark-color-bg-{}", i + 1)))),
        bg_white: solid(v.hex("dark-color-bg-white")),
        white: v.fade("dark-color-white"),
        black: solid(v.hex("dark-color-black")),
    };

    // ---- 标量令牌 --------------------------------------------------------
    let border_width: Vec<f64> = std::iter::once(v.px("border-none"))
        .chain((1..=5).map(|i| v.px(&format!("border-{i}"))))
        .collect();
    let size: Vec<f64> = std::iter::once(v.px("size-none"))
        .chain((1..=50).map(|i| v.px(&format!("size-{i}"))))
        .collect();
    let spacing: Vec<f64> = std::iter::once(v.px("spacing-none"))
        .chain((1..=22).map(|i| v.px(&format!("spacing-{i}"))))
        .collect();
    let opacity: Vec<f64> = std::iter::once(v.pct("opacity-none"))
        .chain((1..=10).map(|i| v.pct(&format!("opacity-{i}"))))
        .collect();

    let radius = ["none", "small", "medium", "large"].map(|n| v.px(&format!("border-radius-{n}")));
    let size_named = ["mini", "small", "default", "large"].map(|n| v.px(&format!("size-{n}")));
    let font = FontSizes {
        body: [1, 2, 3].map(|i| v.px(&format!("font-size-body-{i}"))),
        title: [1, 2, 3].map(|i| v.px(&format!("font-size-title-{i}"))),
        display: [1, 2, 3].map(|i| v.px(&format!("font-size-display-{i}"))),
        caption: v.px("font-size-caption"),
    };

    let shadow_special = v.shadow("shadow-special");
    let shadow_sets: Vec<Vec<ShadowVal>> = (1..=3)
        .map(|lvl| {
            SHADOW_DIRS
                .iter()
                .map(|d| v.shadow(&format!("shadow{lvl}-{d}")))
                .collect()
        })
        .collect();

    // ---- 渲染 ------------------------------------------------------------
    let mut out = String::new();
    out.push_str(
        "//! 机器生成 —— `cargo run -p sv-arco-tokens --bin gen_tokens` 重写,勿手改。\n\
         //! 来源:`@arco-design/web-react` 2.66.16(commit fbf2ec0a8cc2)的\n\
         //! `global.less` / `colors.less`(vendored 于 `assets/`);色板由\n\
         //! `crate::palette` 现算。与生成器的一致性由 `tests/sync.rs` 守护。\n\
         //!\n\
         //! rustfmt 经由 lib.rs 里 `mod generated` 上的 `#[rustfmt::skip]` 跳过\n\
         //! 本文件:同步测试比对生成器的逐字符输出,fmt 一重排就永久对不上。\n\n\
         use crate::{Rgb, Rgba, SemanticColors, Shadow, ShadowSet};\n\n",
    );

    render_palettes(
        &mut out,
        "PALETTES_LIGHT",
        "亮色模式",
        &palettes_light,
        &bases,
    );
    render_palettes(
        &mut out,
        "PALETTES_DARK",
        "暗色模式",
        &palettes_dark,
        &bases,
    );
    render_semantic(&mut out, "SEMANTIC_LIGHT", "亮色", &light_sem);
    render_semantic(&mut out, "SEMANTIC_DARK", "暗色", &dark_sem);

    writeln!(out, "/// 边框宽度 `border-none` + `border-1..5`(px)。").unwrap();
    writeln!(
        out,
        "pub const BORDER_WIDTH: [f32; 6] = {};\n",
        r_f32_arr(&border_width)
    )
    .unwrap();

    let radius_names = ["NONE", "SMALL", "MEDIUM", "LARGE"];
    for (n, val) in radius_names.iter().zip(radius) {
        writeln!(
            out,
            "/// 圆角 `border-radius-{}`(px;`circle` 档是 50%,不适用桌面场景未编)。",
            n.to_lowercase()
        )
        .unwrap();
        writeln!(out, "pub const RADIUS_{n}: f32 = {};", r_f32(val)).unwrap();
    }
    out.push('\n');

    writeln!(
        out,
        "/// 尺寸阶梯 `size-none` + `size-1..50`(px,4px 等差;下标即档位)。"
    )
    .unwrap();
    writeln!(out, "pub const SIZE: [f32; 51] = {};\n", r_f32_arr(&size)).unwrap();
    let size_named_names = ["MINI", "SMALL", "DEFAULT", "LARGE"];
    for (n, val) in size_named_names.iter().zip(size_named) {
        writeln!(out, "/// 控件高度档 `size-{}`。", n.to_lowercase()).unwrap();
        writeln!(out, "pub const SIZE_{n}: f32 = {};", r_f32(val)).unwrap();
    }
    out.push('\n');

    writeln!(
        out,
        "/// 间距阶梯 `spacing-none` + `spacing-1..22`(px,非均匀;下标即档位)。"
    )
    .unwrap();
    writeln!(
        out,
        "pub const SPACING: [f32; 23] = {};\n",
        r_f32_arr(&spacing)
    )
    .unwrap();
    writeln!(
        out,
        "/// 不透明度阶梯 `opacity-none` + `opacity-1..10`(0.0..=1.0;下标即档位)。"
    )
    .unwrap();
    writeln!(
        out,
        "pub const OPACITY: [f32; 11] = {};\n",
        r_f32_arr(&opacity)
    )
    .unwrap();

    for (grp, vals) in [
        ("BODY", font.body),
        ("TITLE", font.title),
        ("DISPLAY", font.display),
    ] {
        for (i, val) in vals.iter().enumerate() {
            writeln!(
                out,
                "/// 字号 `font-size-{}-{}`(px)。",
                grp.to_lowercase(),
                i + 1
            )
            .unwrap();
            writeln!(
                out,
                "pub const FONT_SIZE_{grp}_{}: f32 = {};",
                i + 1,
                r_f32(*val)
            )
            .unwrap();
        }
    }
    writeln!(out, "/// 字号 `font-size-caption`(px)。").unwrap();
    writeln!(
        out,
        "pub const FONT_SIZE_CAPTION: f32 = {};\n",
        r_f32(font.caption)
    )
    .unwrap();

    writeln!(out, "/// 特殊阴影 `shadow-special`(0 0 1px 黑 30%)。").unwrap();
    writeln!(
        out,
        "pub const SHADOW_SPECIAL: Shadow = {};\n",
        r_shadow(&shadow_special)
    )
    .unwrap();
    for (lvl, set) in shadow_sets.iter().enumerate() {
        let lvl = lvl + 1;
        writeln!(out, "/// 阴影第 {lvl} 档 `shadow{lvl}-*` 全九向。").unwrap();
        writeln!(out, "pub const SHADOW{lvl}: ShadowSet = ShadowSet {{").unwrap();
        for (dir, sh) in SHADOW_DIRS.iter().zip(set) {
            writeln!(out, "    {}: {},", dir.replace('-', "_"), r_shadow(sh)).unwrap();
        }
        out.push_str("};\n\n");
    }

    let css_light = render_css(
        &palettes_light,
        &light_sem,
        &border_width,
        &size,
        size_named,
        &spacing,
        radius,
        &font,
    );
    let css_dark = render_css(
        &palettes_dark,
        &dark_sem,
        &border_width,
        &size,
        size_named,
        &spacing,
        radius,
        &font,
    );
    writeln!(
        out,
        "/// 亮色模式全部令牌的 `:root` CSS 变量块(`.svelte` 的 `<style>` 用)。"
    )
    .unwrap();
    writeln!(
        out,
        "pub const CSS_ROOT_LIGHT: &str = r#\"{css_light}\"#;\n"
    )
    .unwrap();
    writeln!(
        out,
        "/// 暗色模式全部令牌的 `:root` CSS 变量块(与亮色同名,整块替换)。"
    )
    .unwrap();
    writeln!(out, "pub const CSS_ROOT_DARK: &str = r#\"{css_dark}\"#;").unwrap();

    out
}

const SHADOW_DIRS: [&str; 9] = [
    "center",
    "up",
    "down",
    "left",
    "right",
    "left-up",
    "left-down",
    "right-up",
    "right-down",
];

struct Semantic {
    text: [Rgba; 4],
    fill: [Rgba; 4],
    border_levels: [Rgba; 4],
    border: Rgba,
    bg: [Rgba; 5],
    bg_white: Rgba,
    white: Rgba,
    black: Rgba,
}

struct FontSizes {
    body: [f64; 3],
    title: [f64; 3],
    display: [f64; 3],
    caption: f64,
}

#[derive(Clone)]
struct ShadowVal {
    dx: f64,
    dy: f64,
    blur: f64,
    color: Rgba,
}

// ---- less 解析 ------------------------------------------------------------

/// 逐行收集 `@name: value;`(跳过 `@import`/`@plugin`;值取到第一个 `;` 为止,
/// 行尾 `//` 注释因此自然剥落)。后写的文件覆盖先写的(本用途下无同名冲突)。
fn parse_vars(sources: &[&str]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for src in sources {
        for line in src.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix('@') else {
                continue;
            };
            if rest.starts_with("import") || rest.starts_with("plugin") || rest.starts_with('{') {
                continue;
            }
            let Some((name, value)) = rest.split_once(':') else {
                continue;
            };
            let Some(value) = value.split(';').next() else {
                continue;
            };
            map.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

struct Vars<'a>(&'a BTreeMap<String, String>);

impl Vars<'_> {
    /// 取值并解引用别名链(如 `@size-mini: @size-6`)。
    fn resolve(&self, name: &str) -> &str {
        let mut cur = name;
        for _ in 0..8 {
            let val = self
                .0
                .get(cur)
                .unwrap_or_else(|| panic!("less 变量缺失:@{cur}"));
            match val.strip_prefix('@') {
                // 纯别名(裸 @token)才追;带表达式的值(rgb(var(...)) 等)原样返回
                Some(next) if !next.contains(|c: char| c.is_whitespace() || c == '(') => cur = next,
                _ => return val,
            }
        }
        panic!("less 别名链过深:@{name}");
    }

    fn px(&self, name: &str) -> f64 {
        let val = self.resolve(name);
        val.strip_suffix("px")
            .unwrap_or(val)
            .parse()
            .unwrap_or_else(|_| panic!("@{name} 不是 px 值:{val}"))
    }

    fn pct(&self, name: &str) -> f64 {
        let val = self.resolve(name);
        let n: f64 = val
            .strip_suffix('%')
            .unwrap_or(val)
            .parse()
            .unwrap_or_else(|_| panic!("@{name} 不是百分比:{val}"));
        n / 100.0
    }

    fn hex(&self, name: &str) -> Rgb {
        let val = self.resolve(name);
        let val = if val == "#fff" { "#ffffff" } else { val };
        Rgb::from_hex(val).unwrap_or_else(|| panic!("@{name} 不是六位 hex:{val}"))
    }

    /// `fade(#fff, N%)` → 白 + alpha(N% → 四舍五入到字节)。
    fn fade(&self, name: &str) -> Rgba {
        let val = self.resolve(name);
        let inner = val
            .strip_prefix("fade(#fff,")
            .and_then(|s| s.strip_suffix("%)"))
            .unwrap_or_else(|| panic!("@{name} 不是 fade(#fff, N%):{val}"));
        let pct: f64 = inner
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("@{name} 百分比坏:{val}"));
        Rgba::new(255, 255, 255, (pct / 100.0 * 255.0).round() as u8)
    }

    /// `X Y BLURpx rgba(r, g, b, a)` → 阴影参数。
    fn shadow(&self, name: &str) -> ShadowVal {
        let val = self.resolve(name);
        let (head, tail) = val
            .split_once("rgba(")
            .unwrap_or_else(|| panic!("@{name} 不是阴影:{val}"));
        let px = |s: &str| -> f64 {
            s.strip_suffix("px")
                .unwrap_or(s)
                .parse()
                .unwrap_or_else(|_| panic!("@{name} 偏移坏:{s}"))
        };
        let nums: Vec<f64> = head.split_whitespace().map(px).collect();
        let [dx, dy, blur] = nums[..] else {
            panic!("@{name} 应为三段偏移:{val}")
        };
        let rgba: Vec<f64> = tail
            .trim_end()
            .trim_end_matches(')')
            .split(',')
            .map(|s| {
                s.trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("@{name} rgba 坏:{val}"))
            })
            .collect();
        let [r, g, b, a] = rgba[..] else {
            panic!("@{name} rgba 应为四通道:{val}")
        };
        ShadowVal {
            dx,
            dy,
            blur,
            color: Rgba::new(r as u8, g as u8, b as u8, (a * 255.0).round() as u8),
        }
    }
}

// ---- Rust 源渲染 -----------------------------------------------------------

fn r_f32(v: f64) -> String {
    format!("{:?}", v as f32)
}

fn r_f32_arr(vals: &[f64]) -> String {
    let inner: Vec<String> = vals.iter().map(|v| r_f32(*v)).collect();
    format!("[{}]", inner.join(", "))
}

fn r_rgb(c: Rgb) -> String {
    format!("Rgb::new({}, {}, {})", c.r, c.g, c.b)
}

fn r_rgba(c: Rgba) -> String {
    format!("Rgba::new({}, {}, {}, {})", c.r, c.g, c.b, c.a)
}

fn r_shadow(s: &ShadowVal) -> String {
    format!(
        "Shadow {{ dx: {}, dy: {}, blur: {}, color: {} }}",
        r_f32(s.dx),
        r_f32(s.dy),
        r_f32(s.blur),
        r_rgba(s.color)
    )
}

fn render_palettes(out: &mut String, name: &str, label: &str, rows: &[[Rgb; 10]], bases: &[Rgb]) {
    writeln!(
        out,
        "/// {label} 14 组色板 × 10 档;行下标 = `Palette` 判别值,列下标 = 档位-1。"
    )
    .unwrap();
    writeln!(out, "pub static {name}: [[Rgb; 10]; 14] = [").unwrap();
    for (i, row) in rows.iter().enumerate() {
        let tag = if i < CHROMATIC.len() {
            format!("{}(基准 {})", CHROMATIC[i], bases[i].hex())
        } else {
            "gray(arco 手调字面值,不走算法)".to_string()
        };
        writeln!(out, "    // {tag}").unwrap();
        let cells: Vec<String> = row.iter().map(|c| r_rgb(*c)).collect();
        writeln!(out, "    [{}],", cells.join(", ")).unwrap();
    }
    out.push_str("];\n\n");
}

fn render_semantic(out: &mut String, name: &str, label: &str, s: &Semantic) {
    let arr = |xs: &[Rgba]| -> String {
        let cells: Vec<String> = xs.iter().map(|c| r_rgba(*c)).collect();
        format!("[{}]", cells.join(", "))
    };
    writeln!(out, "/// {label}模式语义色。").unwrap();
    writeln!(out, "pub static {name}: SemanticColors = SemanticColors {{").unwrap();
    writeln!(out, "    text: {},", arr(&s.text)).unwrap();
    writeln!(out, "    fill: {},", arr(&s.fill)).unwrap();
    writeln!(out, "    border_levels: {},", arr(&s.border_levels)).unwrap();
    writeln!(out, "    border: {},", r_rgba(s.border)).unwrap();
    writeln!(out, "    bg: {},", arr(&s.bg)).unwrap();
    writeln!(out, "    bg_white: {},", r_rgba(s.bg_white)).unwrap();
    writeln!(out, "    white: {},", r_rgba(s.white)).unwrap();
    writeln!(out, "    black: {},", r_rgba(s.black)).unwrap();
    out.push_str("};\n\n");
}

// ---- CSS 渲染 ---------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_css(
    palettes: &[[Rgb; 10]],
    sem: &Semantic,
    border_width: &[f64],
    size: &[f64],
    size_named: [f64; 4],
    spacing: &[f64],
    radius: [f64; 4],
    font: &FontSizes,
) -> String {
    let mut css = String::from(":root {\n");
    let px = |v: f64| -> String {
        if v == 0.0 {
            "0".into()
        } else {
            format!(
                "{}px",
                if v.fract() == 0.0 {
                    format!("{}", v as i64)
                } else {
                    format!("{v}")
                }
            )
        }
    };

    // 14 组色板
    let mut names: Vec<&str> = CHROMATIC.to_vec();
    names.push("gray");
    for (name, row) in names.iter().zip(palettes) {
        for (i, c) in row.iter().enumerate() {
            let _ = writeln!(css, "  --{name}-{}: {};", i + 1, c.hex());
        }
    }
    // 功能色接线(colors.less L314-436:primary/link→arcoblue,success→green,
    // danger→red,warning→orange)
    for (fname, idx) in [
        ("primary", 9usize),
        ("success", 6),
        ("warning", 2),
        ("danger", 0),
        ("link", 9),
    ] {
        for (i, c) in palettes[idx].iter().enumerate() {
            let _ = writeln!(css, "  --{fname}-{}: {};", i + 1, c.hex());
        }
    }
    // 语义色
    for (i, c) in sem.text.iter().enumerate() {
        let _ = writeln!(css, "  --color-text-{}: {};", i + 1, c.hex());
    }
    for (i, c) in sem.fill.iter().enumerate() {
        let _ = writeln!(css, "  --color-fill-{}: {};", i + 1, c.hex());
    }
    for (i, c) in sem.border_levels.iter().enumerate() {
        let _ = writeln!(css, "  --color-border-{}: {};", i + 1, c.hex());
    }
    let _ = writeln!(css, "  --color-border: {};", sem.border.hex());
    for (i, c) in sem.bg.iter().enumerate() {
        let _ = writeln!(css, "  --color-bg-{}: {};", i + 1, c.hex());
    }
    let _ = writeln!(css, "  --color-bg-white: {};", sem.bg_white.hex());
    let _ = writeln!(css, "  --color-white: {};", sem.white.hex());
    let _ = writeln!(css, "  --color-black: {};", sem.black.hex());
    // 标量
    for (i, w) in border_width.iter().enumerate().skip(1) {
        let _ = writeln!(css, "  --border-{i}: {};", px(*w));
    }
    for (n, r) in ["none", "small", "medium", "large"].iter().zip(radius) {
        let _ = writeln!(css, "  --border-radius-{n}: {};", px(r));
    }
    for (grp, vals) in [
        ("body", &font.body),
        ("title", &font.title),
        ("display", &font.display),
    ] {
        for (i, f) in vals.iter().enumerate() {
            let _ = writeln!(css, "  --font-size-{grp}-{}: {};", i + 1, px(*f));
        }
    }
    let _ = writeln!(css, "  --font-size-caption: {};", px(font.caption));
    for (i, s) in spacing.iter().enumerate().skip(1) {
        let _ = writeln!(css, "  --spacing-{i}: {};", px(*s));
    }
    for (i, s) in size.iter().enumerate().skip(1) {
        let _ = writeln!(css, "  --size-{i}: {};", px(*s));
    }
    for (n, s) in ["mini", "small", "default", "large"].iter().zip(size_named) {
        let _ = writeln!(css, "  --size-{n}: {};", px(s));
    }
    css.push('}');
    css
}
