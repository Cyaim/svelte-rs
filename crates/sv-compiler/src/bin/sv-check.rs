//! `sv check` —— 跑 `cargo check`,把落在生成 `.rs` 上的 rustc 诊断搬回 `.sv`。
//!
//! 用法:
//! ```sh
//! cargo run -q -p sv-compiler --bin sv-check              # 默认 --workspace
//! cargo run -q -p sv-compiler --bin sv-check -- -p counter-sfc
//! ```
//! 参数原样透传给 `cargo check`(features/target 都照用)。
//! 输出是 rustc 风格的单行 `路径:行:列: level[code]: 消息`,
//! `.vscode/tasks.json` 里的 problemMatcher 直接吃这一行。
//!
//! 重映射逻辑与降级策略全在 [`sv_compiler::check`],这里只做进程编排。

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use sv_compiler::check::{Line, Session, scrape_build_script_error};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!(
            "sv check —— 把 rustc 落在生成 .rs 上的诊断搬回 .sv\n\n\
             用法: sv-check [cargo check 的参数...]\n\
             不给参数时等价于 `cargo check --workspace`。"
        );
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
            eprintln!("sv-check: 启动 cargo 失败: {e}");
            return 2;
        }
    };

    // stderr 单开一条线程:一边原样转发(用户要看 cargo 的进度与链接错误),
    // 一边留下 build.rs 里 `.sv` 编译器域错误的 panic dump —— 那条路根本不进
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
                eprintln!("sv-check: 下面这行 cargo 输出解析不了,原样透出:");
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
