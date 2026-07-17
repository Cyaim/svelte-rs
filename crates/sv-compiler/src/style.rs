//! `style="padding:24; gap:12; bg:#ff3e00"` 迷你样式语言 → Style 字段赋值代码
//!
//! 也负责 `<style>` 块:`.类名 { 声明; ... }` 的极简"类 + 封闭属性集"语言
//! (调研 09 §4 设计:天然 scoped——类编译进组件函数,零运行时选择器)。

use std::collections::HashMap;

use proc_macro2::TokenStream;
use quote::quote;

use crate::CompileError;

/// 解析 `<style>` 块:`.name { padding: 24; ... }` 列表 → 类名 → 赋值语句流。
/// `offset` 是 style 块内容在 .sv 里的起点(错误定位)。
pub fn parse_style_block(
    source: &str,
    block: &str,
    offset: usize,
) -> Result<HashMap<String, TokenStream>, CompileError> {
    let mut classes = HashMap::new();
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
                "style 块里应为 `.类名 { ... }` 规则",
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
        if classes.insert(name.clone(), setters).is_some() {
            return Err(CompileError::at_offset(
                source,
                offset + name_start,
                format!("类 `.{name}` 重复定义"),
            ));
        }
        pos += close_rel + 1;
    }
    Ok(classes)
}

/// 解析样式串,返回 `|s: &mut Style| { ... }` 闭包体内的赋值语句流。
/// `source`/`offset` 用于错误定位(offset 是样式串在 .sv 里的起点)。
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

        let num = || -> Result<f32, CompileError> {
            value
                .parse::<f32>()
                .map_err(|_| err(format!("样式 `{key}` 的值 `{value}` 不是数字")))
        };
        let stmt = match key {
            "padding" => { let v = num()?; quote! { s.padding = #v; } }
            "gap" => { let v = num()?; quote! { s.gap = #v; } }
            "font-size" | "font_size" => { let v = num()?; quote! { s.font_size = #v; } }
            "radius" | "corner-radius" => { let v = num()?; quote! { s.corner_radius = #v; } }
            "width" => { let v = num()?; quote! { s.width = Some(#v); } }
            "height" => { let v = num()?; quote! { s.height = Some(#v); } }
            "direction" => match value {
                "row" => quote! { s.direction = ::sv_ui::Direction::Row; },
                "column" => quote! { s.direction = ::sv_ui::Direction::Column; },
                _ => return Err(err(format!("direction 只能是 row|column,收到 `{value}`"))),
            },
            "bg" => { let c = color(value).map_err(&err)?; quote! { s.bg = Some(#c); } }
            "fg" | "color" => { let c = color(value).map_err(&err)?; quote! { s.fg = Some(#c); } }
            _ => {
                return Err(err(format!(
                    "未知样式键 `{key}`(支持 padding/gap/font-size/radius/width/height/direction/bg/fg)"
                )));
            }
        };
        setters.extend(stmt);
    }
    Ok(setters)
}

fn color(value: &str) -> Result<TokenStream, String> {
    let hex = value
        .strip_prefix('#')
        .ok_or_else(|| format!("颜色 `{value}` 应为 #rgb 或 #rrggbb"))?;
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
