//! .sv 文件切块:`<script>...</script>` 与模板部分

use crate::CompileError;

/// 源码中的一个区块(在原文件中的字节区间,用于错误定位)
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}

pub struct Sfc {
    pub script: Option<Span>,
    pub template: Span,
    pub style: Option<Span>,
}

/// 把 .sv 源码切成 script 块 + 模板 + style 块。
/// 结构约定(Svelte 型):script 在顶部,style 在底部,中间是模板。
/// v0 限制:script 里出现字符串 "</script>" 会截断(与 HTML 同款限制)。
pub fn split(source: &str) -> Result<Sfc, CompileError> {
    let (script, template_start) = match source.find("<script>") {
        Some(open) => {
            let content_start = open + "<script>".len();
            let close_rel = source[content_start..].find("</script>").ok_or_else(|| {
                CompileError::at_offset(source, open, "<script> 没有对应的 </script>")
            })?;
            let content_end = content_start + close_rel;
            let before = &source[..open];
            if !before.trim().is_empty() {
                return Err(CompileError::at_offset(
                    source,
                    0,
                    "<script> 之前不允许有模板内容(script 块应在文件顶部)",
                ));
            }
            (
                Some(Span {
                    start: content_start,
                    end: content_end,
                }),
                content_end + "</script>".len(),
            )
        }
        None => (None, 0),
    };

    let (style, template_end) = match source[template_start..].find("<style>") {
        Some(rel) => {
            let open = template_start + rel;
            let content_start = open + "<style>".len();
            let close_rel = source[content_start..].find("</style>").ok_or_else(|| {
                CompileError::at_offset(source, open, "<style> 没有对应的 </style>")
            })?;
            let content_end = content_start + close_rel;
            let after = content_end + "</style>".len();
            if !source[after..].trim().is_empty() {
                return Err(CompileError::at_offset(
                    source,
                    after,
                    "<style> 之后不允许再有模板内容(style 块应在文件底部)",
                ));
            }
            (
                Some(Span {
                    start: content_start,
                    end: content_end,
                }),
                open,
            )
        }
        None => (None, source.len()),
    };

    Ok(Sfc {
        script,
        template: Span {
            start: template_start,
            end: template_end,
        },
        style,
    })
}
