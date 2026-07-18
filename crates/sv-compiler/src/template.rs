//! 模板解析:原汁 Svelte 语法 → IR。
//!
//! 因为不受 Rust tokenizer 约束,这里可以支持 proc-macro 路线做不到/别扭的语法:
//! 免引号文本、`{#if}{:else if}{:else}{/if}`、`{#each list as item, i}`、`on:click={...}`。

use crate::CompileError;
use crate::sfc::Span;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Tag {
    View,
    Text,
    Button,
    /// 复选框叶子(bind:checked 双向绑定的宿主)
    Checkbox,
    /// 单行文本输入叶子(bind:value 双向绑定的宿主)
    Input,
    /// 大写开头的标签 = 组件调用,如 `<TodoItem />`
    Component(String),
}

#[derive(Debug)]
pub struct ExprSrc {
    pub src: String,
    /// 在 .sv 全文中的字节偏移(错误定位)
    pub offset: usize,
}

#[derive(Debug)]
pub enum Segment {
    Static(String),
    Expr(ExprSrc),
}

#[derive(Debug)]
pub struct Attr {
    pub name: String,
    pub value: AttrValue,
    pub offset: usize,
}

#[derive(Debug)]
pub enum AttrValue {
    Str { value: String, offset: usize },
    Expr(ExprSrc),
}

#[derive(Debug)]
pub struct Arm {
    pub cond: ExprSrc,
    pub children: Vec<Node>,
}

#[derive(Debug)]
#[allow(dead_code)] // offset 字段留给后续诊断增强
pub enum Node {
    Element {
        tag: Tag,
        attrs: Vec<Attr>,
        children: Vec<Node>,
        offset: usize,
    },
    Text {
        segments: Vec<Segment>,
    },
    If {
        arms: Vec<Arm>,
        else_children: Vec<Node>,
        offset: usize,
    },
    Each {
        list: ExprSrc,
        pat: String,
        pat_offset: usize,
        index: Option<String>,
        /// `(key)` keyed 形态:按 key 复用行作用域
        key: Option<ExprSrc>,
        children: Vec<Node>,
        /// `{:else}` 空状态
        else_children: Vec<Node>,
        offset: usize,
    },
    /// `{#key expr} ... {/key}`:key 变化时销毁重建
    Key {
        key: ExprSrc,
        children: Vec<Node>,
        offset: usize,
    },
    /// `{@const name = expr}`:块级 derived
    Const {
        name: String,
        expr: ExprSrc,
        offset: usize,
    },
    /// `{#snippet name(param: Type, ...)} ... {/snippet}`:模板级可复用闭包
    Snippet {
        name: String,
        /// (参数名, 类型源码)
        params: Vec<(String, String)>,
        children: Vec<Node>,
        offset: usize,
    },
    /// `{@render name(args...)}`
    Render {
        call: ExprSrc,
        offset: usize,
    },
    /// `{@debug a, b}`:依赖变化时 Debug 打印
    Debug {
        args: Vec<ExprSrc>,
        offset: usize,
    },
    /// `{#await fut}...{:then v}...{:catch e}...{/await}`
    Await {
        fut: ExprSrc,
        pending: Vec<Node>,
        then_pat: Option<String>,
        then_children: Vec<Node>,
        catch_pat: Option<String>,
        catch_children: Vec<Node>,
        offset: usize,
    },
}

pub fn parse(source: &str, span: &Span) -> Result<Vec<Node>, CompileError> {
    let mut p = Parser {
        source,
        pos: span.start,
        end: span.end,
    };
    let nodes = p.parse_nodes(&[])?;
    p.skip_ws();
    if p.pos < p.end {
        return Err(p.err(p.pos, "多余的内容(可能是未匹配的闭合标签或块标记)"));
    }
    Ok(nodes)
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
    end: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.source[self.pos..self.end]
    }

    fn cur(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn starts(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    /// 块关键字匹配:关键字后必须是空白或 `}`,防止 `{#iffy}` 被当成 `{#if fy}`
    fn starts_block(&self, kw: &str) -> bool {
        self.starts(kw)
            && self.source[self.pos + kw.len()..self.end]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace() || c == '}')
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.starts(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str, what: &str) -> Result<(), CompileError> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(self.err(self.pos, format!("此处应为 `{s}`({what})")))
        }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.cur() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn err(&self, offset: usize, msg: impl Into<String>) -> CompileError {
        CompileError::at_offset(self.source, offset, msg)
    }

    fn at_any(&self, terms: &[&str]) -> bool {
        terms.iter().any(|t| self.starts(t))
    }

    fn parse_nodes(&mut self, terms: &[&str]) -> Result<Vec<Node>, CompileError> {
        let mut nodes = Vec::new();
        let mut segments: Vec<Segment> = Vec::new();
        loop {
            if self.pos >= self.end || self.at_any(terms) {
                break;
            }
            if self.starts("<!--") {
                // 模板注释:跳过,不进产物
                let off = self.pos;
                match self.rest().find("-->") {
                    Some(rel) => self.pos += rel + 3,
                    None => return Err(self.err(off, "模板注释未闭合(缺 -->)")),
                }
                continue;
            }
            if self.starts("<svelte:options") || self.starts("<sv:options") {
                // 选项元素:桌面场景无对应语义,接受语法并忽略(自闭合形态)
                let off = self.pos;
                match self.rest().find("/>") {
                    Some(rel) => self.pos += rel + 2,
                    None => return Err(self.err(off, "<svelte:options> 应自闭合(... />)")),
                }
                continue;
            }
            if self.starts("</") {
                return Err(self.err(self.pos, "未匹配的闭合标签"));
            }
            if self.starts("<") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_element()?);
            } else if self.starts_block("{#if") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_if()?);
            } else if self.starts_block("{#each") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_each()?);
            } else if self.starts_block("{#key") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_key()?);
            } else if self.starts_block("{#snippet") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_snippet()?);
            } else if self.starts_block("{#await") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_await()?);
            } else if self.starts_block("{@const") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_const()?);
            } else if self.starts_block("{@render") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_render()?);
            } else if self.starts_block("{@debug") {
                flush_segments(&mut segments, &mut nodes);
                nodes.push(self.parse_debug()?);
            } else if self.starts("{@") {
                return Err(self.err(
                    self.pos,
                    "未知 {@...} 标记(支持 {@const}/{@render}/{@debug})",
                ));
            } else if self.starts("{#") {
                return Err(self.err(self.pos, "未知块类型(支持 {#if}/{#each}/{#key}/{#snippet})"));
            } else if self.starts("{:") || self.starts("{/") {
                return Err(self.err(self.pos, "意外的块标记(没有对应的开启块)"));
            } else if self.starts("{") {
                let off = self.pos + 1;
                self.pos += 1;
                let src = self.read_balanced()?;
                self.expect("}", "插值表达式结束")?;
                segments.push(Segment::Expr(ExprSrc { src, offset: off }));
            } else {
                let start = self.pos;
                while let Some(c) = self.cur() {
                    if c == '<' || c == '{' {
                        break;
                    }
                    self.pos += c.len_utf8();
                }
                segments.push(Segment::Static(self.source[start..self.pos].to_string()));
            }
        }
        flush_segments(&mut segments, &mut nodes);
        Ok(nodes)
    }

    fn parse_element(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.expect("<", "标签开始")?;
        let name = self.read_name();
        let tag = match name.as_str() {
            "view" => Tag::View,
            "text" => Tag::Text,
            "button" => Tag::Button,
            "checkbox" => Tag::Checkbox,
            "input" => Tag::Input,
            "" => return Err(self.err(off, "`<` 后应为标签名")),
            other if other.chars().next().unwrap().is_ascii_uppercase() => {
                Tag::Component(other.to_string())
            }
            other => {
                return Err(self.err(
                    off + 1,
                    format!(
                        "未知标签 `{other}`(内置 view/text/button/checkbox/input;组件用大写开头)"
                    ),
                ));
            }
        };
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            if self.eat("/>") {
                return Ok(Node::Element {
                    tag,
                    attrs,
                    children: Vec::new(),
                    offset: off,
                });
            }
            if self.eat(">") {
                break;
            }
            if self.pos >= self.end {
                return Err(self.err(off, format!("`<{name}>` 标签未闭合")));
            }
            attrs.push(self.parse_attr()?);
        }
        let close = format!("</{name}>");
        let children = self.parse_nodes(std::slice::from_ref(&close.as_str()))?;
        if !self.eat(&close) {
            return Err(self.err(off, format!("`<{name}>` 缺少闭合标签 `{close}`")));
        }
        match tag {
            Tag::View => Ok(Node::Element {
                tag,
                attrs,
                children,
                offset: off,
            }),
            // 组件 children:非自闭合形态的子内容编译成隐式 children snippet
            Tag::Component(_) => Ok(Node::Element {
                tag,
                attrs,
                children,
                offset: off,
            }),
            Tag::Checkbox | Tag::Input => {
                if !children.is_empty() {
                    return Err(self.err(off, format!("`<{name}>` 是叶子元素,请自闭合")));
                }
                Ok(Node::Element {
                    tag,
                    attrs,
                    children,
                    offset: off,
                })
            }
            Tag::Text | Tag::Button => {
                // 叶子标签:内容折叠成一段文本
                let mut segments = Vec::new();
                for child in children {
                    match child {
                        Node::Text { segments: s } => segments.extend(s),
                        _ => {
                            return Err(self.err(
                                off,
                                format!("`<{name}>` 内只能是文本与插值,不能嵌元素或块"),
                            ));
                        }
                    }
                }
                Ok(Node::Element {
                    tag,
                    attrs,
                    children: vec![Node::Text { segments }],
                    offset: off,
                })
            }
        }
    }

    fn parse_attr(&mut self) -> Result<Attr, CompileError> {
        let off = self.pos;
        // {@attach 表达式}:附着闭包(挂载时以 (doc, 节点) 调用,响应式重跑)
        if self.starts("{@attach") {
            self.pos += "{@attach".len();
            self.skip_ws();
            let voff = self.pos;
            let src = self.read_balanced()?;
            self.expect("}", "{@attach} 结束")?;
            if src.trim().is_empty() {
                return Err(self.err(off, "{@attach} 需要闭包表达式"));
            }
            return Ok(Attr {
                name: "@attach".to_string(),
                value: AttrValue::Expr(ExprSrc { src, offset: voff }),
                offset: off,
            });
        }
        // 简写 {name}:等价 name={name}
        if self.starts("{") {
            self.pos += 1;
            self.skip_ws();
            let name = self.read_name();
            if name.is_empty() {
                return Err(self.err(off, "简写属性 {名字} 里应为标识符"));
            }
            self.skip_ws();
            self.expect("}", "简写属性结束")?;
            return Ok(Attr {
                name: name.clone(),
                value: AttrValue::Expr(ExprSrc {
                    src: name,
                    offset: off + 1,
                }),
                offset: off,
            });
        }
        let name = self.read_attr_name();
        if name.is_empty() {
            return Err(self.err(off, "无法解析属性名"));
        }
        self.skip_ws();
        // 无值属性(如 `transition:fade`):值记为空字符串
        if !self.starts("=") {
            return Ok(Attr {
                name,
                value: AttrValue::Str {
                    value: String::new(),
                    offset: off,
                },
                offset: off,
            });
        }
        self.expect("=", &format!("属性 `{name}` 的值"))?;
        self.skip_ws();
        let value = if self.starts("\"") {
            let voff = self.pos + 1;
            self.pos += 1;
            let start = self.pos;
            while let Some(c) = self.cur() {
                if c == '"' {
                    break;
                }
                self.pos += c.len_utf8();
            }
            let value = self.source[start..self.pos].to_string();
            self.expect("\"", "字符串属性值结束")?;
            AttrValue::Str {
                value,
                offset: voff,
            }
        } else if self.starts("{") {
            let voff = self.pos + 1;
            self.pos += 1;
            let src = self.read_balanced()?;
            self.expect("}", "表达式属性值结束")?;
            AttrValue::Expr(ExprSrc { src, offset: voff })
        } else {
            return Err(self.err(
                self.pos,
                format!("属性 `{name}` 的值应为 \"...\" 或 {{...}}"),
            ));
        };
        Ok(Attr {
            name,
            value,
            offset: off,
        })
    }

    fn parse_if(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{#if".len();
        let cond_off = self.pos;
        let cond = self.read_balanced()?;
        if cond.trim().is_empty() {
            return Err(self.err(off, "{#if} 缺少条件表达式"));
        }
        self.expect("}", "{#if 条件} 结束")?;
        // `</` 也作为终止符:块里遇到的闭合标签必然属于祖先元素,
        // 交回上层处理;若 {/if} 缺失,下面会在 {#if} 处报错
        let mut arms = vec![Arm {
            cond: ExprSrc {
                src: cond,
                offset: cond_off,
            },
            children: self.parse_nodes(&["{:else", "{/if}", "</"])?,
        }];
        let mut else_children = Vec::new();
        loop {
            if self.starts("{:else") {
                self.pos += "{:else".len();
                self.skip_ws();
                if self.eat("if") {
                    let coff = self.pos;
                    let cond = self.read_balanced()?;
                    self.expect("}", "{:else if 条件} 结束")?;
                    arms.push(Arm {
                        cond: ExprSrc {
                            src: cond,
                            offset: coff,
                        },
                        children: self.parse_nodes(&["{:else", "{/if}", "</"])?,
                    });
                } else {
                    self.expect("}", "{:else} 结束")?;
                    else_children = self.parse_nodes(&["{/if}", "</"])?;
                    break;
                }
            } else {
                break;
            }
        }
        if !self.eat("{/if}") {
            return Err(self.err(off, "{#if} 没有对应的 {/if}"));
        }
        Ok(Node::If {
            arms,
            else_children,
            offset: off,
        })
    }

    fn parse_each(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{#each".len();
        let header_off = self.pos;
        let header = self.read_balanced()?;
        self.expect("}", "{#each ...} 结束")?;

        // 在顶层深度找最后一个独立 `as`(列表表达式里的 `x as f32` 转型也在顶层,
        // 取最后一个可以让 `expr as pat` 的常见写法工作;病态嵌套 v0 不管)
        // `{#each expr}`(省略 as):不绑定项,按长度渲染 N 次
        let Some(as_pos) = find_top_level_as(&header) else {
            if header.trim().is_empty() {
                return Err(self.err(off, "{#each} 缺少列表表达式"));
            }
            let children = self.parse_nodes(&["{:else", "{/each}", "</"])?;
            let mut else_children = Vec::new();
            if self.starts("{:else") {
                self.pos += "{:else".len();
                self.skip_ws();
                self.expect("}", "{:else} 结束")?;
                else_children = self.parse_nodes(&["{/each}", "</"])?;
            }
            if !self.eat("{/each}") {
                return Err(self.err(off, "{#each} 没有对应的 {/each}"));
            }
            return Ok(Node::Each {
                list: ExprSrc {
                    src: header.trim().to_string(),
                    offset: header_off,
                },
                pat: "_".to_string(),
                pat_offset: header_off,
                index: None,
                key: None,
                children,
                else_children,
                offset: off,
            });
        };
        let list_src = header[..as_pos].trim().to_string();
        let mut binding = &header[as_pos + 2..];

        // keyed 形态:绑定尾部的顶层 `(key)`
        let mut key = None;
        if let Some(paren) = find_last_top_level_open_paren(binding) {
            let after_close = binding[paren..].rfind(')').map(|r| paren + r + 1);
            if let Some(end) = after_close
                && binding[end..].trim().is_empty()
            {
                let key_src = binding[paren + 1..end - 1].trim().to_string();
                if key_src.is_empty() {
                    return Err(self.err(off, "{#each} 的 (key) 不能为空"));
                }
                key = Some(ExprSrc {
                    src: key_src,
                    offset: header_off + as_pos + 2 + paren + 1,
                });
                binding = &binding[..paren];
            }
        }

        let (pat_src, index) = match find_top_level(binding, ",") {
            Some(c) => {
                let idx = binding[c + 1..].trim();
                if idx.is_empty() || !idx.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
                    return Err(self.err(off, format!("{{#each}} 的索引名 `{idx}` 不是合法标识符")));
                }
                (binding[..c].trim().to_string(), Some(idx.to_string()))
            }
            None => (binding.trim().to_string(), None),
        };
        if list_src.is_empty() || pat_src.is_empty() {
            return Err(self.err(off, "{#each 列表 as 模式} 两侧都不能为空"));
        }
        if key.is_some() && index.is_some() {
            return Err(self.err(off, "keyed {#each} 暂不支持索引绑定(行复用时索引会失真)"));
        }

        let children = self.parse_nodes(&["{:else", "{/each}", "</"])?;
        let mut else_children = Vec::new();
        if self.starts("{:else") {
            self.pos += "{:else".len();
            self.skip_ws();
            self.expect("}", "{:else} 结束({#each} 不支持 else if)")?;
            else_children = self.parse_nodes(&["{/each}", "</"])?;
        }
        if !self.eat("{/each}") {
            return Err(self.err(off, "{#each} 没有对应的 {/each}"));
        }
        Ok(Node::Each {
            list: ExprSrc {
                src: list_src,
                offset: header_off,
            },
            pat: pat_src,
            pat_offset: header_off + as_pos + 2,
            index,
            key,
            children,
            else_children,
            offset: off,
        })
    }

    fn parse_snippet(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{#snippet".len();
        self.skip_ws();
        let name = self.read_name();
        if name.is_empty() || name.contains('-') {
            return Err(self.err(off, "{#snippet} 需要合法的名字"));
        }
        self.skip_ws();
        self.expect("(", "{#snippet 名字(参数)} 参数表")?;
        // 读到匹配的 ')'(类型里可能有嵌套括号,如 Vec<(i32, i32)>)
        let params_start = self.pos;
        let mut depth = 0usize;
        loop {
            let Some(c) = self.cur() else {
                return Err(self.err(off, "{#snippet} 参数表未闭合"));
            };
            match c {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.pos += c.len_utf8();
        }
        let params_src = self.source[params_start..self.pos].to_string();
        self.pos += 1; // ')'
        self.skip_ws();
        self.expect("}", "{#snippet ...} 结束")?;

        let mut params = Vec::new();
        for part in split_top_level(&params_src, ',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((pname, pty)) = part.split_once(':') else {
                return Err(self.err(
                    off,
                    format!("snippet 参数 `{part}` 缺少类型标注(Rust 需要,写 `名字: 类型`)"),
                ));
            };
            params.push((pname.trim().to_string(), pty.trim().to_string()));
        }

        let children = self.parse_nodes(&["{/snippet}", "</"])?;
        if !self.eat("{/snippet}") {
            return Err(self.err(off, "{#snippet} 没有对应的 {/snippet}"));
        }
        Ok(Node::Snippet {
            name,
            params,
            children,
            offset: off,
        })
    }

    fn parse_await(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{#await".len();
        self.skip_ws();
        let fut_off = self.pos;
        let fut = self.read_balanced()?;
        if fut.trim().is_empty() {
            return Err(self.err(off, "{#await} 缺少 Future 表达式"));
        }
        self.expect("}", "{#await 表达式} 结束")?;
        let pending = self.parse_nodes(&["{:then", "{:catch", "{/await}", "</"])?;

        let read_arm =
            |p: &mut Self, kw: &str| -> Result<(Option<String>, Vec<Node>), CompileError> {
                p.pos += kw.len();
                p.skip_ws();
                let pat = {
                    let name = p.read_name();
                    if name.is_empty() { None } else { Some(name) }
                };
                p.skip_ws();
                p.expect("}", "分支头结束")?;
                let children = p.parse_nodes(&["{:then", "{:catch", "{/await}", "</"])?;
                Ok((pat, children))
            };

        let (mut then_pat, mut then_children) = (None, Vec::new());
        let (mut catch_pat, mut catch_children) = (None, Vec::new());
        let mut has_then = false;
        let mut has_catch = false;
        while self.starts("{:then") || self.starts("{:catch") {
            if self.starts("{:then") {
                if has_then {
                    return Err(self.err(self.pos, "{#await} 只能有一个 {:then}"));
                }
                (then_pat, then_children) = read_arm(self, "{:then")?;
                has_then = true;
            } else {
                if has_catch {
                    return Err(self.err(self.pos, "{#await} 只能有一个 {:catch}"));
                }
                (catch_pat, catch_children) = read_arm(self, "{:catch")?;
                has_catch = true;
            }
        }
        if !self.eat("{/await}") {
            return Err(self.err(off, "{#await} 没有对应的 {/await}"));
        }
        if !has_then {
            return Err(self.err(off, "{#await} 需要 {:then} 分支(v0 不支持纯 pending 形态)"));
        }
        Ok(Node::Await {
            fut: ExprSrc {
                src: fut,
                offset: fut_off,
            },
            pending,
            then_pat,
            then_children,
            catch_pat,
            catch_children,
            offset: off,
        })
    }

    fn parse_render(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{@render".len();
        self.skip_ws();
        let call_off = self.pos;
        let src = self.read_balanced()?;
        if src.trim().is_empty() {
            return Err(self.err(off, "{@render} 需要 snippet 调用,如 {@render row(item)}"));
        }
        self.expect("}", "{@render} 结束")?;
        Ok(Node::Render {
            call: ExprSrc {
                src,
                offset: call_off,
            },
            offset: off,
        })
    }

    fn parse_debug(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{@debug".len();
        self.skip_ws();
        let args_off = self.pos;
        let src = self.read_balanced()?;
        self.expect("}", "{@debug} 结束")?;
        let args = split_top_level(&src, ',')
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| ExprSrc {
                src: s,
                offset: args_off,
            })
            .collect::<Vec<_>>();
        if args.is_empty() {
            return Err(self.err(off, "{@debug} 需要至少一个表达式"));
        }
        Ok(Node::Debug { args, offset: off })
    }

    fn parse_key(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{#key".len();
        let key_off = self.pos;
        let key = self.read_balanced()?;
        if key.trim().is_empty() {
            return Err(self.err(off, "{#key} 缺少表达式"));
        }
        self.expect("}", "{#key 表达式} 结束")?;
        let children = self.parse_nodes(&["{/key}", "</"])?;
        if !self.eat("{/key}") {
            return Err(self.err(off, "{#key} 没有对应的 {/key}"));
        }
        Ok(Node::Key {
            key: ExprSrc {
                src: key,
                offset: key_off,
            },
            children,
            offset: off,
        })
    }

    fn parse_const(&mut self) -> Result<Node, CompileError> {
        let off = self.pos;
        self.pos += "{@const".len();
        self.skip_ws();
        let name = self.read_name();
        if name.is_empty() || name.chars().next().unwrap().is_ascii_digit() || name.contains('-') {
            return Err(self.err(off, "{@const} 需要合法的变量名"));
        }
        self.skip_ws();
        self.expect("=", "{@const 名字 = 表达式}")?;
        let expr_off = self.pos;
        let expr = self.read_balanced()?;
        if expr.trim().is_empty() {
            return Err(self.err(off, "{@const} 缺少表达式"));
        }
        self.expect("}", "{@const} 结束")?;
        Ok(Node::Const {
            name,
            expr: ExprSrc {
                src: expr,
                offset: expr_off,
            },
            offset: off,
        })
    }

    /// 读到深度 0 的 `}` 为止(不消费)。跳过字符串/字符字面量与 `//`、`/* */` 注释,
    /// 这些里面的 `{`/`}` 不参与配平。返回原文(不 trim,调用方按需 trim,
    /// 保证 offset 与源文件对齐)
    fn read_balanced(&mut self) -> Result<String, CompileError> {
        let start = self.pos;
        let mut depth = 0usize;
        while self.pos < self.end {
            let c = self.cur().unwrap();
            match c {
                '"' => {
                    self.pos += 1;
                    while let Some(sc) = self.cur() {
                        self.pos += sc.len_utf8();
                        match sc {
                            '\\' => {
                                if let Some(esc) = self.cur() {
                                    self.pos += esc.len_utf8();
                                }
                            }
                            '"' => break,
                            _ => {}
                        }
                    }
                    continue;
                }
                // 字符字面量 'x' / '\n' / '}';生命周期 'a 不消费(后面没有配对引号)
                '\'' => {
                    let rest = &self.source[self.pos + 1..self.end];
                    let mut chars = rest.chars();
                    let consumed = match chars.next() {
                        Some('\\') => {
                            // '\x' 转义:跳过转义符后找闭合引号
                            let mut n = 2; // ' + \
                            for esc in chars {
                                n += esc.len_utf8();
                                if esc == '\'' {
                                    break;
                                }
                            }
                            Some(n)
                        }
                        Some(ch) => {
                            // 'c' 形态:第三个字符必须是闭合引号,否则当作生命周期
                            if rest[ch.len_utf8()..].starts_with('\'') {
                                Some(1 + ch.len_utf8() + 1)
                            } else {
                                None
                            }
                        }
                        None => None,
                    };
                    match consumed {
                        Some(n) => {
                            self.pos += n;
                            continue;
                        }
                        None => {
                            self.pos += 1; // 生命周期的 ',正常前进
                            continue;
                        }
                    }
                }
                '/' if self.starts("//") => {
                    while let Some(cc) = self.cur() {
                        if cc == '\n' {
                            break;
                        }
                        self.pos += cc.len_utf8();
                    }
                    continue;
                }
                '/' if self.starts("/*") => {
                    let mut level = 0usize;
                    while self.pos < self.end {
                        if self.starts("/*") {
                            level += 1;
                            self.pos += 2;
                        } else if self.starts("*/") {
                            level -= 1;
                            self.pos += 2;
                            if level == 0 {
                                break;
                            }
                        } else {
                            self.pos += self.cur().unwrap().len_utf8();
                        }
                    }
                    continue;
                }
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        return Ok(self.source[start..self.pos].to_string());
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.pos += c.len_utf8();
        }
        Err(self.err(start, "花括号未闭合"))
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.cur() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.source[start..self.pos].to_string()
    }

    fn read_attr_name(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.cur() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.source[start..self.pos].to_string()
    }
}

/// 顶层深度(不在括号/花括号/方括号/字符串内)查找 needle 的最后一次出现。
/// 按 char 边界扫描(字节切片会在多字节 UTF-8 上 panic)
fn find_top_level(s: &str, needle: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut skip_next = false;
    let mut last = None;
    for (i, c) in s.char_indices() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => skip_next = true,
                '"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {
                    if depth == 0 && s[i..].starts_with(needle) {
                        last = Some(i);
                    }
                }
            }
        }
    }
    last
}

/// 顶层深度找最后一个 `(` 的位置(keyed each 的 `(key)` 提取)
fn find_last_top_level_open_paren(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut skip_next = false;
    let mut last = None;
    for (i, c) in s.char_indices() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => skip_next = true,
                '"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '(' => {
                    if depth == 0 {
                        last = Some(i);
                    }
                    depth += 1;
                }
                '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }
    }
    last
}

/// 顶层深度按分隔符切分(参数表/表达式列表)
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut skip_next = false;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => skip_next = true,
                '"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' | '<' => depth += 1,
                ')' | ']' | '}' | '>' => depth -= 1,
                _ if c == sep && depth == 0 => {
                    parts.push(&s[start..i]);
                    start = i + c.len_utf8();
                }
                _ => {}
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

/// 顶层深度查找独立的 `as` 关键字(前后是任意空白,不只单个空格),返回 `a` 的字节位置
fn find_top_level_as(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut skip_next = false;
    let mut last = None;
    let mut prev_ws = true;
    for (i, c) in s.char_indices() {
        if skip_next {
            skip_next = false;
            prev_ws = false;
            continue;
        }
        if in_str {
            match c {
                '\\' => skip_next = true,
                '"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {
                    if depth == 0 && prev_ws && s[i..].starts_with("as") {
                        let after = s[i + 2..].chars().next();
                        if after.is_some_and(|c| c.is_whitespace()) {
                            last = Some(i);
                        }
                    }
                }
            }
        }
        prev_ws = c.is_whitespace();
    }
    last
}

/// 文本段落整理:去掉元素间的纯缩进/换行,段内换行折叠为单个空格
fn flush_segments(segments: &mut Vec<Segment>, nodes: &mut Vec<Node>) {
    if segments.is_empty() {
        return;
    }
    let mut segs = std::mem::take(segments);
    for s in segs.iter_mut() {
        if let Segment::Static(text) = s {
            let mut out = String::new();
            let mut ws_buf = String::new();
            for c in text.chars() {
                if c.is_whitespace() {
                    ws_buf.push(c);
                } else {
                    if !ws_buf.is_empty() {
                        out.push(' ');
                        ws_buf.clear();
                    }
                    out.push(c);
                }
            }
            if !ws_buf.is_empty() {
                out.push(' ');
            }
            *text = out;
        }
    }
    // 首尾修剪
    if let Some(Segment::Static(t)) = segs.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(Segment::Static(t)) = segs.last_mut() {
        *t = t.trim_end().to_string();
    }
    segs.retain(|s| !matches!(s, Segment::Static(t) if t.is_empty()));
    if !segs.is_empty() {
        nodes.push(Node::Text { segments: segs });
    }
}
