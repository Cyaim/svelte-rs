//! `sv-lsp` 二进制:LSP 的 stdio 传输层。
//!
//! 只负责**分帧**:按 LSP 规范读 `Content-Length: N\r\n\r\n` + N 字节的 body,
//! 解析成 JSON 交给 [`sv_lsp::Server::handle`],再把返回的每条消息按同样的
//! 帧格式写回 stdout。所有语义都在 lib 里(纯函数、有测试),这里薄到不必测。
//!
//! 编辑器接法(VS Code 等):把本二进制配成 `.sv` 的 language server,stdio 通道。

use std::io::{Read, Write};

use sv_compiler::check::json;
use sv_lsp::{Out, Server};

fn main() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut server = Server::new();

    while let Some(body) = read_message(&mut stdin) {
        // 解析失败就丢弃这一帧(cargo/编辑器偶尔会插入非 JSON;LSP 允许忽略坏消息)
        let Some(msg) = json::parse(&body) else {
            continue;
        };
        for out in server.handle(&msg) {
            match out {
                Out::Send(text) => write_message(&mut stdout, &text),
                Out::Exit(code) => std::process::exit(code),
            }
        }
    }
}

/// 读一帧:头部若干行 `Key: Value\r\n`,空行后是 `Content-Length` 指定长度的 body。
/// 读到 EOF 返回 `None`(客户端关了管道)。
fn read_message(r: &mut impl Read) -> Option<String> {
    let mut headers = Vec::new();
    // 逐字节读到 `\r\n\r\n`(头部结束)
    let mut window = [0u8; 4];
    let mut one = [0u8; 1];
    loop {
        if r.read_exact(&mut one).is_err() {
            return None;
        }
        headers.push(one[0]);
        let n = headers.len();
        if n >= 4 {
            window.copy_from_slice(&headers[n - 4..]);
            if &window == b"\r\n\r\n" {
                break;
            }
        }
    }
    let head = String::from_utf8_lossy(&headers);
    let len: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().eq_ignore_ascii_case("Content-Length")).then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    if len == 0 {
        return Some(String::new());
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn write_message(w: &mut impl Write, body: &str) {
    // body 是 ASCII/UTF-8;Content-Length 是**字节**长度
    let _ = write!(w, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = w.flush();
}
