//! 样式语言 → Style 字段赋值代码。
//!
//! **CSS 心智兼容层**(降低 Svelte 开发者迁移负担):
//! - 标准 CSS 属性名与本地简名都认:`background-color`/`bg`、`color`/`fg`、
//!   `border-radius`/`radius`、`flex-direction`/`direction`、`font-size`…
//! - 数值接受 `px` 后缀(`padding: 8px`);`em/rem/%` 明确报错引导(矩阵 ⏳)
//! - 颜色:`#rgb`/`#rrggbb`、`rgb()/rgba()`、常用颜色名
//! - `<style>` 块:`.类 { ... }` + `.类:hover { ... }` 伪类变体
//!   (类天然 scoped:编译进组件函数,零运行时选择器)

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;

use crate::CompileError;

/// 一个样式类:基础声明 + 可选 :hover 变体
#[derive(Clone, Default)]
pub struct ClassStyle {
    pub base: TokenStream,
    pub hover: Option<TokenStream>,
}

/// 解析 `<style>` 块:`.name { ... }` / `.name:hover { ... }` 规则列表
pub fn parse_style_block(
    source: &str,
    block: &str,
    offset: usize,
) -> Result<HashMap<String, ClassStyle>, CompileError> {
    let mut classes: HashMap<String, ClassStyle> = HashMap::new();
    let mut pos = 0usize;
    let len = block.len();
    while pos < len {
        // 跳过空白与 /* 注释 */
        loop {
            let rest = &block[pos..];
            if let Some(c) = rest.chars().next()
                && c.is_whitespace()
            {
                pos += c.len_utf8();
                continue;
            }
            if rest.starts_with("/*") {
                match rest.find("*/") {
                    Some(end) => {
                        pos += end + 2;
                        continue;
                    }
                    None => {
                        return Err(CompileError::at_offset(source, offset + pos, "style 注释未闭合"));
                    }
                }
            }
            break;
        }
        if pos >= len {
            break;
        }
        if !block[pos..].starts_with('.') {
            return Err(CompileError::at_offset(
                source,
                offset + pos,
                "style 块里应为 `.类名 { ... }` 或 `.类名:hover { ... }` 规则",
            ));
        }
        pos += 1;
        let name_start = pos;
        while pos < len {
            let c = block[pos..].chars().next().unwrap();
            if c.is_alphanumeric() || c == '_' || c == '-' {
                pos += c.len_utf8();
            } else {
                break;
            }
        }
        let name = block[name_start..pos].to_string();
        if name.is_empty() {
            return Err(CompileError::at_offset(source, offset + pos, "`.` 后应为类名"));
        }
        // 伪类
        let mut hover = false;
        if block[pos..].starts_with(':') {
            let pseudo_start = pos + 1;
            pos += 1;
            while pos < len {
                let c = block[pos..].chars().next().unwrap();
                if c.is_alphanumeric() || c == '-' {
                    pos += c.len_utf8();
                } else {
                    break;
                }
            }
            match &block[pseudo_start..pos] {
                "hover" => hover = true,
                other => {
                    return Err(CompileError::at_offset(
                        source,
                        offset + pseudo_start,
                        format!("暂支持 :hover 伪类(:active/:focus 待按压/焦点状态,见矩阵),收到 `:{other}`"),
                    ));
                }
            }
        }
        while pos < len && block[pos..].starts_with(char::is_whitespace) {
            pos += block[pos..].chars().next().unwrap().len_utf8();
        }
        if !block[pos..].starts_with('{') {
            return Err(CompileError::at_offset(
                source,
                offset + pos,
                format!("类 `.{name}` 后应为 `{{ 声明 }}`"),
            ));
        }
        pos += 1;
        let body_start = pos;
        let Some(close_rel) = block[pos..].find('}') else {
            return Err(CompileError::at_offset(source, offset + body_start, "style 规则花括号未闭合"));
        };
        let body = &block[pos..pos + close_rel];
        let setters = parse_style(source, body, offset + body_start)?;
        let entry = classes.entry(name.clone()).or_default();
        if hover {
            if entry.hover.is_some() {
                return Err(CompileError::at_offset(
                    source,
                    offset + name_start,
                    format!("类 `.{name}:hover` 重复定义"),
                ));
            }
            entry.hover = Some(setters);
        } else {
            if !entry.base.is_empty() {
                return Err(CompileError::at_offset(
                    source,
                    offset + name_start,
                    format!("类 `.{name}` 重复定义"),
                ));
            }
            entry.base = setters;
        }
        pos += close_rel + 1;
    }
    Ok(classes)
}

/// 解析声明串(`键:值; ...`),返回赋值语句流
pub fn parse_style(source: &str, style_str: &str, offset: usize) -> Result<TokenStream, CompileError> {
    let mut setters = TokenStream::new();
    let mut cursor = 0usize;
    for decl in style_str.split(';') {
        let decl_offset = offset + cursor;
        cursor += decl.len() + 1;
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((key, value)) = decl.split_once(':') else {
            return Err(CompileError::at_offset(
                source,
                decl_offset,
                format!("样式项 `{decl}` 缺少冒号(应为 `键:值`)"),
            ));
        };
        let key = key.trim();
        let value = value.trim();
        let err = |msg: String| CompileError::at_offset(source, decl_offset, msg);

        // 数值:接受裸数或 px 后缀;em/rem/% 显式引导
        let num = || -> Result<f32, CompileError> {
            let v = value.trim();
            for bad in ["em", "rem", "%", "vw", "vh", "pt"] {
                if v.ends_with(bad) && v[..v.len() - bad.len()].trim().parse::<f32>().is_ok() {
                    return Err(err(format!(
                        "单位 `{bad}` 暂不支持(见 SVELTE-SUPPORT 矩阵),请用 px 或裸数:`{key}: 8px`"
                    )));
                }
            }
            let v = v.strip_suffix("px").map(str::trim).unwrap_or(v);
            v.parse::<f32>()
                .map_err(|_| err(format!("样式 `{key}` 的值 `{value}` 不是数字")))
        };
        let stmt = match key {
            "padding" => { let v = num()?; quote! { s.padding = #v; } }
            "gap" | "row-gap" | "column-gap" => { let v = num()?; quote! { s.gap = #v; } }
            "font-size" | "font_size" => { let v = num()?; quote! { s.font_size = #v; } }
            "radius" | "corner-radius" | "border-radius" => { let v = num()?; quote! { s.corner_radius = #v; } }
            "opacity" => { let v = num()?; quote! { s.opacity = #v; } }
            "width" => { let v = num()?; quote! { s.width = Some(#v); } }
            "height" => { let v = num()?; quote! { s.height = Some(#v); } }
            "direction" | "flex-direction" => match value {
                "row" => quote! { s.direction = ::sv_ui::Direction::Row; },
                "column" => quote! { s.direction = ::sv_ui::Direction::Column; },
                _ => return Err(err(format!("{key} 只能是 row|column,收到 `{value}`"))),
            },
            "bg" | "background" | "background-color" => {
                let c = color(value).map_err(&err)?;
                quote! { s.bg = Some(#c); }
            }
            "fg" | "color" => {
                let c = color(value).map_err(&err)?;
                quote! { s.fg = Some(#c); }
            }
            _ => {
                return Err(err(format!(
                    "未知样式键 `{key}`(支持 padding/gap/font-size/border-radius/opacity/width/height/flex-direction/background(-color)/color 及本地简名;margin/border 等见矩阵)"
                )));
            }
        };
        setters.extend(stmt);
    }
    Ok(setters)
}

fn color(value: &str) -> Result<TokenStream, String> {
    // 颜色名(常用子集)
    let named: Option<(u8, u8, u8, u8)> = match value.to_ascii_lowercase().as_str() {
        "white" => Some((255, 255, 255, 255)),
        "black" => Some((0, 0, 0, 255)),
        "red" => Some((255, 0, 0, 255)),
        "green" => Some((0, 128, 0, 255)),
        "blue" => Some((0, 0, 255, 255)),
        "gray" | "grey" => Some((128, 128, 128, 255)),
        "orange" => Some((255, 165, 0, 255)),
        "transparent" => Some((0, 0, 0, 0)),
        _ => None,
    };
    if let Some((r, g, b, a)) = named {
        return Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) });
    }
    // rgb() / rgba()
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() < 3 || parts.len() > 4 {
            return Err(format!("颜色 `{value}` 应为 rgb(r, g, b) 或 rgba(r, g, b, a)"));
        }
        let ch = |s: &str| s.parse::<u8>().map_err(|_| format!("颜色分量 `{s}` 不是 0-255"));
        let (r, g, b) = (ch(parts[0])?, ch(parts[1])?, ch(parts[2])?);
        let a: u8 = if parts.len() == 4 {
            let f = parts[3]
                .parse::<f32>()
                .map_err(|_| format!("alpha `{}` 不是数字", parts[3]))?;
            (f.clamp(0.0, 1.0) * 255.0) as u8
        } else {
            255
        };
        return Ok(quote! { ::sv_ui::Color::rgba(#r, #g, #b, #a) });
    }
    // 十六进制
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| format!("颜色 `{value}` 应为 #rgb/#rrggbb、rgb()/rgba() 或颜色名"))?;
    let expand = |s: &str| u8::from_str_radix(s, 16).map_err(|_| format!("颜色 `{value}` 不是合法十六进制"));
    let (r, g, b) = match hex.len() {
        3 => {
            let d = |i: usize| expand(&hex[i..i + 1]).map(|v| v * 17);
            (d(0)?, d(1)?, d(2)?)
        }
        6 => (expand(&hex[0..2])?, expand(&hex[2..4])?, expand(&hex[4..6])?),
        _ => return Err(format!("颜色 `{value}` 应为 3 位或 6 位十六进制")),
    };
    Ok(quote! { ::sv_ui::Color::rgb(#r, #g, #b) })
}
