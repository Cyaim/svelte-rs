//! `.svelte` 语言服务器(LSP)——最小可用版(MVP)。
//!
//! # 它做什么
//!
//! 编辑器每次打开 / 改动一个 `.svelte` 文件,就把全文交给 `sv_compiler::compile`
//! 编译一遍;编译前端报的错(未知标签、非法属性、runes 改写失败、样式语法……)
//! 原地变成 `textDocument/publishDiagnostics`,在编辑器里画波浪线。
//!
//! # 它**不**做什么(与 `sv check` 的分工)
//!
//! - 不跑 `cargo check`:那是 `sv check`(二进制)的活,拿 rustc 的类型错并按
//!   source map 搬回 `.svelte`。LSP 这层只做**编译前端**能立刻算出的错 —— 快、
//!   不落盘、每次击键都能重算,是编辑期最高频的一档。
//! - 不做补全 / 跳转 / hover:那些要真嵌套包络与符号表(`lsp-spike.md` §3),
//!   本 MVP 不碰。textDocumentSync 用 Full(每次传全文),省掉增量同步的状态机。
//!
//! # 为什么零外部依赖
//!
//! 全仓的纪律是依赖面尽量干净(sv-compiler 自带手写 JSON、sv-pag 零依赖)。
//! LSP 协议就是 `Content-Length` 分帧 + JSON-RPC,两者都能手写;协议解析用
//! `sv_compiler::check::json`,序列化在本文件里(~40 行)。不引 tower-lsp/tokio。
//!
//! # 可测性
//!
//! 协议处理是**纯函数** [`Server::handle`]:喂一个 JSON-RPC `Value`,拿回若干条
//! 待发的 JSON 文本。传输层(stdio 分帧)在 `main.rs`,薄到不需要测。

use std::collections::HashMap;

use sv_compiler::check::json::Value;

/// 把 [`Value`] 序列化成紧凑 JSON(LSP 传输用)。
///
/// 独立实现而不复用解析器:`json` 模块只解析不序列化。字符串按 JSON 规范转义;
/// 数字若是整数就不带 `.0`(LSP 的 line/character 是整数,`3.0` 虽合法但难看)。
pub fn serialize(v: &Value) -> String {
    let mut s = String::new();
    write_value(&mut s, v);
    s
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                out.push_str(&(*n as i64).to_string());
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::Str(s) => write_string(out, s),
        Value::Arr(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, e);
            }
            out.push(']');
        }
        Value::Obj(kv) => {
            out.push('{');
            for (i, (k, val)) in kv.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, val);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// 小构造器:让下面的消息拼装读起来像 JSON 字面量
fn obj(fields: Vec<(&str, Value)>) -> Value {
    Value::Obj(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
fn num(n: usize) -> Value {
    Value::Num(n as f64)
}
fn str_(s: &str) -> Value {
    Value::Str(s.to_string())
}

/// 语言服务器状态:打开的文档(uri → 全文)。
#[derive(Default)]
pub struct Server {
    docs: HashMap<String, String>,
    /// 收到 `shutdown` 后置位;`exit` 通知据此决定退出码(LSP 规范)
    shutdown: bool,
}

/// [`Server::handle`] 的一个副作用出口。`Send` 是给客户端的一条完整 JSON-RPC
/// 消息(响应或通知);`Exit` 让传输层结束循环。
#[derive(Debug, PartialEq)]
pub enum Out {
    Send(String),
    Exit(i32),
}

impl Server {
    pub fn new() -> Server {
        Server::default()
    }

    /// 处理一条 JSON-RPC 消息,返回若干条待发消息 / 退出信号。**纯函数式**:
    /// 除了 `self.docs` 的增删,没有别的副作用(不碰 IO)。
    pub fn handle(&mut self, msg: &Value) -> Vec<Out> {
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();
        match method {
            "initialize" => vec![self.respond(id, self.capabilities())],
            "initialized" => vec![], // 通知,无需回应
            "shutdown" => {
                self.shutdown = true;
                vec![self.respond(id, Value::Null)]
            }
            "exit" => vec![Out::Exit(if self.shutdown { 0 } else { 1 })],
            "textDocument/didOpen" => {
                let td = msg.get("params").and_then(|p| p.get("textDocument"));
                let uri = td.and_then(|t| t.get("uri")).and_then(|u| u.as_str());
                let text = td.and_then(|t| t.get("text")).and_then(|t| t.as_str());
                if let (Some(uri), Some(text)) = (uri, text) {
                    self.docs.insert(uri.to_string(), text.to_string());
                    vec![self.diagnostics(uri, text)]
                } else {
                    vec![]
                }
            }
            "textDocument/didChange" => {
                let params = msg.get("params");
                let uri = params
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str());
                // Full 同步:contentChanges 最后一项的 text 是全文
                let text = params
                    .and_then(|p| p.get("contentChanges"))
                    .map(|c| c.as_arr())
                    .and_then(|a| a.last())
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str());
                if let (Some(uri), Some(text)) = (uri, text) {
                    self.docs.insert(uri.to_string(), text.to_string());
                    vec![self.diagnostics(uri, text)]
                } else {
                    vec![]
                }
            }
            "textDocument/didClose" => {
                let uri = msg
                    .get("params")
                    .and_then(|p| p.get("textDocument"))
                    .and_then(|t| t.get("uri"))
                    .and_then(|u| u.as_str());
                if let Some(uri) = uri {
                    self.docs.remove(uri);
                    // 关闭时清空该文件的诊断(否则波浪线残留)
                    vec![self.publish(uri, Vec::new())]
                } else {
                    vec![]
                }
            }
            // 未知请求(有 id):回一个空 result,别让客户端干等
            _ if id.is_some() => vec![self.respond(id, Value::Null)],
            _ => vec![],
        }
    }

    fn capabilities(&self) -> Value {
        obj(vec![
            (
                "capabilities",
                obj(vec![
                    // 1 = Full:每次改动传全文(MVP 不做增量同步)
                    ("textDocumentSync", num(1)),
                ]),
            ),
            (
                "serverInfo",
                obj(vec![
                    ("name", str_("sv-lsp")),
                    ("version", str_(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ])
    }

    fn respond(&self, id: Option<Value>, result: Value) -> Out {
        Out::Send(serialize(&obj(vec![
            ("jsonrpc", str_("2.0")),
            ("id", id.unwrap_or(Value::Null)),
            ("result", result),
        ])))
    }

    /// 编译一遍 `.svelte`,把编译前端的错变成一条 publishDiagnostics。
    fn diagnostics(&self, uri: &str, text: &str) -> Out {
        let diags = match sv_compiler::compile(text, "component") {
            Ok(_) => Vec::new(),
            Err(e) => vec![diagnostic(&e)],
        };
        self.publish(uri, diags)
    }

    fn publish(&self, uri: &str, diagnostics: Vec<Value>) -> Out {
        Out::Send(serialize(&obj(vec![
            ("jsonrpc", str_("2.0")),
            ("method", str_("textDocument/publishDiagnostics")),
            (
                "params",
                obj(vec![
                    ("uri", str_(uri)),
                    ("diagnostics", Value::Arr(diagnostics)),
                ]),
            ),
        ])))
    }
}

/// `CompileError` → LSP Diagnostic。
///
/// CompileError 的行列是 **1-based**(与 rustc 对齐);LSP 的 line/character 是
/// **0-based**,所以各减一。终点列取 `col`(即起点后一格),给出一个至少一列宽的
/// 波浪线 —— 编译前端只给单点位置,没有区间,这是最诚实的近似。
fn diagnostic(e: &sv_compiler::CompileError) -> Value {
    let line = e.line.saturating_sub(1);
    let start_ch = e.col.saturating_sub(1);
    obj(vec![
        (
            "range",
            obj(vec![
                (
                    "start",
                    obj(vec![("line", num(line)), ("character", num(start_ch))]),
                ),
                (
                    "end",
                    obj(vec![("line", num(line)), ("character", num(e.col))]),
                ),
            ]),
        ),
        ("severity", num(1)), // 1 = Error
        ("source", str_("sv check")),
        ("message", str_(&e.message)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_compiler::check::json;

    fn parse(s: &str) -> Value {
        json::parse(s).expect("测试输入应是合法 JSON")
    }

    #[test]
    fn serialize_roundtrips_and_escapes() {
        let v = parse(r#"{"a":1,"b":[true,null,"x\ny"],"c":"quote\"here"}"#);
        let s = serialize(&v);
        // 再解析回来应等价(键序保持,因为 Obj 是 Vec)
        assert_eq!(parse(&s), v);
        // 整数不带 .0
        assert!(s.contains("\"a\":1"), "{s}");
        // 换行与引号被转义
        assert!(s.contains("x\\ny") && s.contains("quote\\\"here"), "{s}");
    }

    #[test]
    fn initialize_reports_capabilities() {
        let mut srv = Server::new();
        let out = srv.handle(&parse(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        ));
        assert_eq!(out.len(), 1);
        let Out::Send(msg) = &out[0] else {
            panic!("应是 Send")
        };
        let v = parse(msg);
        assert_eq!(v.get("id").unwrap(), &Value::Num(1.0));
        assert_eq!(
            v.get("result")
                .and_then(|r| r.get("capabilities"))
                .and_then(|c| c.get("textDocumentSync")),
            Some(&Value::Num(1.0))
        );
    }

    #[test]
    fn valid_sv_publishes_empty_diagnostics() {
        let mut srv = Server::new();
        let msg = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///a.svelte","text":{}}}}}}}"#,
            serialize(&Value::Str(
                "<script></script>\n<view><text>hi</text></view>\n".to_string()
            ))
        );
        let out = srv.handle(&parse(&msg));
        let Out::Send(m) = &out[0] else { panic!() };
        let v = parse(m);
        assert_eq!(
            v.get("method").and_then(|x| x.as_str()),
            Some("textDocument/publishDiagnostics")
        );
        let diags = v.get("params").and_then(|p| p.get("diagnostics")).unwrap();
        assert_eq!(diags.as_arr().len(), 0, "合法 .svelte 不该报诊断:{m}");
    }

    #[test]
    fn broken_sv_publishes_a_diagnostic_at_the_right_place() {
        let mut srv = Server::new();
        // <anim> 是未知标签 → 编译前端报错
        let src = "<script></script>\n<view><anim /></view>\n";
        let msg = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///b.svelte","text":{}}}}}}}"#,
            serialize(&Value::Str(src.to_string()))
        );
        let out = srv.handle(&parse(&msg));
        let Out::Send(m) = &out[0] else { panic!() };
        let v = parse(m);
        let diags = v.get("params").and_then(|p| p.get("diagnostics")).unwrap();
        assert_eq!(diags.as_arr().len(), 1, "坏 .svelte 应报一条诊断:{m}");
        let d = &diags.as_arr()[0];
        // 0-based:错误在第 2 行(index 1)
        let line = d
            .get("range")
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(|l| l.as_usize());
        assert_eq!(line, Some(1), "错误行应是 0-based 的 1:{m}");
        assert!(
            d.get("message")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .contains("animation"),
            "诊断消息应提到 animation:{m}"
        );
    }

    #[test]
    fn didchange_updates_and_close_clears() {
        let mut srv = Server::new();
        // 改成坏的
        let bad = "<script></script><view><anim /></view>";
        let change = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"file:///c.svelte"}},"contentChanges":[{{"text":{}}}]}}}}"#,
            serialize(&Value::Str(bad.to_string()))
        );
        let out = srv.handle(&parse(&change));
        let Out::Send(m) = &out[0] else { panic!() };
        assert_eq!(
            parse(m)
                .get("params")
                .and_then(|p| p.get("diagnostics"))
                .unwrap()
                .as_arr()
                .len(),
            1
        );

        // 关闭 → 清空诊断
        let close = r#"{"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":"file:///c.svelte"}}}"#;
        let out = srv.handle(&parse(close));
        let Out::Send(m) = &out[0] else { panic!() };
        assert_eq!(
            parse(m)
                .get("params")
                .and_then(|p| p.get("diagnostics"))
                .unwrap()
                .as_arr()
                .len(),
            0,
            "关闭应清空诊断"
        );
    }

    #[test]
    fn shutdown_then_exit_is_clean() {
        let mut srv = Server::new();
        srv.handle(&parse(r#"{"jsonrpc":"2.0","id":9,"method":"shutdown"}"#));
        let out = srv.handle(&parse(r#"{"jsonrpc":"2.0","method":"exit"}"#));
        assert_eq!(out, vec![Out::Exit(0)], "shutdown 后 exit 应是干净退出");
    }
}
