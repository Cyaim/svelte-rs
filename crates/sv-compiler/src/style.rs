//! 样式语言 → Style 字段赋值代码(**C1 批次:真 CSS 语法封闭子集**,ADR-8)。
//!
//! - 规则:`.类`(scoped)、元素类型(view/text/button/checkbox)、`:root { --x }` 变量;
//!   伪类 `:hover`/`:active`(独立规则或 CSS 嵌套 `& :pseudo` 形态)
//! - 声明:标准属性名 + 本地简名;padding/margin 1–4 值简写与 `-left` 系长手;
//!   `border: 1px solid <color>` 与 `border: none`;`cursor`
//! - 单位:px / rem(编译期 ×16)/ 裸数;em/%/vw/vh 报错引导(P2/C2)
//! - 颜色:#hex(3/4/6/8)、rgb()/rgba()(逗号与现代空格斜杠语法)、hsl()/hwb()、
//!   常用命名色、transparent、currentColor(→ 继承)
//! - `var(--x)`(含 fallback)编译期文本代入
//!
//! 一切在编译期折叠成 `Style` 字段赋值,零运行时解析。

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;

use crate::CompileError;

/// 一个样式规则组:基础 + 伪类变体
#[derive(Clone, Default)]
pub struct ClassStyle {
    pub base: TokenStream,
    pub hover: Option<TokenStream>,
    pub active: Option<TokenStream>,
}

/// `<style>` 块编译产物
#[derive(Default)]
pub struct StyleSheet {
    /// `.name` 规则(scoped 类)
    pub classes: HashMap<String, ClassStyle>,
    /// 元素类型规则(`text { }` 作用于组件内全部该类元素)
    pub elements: HashMap<String, ClassStyle>,
}

const ELEMENT_NAMES: &[&str] = &["view", "text", "button", "checkbox"];
const REM: f32 = 16.0;

struct BlockParser<'a> {
    source: &'a str,
    block: &'a str,
    offset: usize,
    pos: usize,
}

impl<'a> BlockParser<'a> {
    fn err(&self, rel: usize, msg: impl Into<String>) -> CompileError {
        CompileError::at_offset(self.source, self.offset + rel, msg)
    }

    fn skip_trivia(&mut self) -> Result<(), CompileError> {
        loop {
            let rest = &self.block[self.pos..];
            if let Some(c) = rest.chars().next()
                && c.is_whitespace()
            {
                self.pos += c.len_utf8();
                continue;
            }
            if rest.starts_with("/*") {
                match rest.find("*/") {
                    Some(end) => {
                        self.pos += end + 2;
                        continue;
                    }
                    None => return Err(self.err(self.pos, "style 注释未闭合")),
                }
            }
            return Ok(());
        }
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.block[self.pos..].chars().next() {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
        self.block[start..self.pos].to_string()
    }

    /// 读平衡花括号体(支持一层嵌套规则),返回内文
    fn read_braced(&mut self) -> Result<&'a str, CompileError> {
        assert!(self.block[self.pos..].starts_with('{'));
        self.pos += 1;
        let start = self.pos;
        let mut depth = 1usize;
        while let Some(c) = self.block[self.pos..].chars().next() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let body = &self.block[start..self.pos];
                        self.pos += 1;
                        return Ok(body);
                    }
                }
                _ => {}
            }
            self.pos += c.len_utf8();
        }
        Err(self.err(start, "style 规则花括号未闭合"))
    }
}

/// 解析 `<style>` 块
pub fn parse_style_block(
    source: &str,
    block: &str,
    offset: usize,
) -> Result<StyleSheet, CompileError> {
    let mut sheet = StyleSheet::default();
    let mut vars: HashMap<String, String> = HashMap::new();
    let mut p = BlockParser {
        source,
        block,
        offset,
        pos: 0,
    };

    // 第一遍:提取 :root { --x: v } 变量(可写在任意位置,先收齐再解析规则)
    {
        let mut q = BlockParser {
            source,
            block,
            offset,
            pos: 0,
        };
        loop {
            q.skip_trivia()?;
            if q.pos >= block.len() {
                break;
            }
            if block[q.pos..].starts_with(":root") {
                q.pos += ":root".len();
                q.skip_trivia()?;
                if !block[q.pos..].starts_with('{') {
                    return Err(q.err(q.pos, ":root 后应为 `{ --变量: 值; }`"));
                }
                let body = q.read_braced()?;
                for decl in body.split(';') {
                    let decl = decl.trim();
                    if decl.is_empty() {
                        continue;
                    }
                    let Some((k, v)) = decl.split_once(':') else {
                        return Err(q.err(q.pos, format!("`{decl}` 不是 `--变量: 值`")));
                    };
                    let k = k.trim();
                    if !k.starts_with("--") {
                        return Err(
                            q.err(q.pos, ":root 里只放 --自定义属性(普通样式写到类/元素规则)")
                        );
                    }
                    vars.insert(k.to_string(), v.trim().to_string());
                }
            } else {
                // 跳过一条完整规则
                while let Some(c) = block[q.pos..].chars().next() {
                    if c == '{' {
                        break;
                    }
                    q.pos += c.len_utf8();
                }
                if q.pos < block.len() {
                    q.read_braced()?;
                } else {
                    break;
                }
            }
        }
    }

    // 第二遍:规则
    loop {
        p.skip_trivia()?;
        if p.pos >= block.len() {
            break;
        }
        if block[p.pos..].starts_with(":root") {
            // 已在第一遍处理,跳过
            p.pos += ":root".len();
            p.skip_trivia()?;
            p.read_braced()?;
            continue;
        }
        let (is_class, name, name_rel) = if block[p.pos..].starts_with('.') {
            p.pos += 1;
            let rel = p.pos;
            (true, p.read_ident(), rel)
        } else {
            let rel = p.pos;
            (false, p.read_ident(), rel)
        };
        if name.is_empty() {
            return Err(p.err(p.pos, "应为 `.类名`、元素名或 :root 规则"));
        }
        if !is_class && !ELEMENT_NAMES.contains(&name.as_str()) {
            return Err(p.err(
                name_rel,
                format!(
                    "未知元素选择器 `{name}`(支持 {}; 类选择器加 `.` 前缀)",
                    ELEMENT_NAMES.join("/")
                ),
            ));
        }
        // 可选伪类(规则头形态 `.btn:hover`)
        let mut pseudo: Option<&str> = None;
        if block[p.pos..].starts_with(':') {
            p.pos += 1;
            let rel = p.pos;
            let ps = p.read_ident();
            pseudo = Some(match ps.as_str() {
                "hover" => "hover",
                "active" => "active",
                other => {
                    return Err(p.err(
                        rel,
                        format!(
                            "暂支持 :hover/:active(:focus/:disabled 随焦点链 C2),收到 `:{other}`"
                        ),
                    ));
                }
            });
        }
        p.skip_trivia()?;
        if !block[p.pos..].starts_with('{') {
            return Err(p.err(p.pos, format!("`{name}` 后应为 `{{ 声明 }}`")));
        }
        let body_rel = p.pos + 1;
        let body = p.read_braced()?;

        // 解析规则体:声明 + 嵌套 `&:pseudo { }`
        let (base, mut hover, mut active) =
            parse_rule_body(source, body, offset + body_rel, &vars)?;
        let entry = if is_class {
            sheet.classes.entry(name.clone()).or_default()
        } else {
            sheet.elements.entry(name.clone()).or_default()
        };
        let (base_slot, msg) = match pseudo {
            Some("hover") => {
                hover = Some(match hover {
                    // `.x:hover { &:hover {} }` 无意义,直接拒绝
                    Some(_) => return Err(p.err(body_rel, "伪类规则里不能再嵌套伪类")),
                    None => base.clone(),
                });
                (None, "hover")
            }
            Some("active") => {
                active = Some(base.clone());
                (None, "active")
            }
            _ => (Some(base), "base"),
        };
        if let Some(b) = base_slot {
            if !entry.base.is_empty() {
                return Err(p.err(name_rel, format!("`{name}` 重复定义")));
            }
            entry.base = b;
        }
        if let Some(h) = hover {
            if entry.hover.is_some() {
                return Err(p.err(name_rel, format!("`{name}:hover` 重复定义")));
            }
            entry.hover = Some(h);
        }
        if let Some(a) = active {
            if entry.active.is_some() {
                return Err(p.err(name_rel, format!("`{name}:active` 重复定义")));
            }
            entry.active = Some(a);
        }
        let _ = msg;
    }
    Ok(sheet)
}

/// 规则体:声明序列 + CSS 嵌套 `&:hover { ... }` / `&:active { ... }`
fn parse_rule_body(
    source: &str,
    body: &str,
    offset: usize,
    vars: &HashMap<String, String>,
) -> Result<(TokenStream, Option<TokenStream>, Option<TokenStream>), CompileError> {
    let mut decls = String::new();
    let mut hover = None;
    let mut active = None;
    let mut p = BlockParser {
        source,
        block: body,
        offset,
        pos: 0,
    };
    loop {
        p.skip_trivia()?;
        if p.pos >= body.len() {
            break;
        }
        if body[p.pos..].starts_with('&') {
            p.pos += 1;
            p.skip_trivia()?;
            if !body[p.pos..].starts_with(':') {
                return Err(p.err(p.pos, "嵌套规则 v0 支持 `&:hover` / `&:active`"));
            }
            p.pos += 1;
            let rel = p.pos;
            let ps = p.read_ident();
            p.skip_trivia()?;
            if !body[p.pos..].starts_with('{') {
                return Err(p.err(p.pos, "嵌套伪类后应为 `{ 声明 }`"));
            }
            let nested_rel = p.pos + 1;
            let nested = p.read_braced()?;
            let setters = parse_style_with_vars(source, nested, offset + nested_rel, vars)?;
            match ps.as_str() {
                "hover" => hover = Some(setters),
                "active" => active = Some(setters),
                other => {
                    return Err(p.err(rel, format!("嵌套伪类支持 hover/active,收到 `{other}`")));
                }
            }
        } else {
            // 声明:读到 ';' 或体尾
            let start = p.pos;
            while let Some(c) = body[p.pos..].chars().next() {
                if c == ';' {
                    break;
                }
                if c == '{' {
                    return Err(p.err(p.pos, "此处不应出现 `{`(嵌套规则要以 `&` 开头)"));
                }
                p.pos += c.len_utf8();
            }
            decls.push_str(&body[start..p.pos]);
            decls.push(';');
            if p.pos < body.len() {
                p.pos += 1; // ';'
            }
        }
    }
    let base = parse_style_with_vars(source, &decls, offset, vars)?;
    Ok((base, hover, active))
}

/// 解析声明串(内联 style="" 与简写属性也走这里;无变量环境)
pub fn parse_style(
    source: &str,
    style_str: &str,
    offset: usize,
) -> Result<TokenStream, CompileError> {
    parse_style_with_vars(source, style_str, offset, &HashMap::new())
}

fn substitute_vars(value: &str, vars: &HashMap<String, String>) -> Result<String, String> {
    let mut out = value.to_string();
    for _ in 0..8 {
        let Some(pos) = out.find("var(") else {
            return Ok(out);
        };
        let after = &out[pos + 4..];
        let Some(close) = after.find(')') else {
            return Err("var( 未闭合".into());
        };
        let inner = &after[..close];
        let (name, fallback) = match inner.split_once(',') {
            Some((n, f)) => (n.trim(), Some(f.trim())),
            None => (inner.trim(), None),
        };
        let replacement = match vars.get(name) {
            Some(v) => v.clone(),
            None => match fallback {
                Some(f) => f.to_string(),
                None => {
                    return Err(format!(
                        "未定义的变量 `{name}`(在 :root 里声明,或给 fallback)"
                    ));
                }
            },
        };
        out = format!(
            "{}{}{}",
            &out[..pos],
            replacement,
            &out[pos + 4 + close + 1..]
        );
    }
    Ok(out)
}

fn parse_style_with_vars(
    source: &str,
    style_str: &str,
    offset: usize,
    vars: &HashMap<String, String>,
) -> Result<TokenStream, CompileError> {
    let mut setters = TokenStream::new();
    let mut cursor_pos = 0usize;
    for decl in style_str.split(';') {
        let decl_offset = offset + cursor_pos;
        cursor_pos += decl.len() + 1;
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((key, raw_value)) = decl.split_once(':') else {
            return Err(CompileError::at_offset(
                source,
                decl_offset,
                format!("样式项 `{decl}` 缺少冒号(应为 `键:值`)"),
            ));
        };
        let key = key.trim();
        let err = |msg: String| CompileError::at_offset(source, decl_offset, msg);
        let value = substitute_vars(raw_value.trim(), vars).map_err(&err)?;
        let value = value.trim();

        let num = |v: &str| -> Result<f32, CompileError> { parse_length(key, v).map_err(&err) };
        let nums = || -> Result<Vec<f32>, CompileError> {
            value
                .split_whitespace()
                .map(|v| parse_length(key, v).map_err(&err))
                .collect()
        };

        let stmt = match key {
            // ---- 盒模型 ----
            "padding" | "margin" => {
                let vs = nums()?;
                let (t, r, b, l) = expand_shorthand(&vs)
                    .ok_or_else(|| err(format!("`{key}` 接受 1–4 个长度值,收到 {}", vs.len())))?;
                let field = quote::format_ident!("{key}");
                quote! { s.#field = ::sv_ui::Edges { top: #t, right: #r, bottom: #b, left: #l }; }
            }
            k if k.starts_with("padding-") || k.starts_with("margin-") => {
                let (field, side) = k.split_once('-').unwrap();
                let field = quote::format_ident!("{field}");
                let v = num(value)?;
                let side = match side {
                    "top" => quote! { top },
                    "right" => quote! { right },
                    "bottom" => quote! { bottom },
                    "left" => quote! { left },
                    _ => return Err(err(format!("未知方向 `{k}`"))),
                };
                quote! { s.#field.#side = #v; }
            }
            "border" => {
                if value == "none" {
                    quote! { s.border = None; }
                } else {
                    // `[solid] <宽度> [solid] [<颜色>]`(颜色可含空格,如 rgb(1, 2, 3);
                    // dashed/dotted P2,v0 仅实线):按顺序剥离
                    for bad in ["dashed", "dotted", "double"] {
                        if value.split_whitespace().any(|t| t == bad) {
                            return Err(err(format!("边框样式 `{bad}` 暂不支持(P2),v0 仅实线")));
                        }
                    }
                    let mut rest = value.trim();
                    if let Some(r) = rest.strip_prefix("solid") {
                        rest = r.trim_start();
                    }
                    let (w_tok, after) = match rest.find(char::is_whitespace) {
                        Some(i) => (&rest[..i], rest[i..].trim_start()),
                        None => (rest, ""),
                    };
                    let w = parse_length("border", w_tok).map_err(&err)?;
                    let mut rest = after;
                    if let Some(r) = rest.strip_prefix("solid") {
                        rest = r.trim_start();
                    }
                    let c = color(if rest.is_empty() { "black" } else { rest }).map_err(&err)?;
                    quote! { s.border = Some(::sv_ui::Border { width: #w, color: #c }); }
                }
            }
            "gap" | "row-gap" | "column-gap" => {
                let v = num(value)?;
                quote! { s.gap = #v; }
            }
            "font-size" | "font_size" => {
                let v = num(value)?;
                quote! { s.font_size = #v; }
            }
            "radius" | "corner-radius" | "border-radius" => {
                let v = num(value)?;
                quote! { s.corner_radius = #v; }
            }
            "opacity" => {
                let v: f32 = value
                    .parse()
                    .map_err(|_| err(format!("opacity `{value}` 不是数字")))?;
                quote! { s.opacity = #v; }
            }
            "width" => {
                let v = num(value)?;
                quote! { s.width = Some(#v); }
            }
            "height" => {
                let v = num(value)?;
                quote! { s.height = Some(#v); }
            }
            "direction" | "flex-direction" => match value {
                "row" => quote! { s.direction = ::sv_ui::Direction::Row; },
                "column" => quote! { s.direction = ::sv_ui::Direction::Column; },
                _ => return Err(err(format!("{key} 只能是 row|column,收到 `{value}`"))),
            },
            "cursor" => {
                let c = match value {
                    "pointer" => quote! { ::sv_ui::Cursor::Pointer },
                    "default" => quote! { ::sv_ui::Cursor::Default },
                    "text" => quote! { ::sv_ui::Cursor::Text },
                    "grab" => quote! { ::sv_ui::Cursor::Grab },
                    "not-allowed" => quote! { ::sv_ui::Cursor::NotAllowed },
                    _ => {
                        return Err(err(format!(
                            "cursor 支持 pointer/default/text/grab/not-allowed,收到 `{value}`"
                        )));
                    }
                };
                quote! { s.cursor = Some(#c); }
            }
            "bg" | "background" | "background-color" => {
                let c = color(value).map_err(&err)?;
                quote! { s.bg = Some(#c); }
            }
            "fg" | "color" => {
                if value == "currentColor" || value == "inherit" {
                    // 继承语义:清除自身值,渲染时沿父链解析
                    quote! { s.fg = None; }
                } else {
                    let c = color(value).map_err(&err)?;
                    quote! { s.fg = Some(#c); }
                }
            }
            _ => {
                return Err(err(format!(
                    "未知样式键 `{key}`(支持盒模型 padding/margin/border(-radius)、gap、font-size、\
                     opacity、width/height、flex-direction、background(-color)、color、cursor;\
                     其余见 CSS-SUPPORT 矩阵)"
                )));
            }
        };
        setters.extend(stmt);
    }
    Ok(setters)
}

/// CSS 1–4 值简写展开(上 右 下 左)
fn expand_shorthand(vs: &[f32]) -> Option<(f32, f32, f32, f32)> {
    match vs {
        [a] => Some((*a, *a, *a, *a)),
        [v, h] => Some((*v, *h, *v, *h)),
        [t, h, b] => Some((*t, *h, *b, *h)),
        [t, r, b, l] => Some((*t, *r, *b, *l)),
        _ => None,
    }
}

/// 长度值:px / rem(×16 编译期折叠)/ 裸数;其它单位给引导
fn parse_length(key: &str, v: &str) -> Result<f32, String> {
    let v = v.trim();
    if let Some(n) = v.strip_suffix("rem") {
        return n
            .trim()
            .parse::<f32>()
            .map(|x| x * REM)
            .map_err(|_| format!("`{v}` 不是数字"));
    }
    if let Some(n) = v.strip_suffix("px") {
        return n
            .trim()
            .parse::<f32>()
            .map_err(|_| format!("`{v}` 不是数字"));
    }
    for bad in ["em", "%", "vw", "vh", "vmin", "vmax", "pt", "ch"] {
        if v.ends_with(bad) && v[..v.len() - bad.len()].trim().parse::<f32>().is_ok() {
            let why = match bad {
                "em" => "需要动态字号基准(随继承管线 P2)",
                "%" | "vw" | "vh" => "需要布局系统(taffy,C2)",
                _ => "长尾单位,见 CSS-SUPPORT 矩阵",
            };
            return Err(format!(
                "单位 `{bad}` 暂不支持——{why};请用 px/rem/裸数(`{key}: 8px`)"
            ));
        }
    }
    v.parse::<f32>()
        .map_err(|_| format!("样式 `{key}` 的值 `{v}` 不是长度"))
}

// ---------------------------------------------------------------------------
// 颜色
// ---------------------------------------------------------------------------

fn named_color(name: &str) -> Option<(u8, u8, u8, u8)> {
    Some(match name {
        "white" => (255, 255, 255, 255),
        "black" => (0, 0, 0, 255),
        "red" => (255, 0, 0, 255),
        "green" => (0, 128, 0, 255),
        "blue" => (0, 0, 255, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "orange" => (255, 165, 0, 255),
        "transparent" => (0, 0, 0, 0),
        "yellow" => (255, 255, 0, 255),
        "lime" => (0, 255, 0, 255),
        "aqua" | "cyan" => (0, 255, 255, 255),
        "fuchsia" | "magenta" => (255, 0, 255, 255),
        "silver" => (192, 192, 192, 255),
        "maroon" => (128, 0, 0, 255),
        "olive" => (128, 128, 0, 255),
        "teal" => (0, 128, 128, 255),
        "navy" => (0, 0, 128, 255),
        "purple" => (128, 0, 128, 255),
        "rebeccapurple" => (102, 51, 153, 255),
        "pink" => (255, 192, 203, 255),
        "hotpink" => (255, 105, 180, 255),
        "crimson" => (220, 20, 60, 255),
        "coral" => (255, 127, 80, 255),
        "tomato" => (255, 99, 71, 255),
        "orangered" => (255, 69, 0, 255),
        "gold" => (255, 215, 0, 255),
        "khaki" => (240, 230, 140, 255),
        "indigo" => (75, 0, 130, 255),
        "violet" => (238, 130, 238, 255),
        "plum" => (221, 160, 221, 255),
        "orchid" => (218, 112, 214, 255),
        "salmon" => (250, 128, 114, 255),
        "brown" => (165, 42, 42, 255),
        "chocolate" => (210, 105, 30, 255),
        "tan" => (210, 180, 140, 255),
        "beige" => (245, 245, 220, 255),
        "ivory" => (255, 255, 240, 255),
        "snow" => (255, 250, 250, 255),
        "whitesmoke" => (245, 245, 245, 255),
        "gainsboro" => (220, 220, 220, 255),
        "lightgray" | "lightgrey" => (211, 211, 211, 255),
        "darkgray" | "darkgrey" => (169, 169, 169, 255),
        "dimgray" | "dimgrey" => (105, 105, 105, 255),
        "slategray" | "slategrey" => (112, 128, 144, 255),
        "skyblue" => (135, 206, 235, 255),
        "lightblue" => (173, 216, 230, 255),
        "steelblue" => (70, 130, 180, 255),
        "royalblue" => (65, 105, 225, 255),
        "dodgerblue" => (30, 144, 255, 255),
        "slateblue" => (106, 90, 205, 255),
        "midnightblue" => (25, 25, 112, 255),
        "turquoise" => (64, 224, 208, 255),
        "aquamarine" => (127, 255, 212, 255),
        "seagreen" => (46, 139, 87, 255),
        "forestgreen" => (34, 139, 34, 255),
        "darkgreen" => (0, 100, 0, 255),
        "springgreen" => (0, 255, 127, 255),
        "lightgreen" => (144, 238, 144, 255),
        "yellowgreen" => (154, 205, 50, 255),
        "goldenrod" => (218, 165, 32, 255),
        "darkorange" => (255, 140, 0, 255),
        "wheat" => (245, 222, 179, 255),
        "lavender" => (230, 230, 250, 255),
        "thistle" => (216, 191, 216, 255),
        _ => return None,
    })
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 360.0;
    let (s, l) = (s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// 函数式颜色参数:支持逗号语法与现代空格 + `/ alpha` 语法
fn color_args(inner: &str) -> (Vec<String>, Option<String>) {
    let (main, alpha) = match inner.split_once('/') {
        Some((m, a)) => (m, Some(a.trim().to_string())),
        None => (inner, None),
    };
    let parts: Vec<String> = if main.contains(',') {
        main.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        main.split_whitespace().map(|s| s.to_string()).collect()
    };
    if alpha.is_none() && parts.len() == 4 {
        let mut p = parts;
        let a = p.pop();
        return (p, a);
    }
    (parts, alpha)
}

fn parse_alpha(a: Option<String>) -> Result<u8, String> {
    match a {
        None => Ok(255),
        Some(s) => {
            let s = s.trim();
            let f = if let Some(pct) = s.strip_suffix('%') {
                pct.trim()
                    .parse::<f32>()
                    .map_err(|_| format!("alpha `{s}` 不是数字"))?
                    / 100.0
            } else {
                s.parse::<f32>()
                    .map_err(|_| format!("alpha `{s}` 不是数字"))?
            };
            Ok((f.clamp(0.0, 1.0) * 255.0).round() as u8)
        }
    }
}

fn color(value: &str) -> Result<TokenStream, String> {
    let value = value.trim();
    if let Some((r, g, b, a)) = named_color(&value.to_ascii_lowercase()) {
        return Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) });
    }
    // rgb()/rgba()
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let (parts, alpha) = color_args(inner);
        if parts.len() != 3 {
            return Err(format!(
                "颜色 `{value}` 应为 rgb(r g b [/ a]) 或 rgb(r, g, b[, a])"
            ));
        }
        let ch = |s: &str| {
            s.trim()
                .parse::<u8>()
                .map_err(|_| format!("颜色分量 `{s}` 不是 0-255"))
        };
        let (r, g, b) = (ch(&parts[0])?, ch(&parts[1])?, ch(&parts[2])?);
        let a = parse_alpha(alpha)?;
        return Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) });
    }
    // hsl()/hsla()
    if let Some(inner) = value
        .strip_prefix("hsla(")
        .or_else(|| value.strip_prefix("hsl("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let (parts, alpha) = color_args(inner);
        if parts.len() != 3 {
            return Err(format!("颜色 `{value}` 应为 hsl(h s% l% [/ a])"));
        }
        let h = parts[0]
            .trim_end_matches("deg")
            .parse::<f32>()
            .map_err(|_| format!("色相 `{}` 不是角度", parts[0]))?;
        let pct = |s: &str| {
            s.trim()
                .strip_suffix('%')
                .ok_or_else(|| format!("`{s}` 应为百分比"))?
                .parse::<f32>()
                .map(|v| v / 100.0)
                .map_err(|_| format!("`{s}` 不是数字"))
        };
        let (r, g, b) = hsl_to_rgb(h, pct(&parts[1])?, pct(&parts[2])?);
        let a = parse_alpha(alpha)?;
        return Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) });
    }
    // hwb()
    if let Some(inner) = value.strip_prefix("hwb(").and_then(|s| s.strip_suffix(')')) {
        let (parts, alpha) = color_args(inner);
        if parts.len() != 3 {
            return Err(format!("颜色 `{value}` 应为 hwb(h w% b%)"));
        }
        let h = parts[0]
            .trim_end_matches("deg")
            .parse::<f32>()
            .map_err(|_| format!("色相 `{}` 不是角度", parts[0]))?;
        let pct = |s: &str| {
            s.trim()
                .strip_suffix('%')
                .ok_or_else(|| format!("`{s}` 应为百分比"))?
                .parse::<f32>()
                .map(|v| (v / 100.0).clamp(0.0, 1.0))
                .map_err(|_| format!("`{s}` 不是数字"))
        };
        let (w, bl) = (pct(&parts[1])?, pct(&parts[2])?);
        let (r0, g0, b0) = hsl_to_rgb(h, 1.0, 0.5);
        let mix = |c: u8| ((c as f32 / 255.0) * (1.0 - w - bl) + w).clamp(0.0, 1.0);
        let (r, g, b) = (
            (mix(r0) * 255.0).round() as u8,
            (mix(g0) * 255.0).round() as u8,
            (mix(b0) * 255.0).round() as u8,
        );
        let a = parse_alpha(alpha)?;
        return Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) });
    }
    // 十六进制:3/4/6/8 位
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| format!("颜色 `{value}` 应为 #hex、rgb()/hsl()/hwb() 或颜色名"))?;
    let expand =
        |s: &str| u8::from_str_radix(s, 16).map_err(|_| format!("颜色 `{value}` 不是合法十六进制"));
    let (r, g, b, a) = match hex.len() {
        3 | 4 => {
            let d = |i: usize| expand(&hex[i..i + 1]).map(|v| v * 17);
            (
                d(0)?,
                d(1)?,
                d(2)?,
                if hex.len() == 4 { d(3)? } else { 255 },
            )
        }
        6 | 8 => (
            expand(&hex[0..2])?,
            expand(&hex[2..4])?,
            expand(&hex[4..6])?,
            if hex.len() == 8 {
                expand(&hex[6..8])?
            } else {
                255
            },
        ),
        _ => return Err(format!("颜色 `{value}` 应为 3/4/6/8 位十六进制")),
    };
    Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) })
}
