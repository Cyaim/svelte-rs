//! `cargo-sv` —— svelte-rs 的开发工具入口(cargo 子命令:`cargo sv <子命令>`;
//! ADR-10 标识符改名:原 `sv-check` 二进制。取 `cargo-sv` 而非裸 `sv` 是刻意的:
//! 裸 `sv` 会与 Linux runit 的 `/usr/bin/sv` 撞名,cargo-* 前缀由 cargo 分发、
//! 无 PATH 遮蔽,且与调研 06 的 `cargo sv check` 设想逐字吻合)。
//!
//! 目前唯一的子命令是 `check`:跑 `cargo check`,把落在生成 `.rs` 上的
//! rustc 诊断搬回 `.svelte`。
//!
//! 用法:
//! ```sh
//! cargo run -q -p sv-compiler --bin cargo-sv -- check    # 仓库内;默认 --workspace
//! cargo run -q -p sv-compiler --bin cargo-sv -- check -p counter-sfc
//! cargo sv check                                         # cargo install 后
//! ```
//! `check` 之后的参数原样透传给 `cargo check`(features/target 都照用)。
//! 输出是 rustc 风格的单行 `路径:行:列: level[code]: 消息`,
//! `.vscode/tasks.json` 里的 problemMatcher 直接吃这一行。
//!
//! 重映射逻辑与降级策略全在 [`sv_compiler::check`],这里只做进程编排。

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use sv_compiler::check::{Line, Session, scrape_build_script_error};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `cargo sv ...` 分发形态:cargo 会把子命令名自身作为第一个参数传入
    if args.first().map(String::as_str) == Some("sv") {
        args.remove(0);
    }
    let usage = "cargo sv —— svelte-rs 开发工具\n\n\
                 用法: cargo sv check [cargo check 的参数...]\n\
                 `check` 把 rustc 落在生成 .rs 上的诊断搬回 .svelte;\n\
                 不给额外参数时等价于 `cargo check --workspace`。";
    match args.first().map(String::as_str) {
        Some("check") => {
            args.remove(0);
        }
        Some("-h") | Some("--help") => {
            eprintln!("{usage}");
            return;
        }
        Some(other) => {
            eprintln!("cargo sv: 未知子命令 `{other}`\n\n{usage}");
            std::process::exit(2);
        }
        None => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("{usage}");
        return;
    }
    std::process::exit(run(&args));
}

fn run(args: &[String]) -> i32 {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.arg("check").arg("--message-format=json");
    if args.is_empty() {
        cmd.arg("--workspace");
    } else {
        cmd.args(args);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sv check: 启动 cargo 失败: {e}");
            return 2;
        }
    };

    // stderr 单开一条线程:一边原样转发(用户要看 cargo 的进度与链接错误),
    // 一边留下 build.rs 里 `.svelte` 编译器域错误的 panic dump —— 那条路根本不进
    // JSON 流(见 check.rs 头部)。`Session` 不跨线程,所以这里只筛不判:
    // 真正的计数与措辞回主线程做(那一半才有单测)
    let err_pipe = child.stderr.take().expect("stderr 已接管");
    let err_thread = std::thread::spawn(move || {
        let mut candidates = Vec::new();
        for line in BufReader::new(err_pipe).lines().map_while(Result::ok) {
            eprintln!("{line}");
            if scrape_build_script_error(&line).is_some() {
                candidates.push(line);
            }
        }
        candidates
    });

    let mut session = Session::new();
    let out = BufReader::new(child.stdout.take().expect("stdout 已接管"));
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    for line in out.lines().map_while(Result::ok) {
        match session.feed_stdout(&line) {
            Line::Skip => {}
            Line::Diag(r) => {
                let _ = writeln!(w, "{}", r.headline);
                for c in &r.context {
                    let _ = writeln!(w, "{c}");
                }
            }
            // 解析不了也**必须留痕**:这行里可能就有一条诊断。走 stderr
            // 是为了不污染 problemMatcher 吃的那条 stdout 流;原文再缩进一格,
            // 免得它自己被当成一条诊断(与 `Rendered::context` 同一套约定)
            Line::Unparsed => {
                eprintln!("sv check: 下面这行 cargo 输出解析不了,原样透出:");
                eprintln!("  {line}");
            }
        }
    }

    let status = child.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);
    for line in err_thread.join().unwrap_or_default() {
        if let Some(d) = session.feed_stderr(&line) {
            let _ = writeln!(w, "{d}");
        }
    }
    let _ = writeln!(w, "{}", session.summary());
    let _ = w.flush();

    session.exit_code(status)
}
