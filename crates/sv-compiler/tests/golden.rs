//! codegen 输出金样 —— **ADR-2 ③ 重构的安全网**。
//!
//! 为什么需要它:`sv-compiler` 的 50 余项既有测试几乎全是
//! `assert!(code.contains("某片段"))` 的弱断言。它们能证明"该发的发了",
//! 证明不了"没多发、没换序、没换形状"。而 ③(setup/render 拆分 + 数据面接管
//! 结构)第一步 S1 的验收条件就是**生成代码逐字节不变** —— 没有金样根本没法验。
//!
//! 金样是**逐字节**的,刻意不做"忽略空白"之类的宽容:prettyplease 的排版
//! 本身也是产物的一部分(生成代码可读性是 ADR-2 的止血手段之一,排版塌了要
//! 立刻知道)。
//!
//! ## 金样变了怎么办
//!
//! 先问一句"这次改动**应该**改变生成代码吗":
//! - 应该(改了 codegen/emit/style 的发射形状)→ 用 `SV_UPDATE_GOLDEN=1` 重写,
//!   **并把 diff 逐段看一遍**再提交。金样的价值全在这一眼上;闭眼刷新等于没有金样。
//! - 不应该(只改了解析、诊断、无关模块)→ 那就是撞见了意外的形状漂移,查它。
//!
//! ```sh
//! SV_UPDATE_GOLDEN=1 cargo test -p sv-compiler --test golden
//! ```

use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// 逐字节比对;`SV_UPDATE_GOLDEN=1` 时重写金样
fn assert_golden(name: &str, actual: &str) {
    let path = fixtures_dir().join(
        // 后缀刻意不是 `.rs`:免得 cargo fmt / clippy 把金样当成源码去处理
        format!("{name}.rs.expected"),
    );
    if std::env::var("SV_UPDATE_GOLDEN").is_ok_and(|v| v == "1") {
        std::fs::write(&path, actual).expect("写金样失败");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "读不到金样 {}:{e}\n第一次生成请跑 SV_UPDATE_GOLDEN=1 cargo test -p sv-compiler --test golden",
            path.display()
        )
    });
    // 仓库里 .rs 会被 git 按 CRLF 检出,比对前统一,免得在 Windows 上恒红
    let (a, b) = (actual.replace("\r\n", "\n"), expected.replace("\r\n", "\n"));
    if a != b {
        // 定位到第一处差异:整文件 diff 打出来没人看得完
        let (mut line, mut col) = (1usize, 1usize);
        for (x, y) in a.chars().zip(b.chars()) {
            if x != y {
                break;
            }
            if x == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        let ctx = |s: &str| {
            s.lines()
                .skip(line.saturating_sub(3))
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        };
        panic!(
            "生成代码与金样不符({name}),首处差异在第 {line} 行第 {col} 列\n\
             --- 金样 ---\n{}\n--- 实际 ---\n{}\n\n\
             改动**应该**改变生成代码就跑 SV_UPDATE_GOLDEN=1 重写并逐段看 diff;\n\
             不该改变的话,你撞见了意外的形状漂移。",
            ctx(&b),
            ctx(&a)
        );
    }
}

/// 宽语法面 fixture:runes 三件套、预克隆、事件族、双向绑定、
/// if/else-if/else、keyed 与非 keyed each(带 :else)、key、await、
/// 过渡、滚动绑定、样式四态(base/hover/active/focus)
#[test]
fn wide_fixture_codegen_is_stable() {
    let src = std::fs::read_to_string(fixtures_dir().join("wide.svelte")).expect("读 fixture 失败");
    let code = sv_compiler::compile_sv(&src, "wide").expect("fixture 应能编译");
    // 生成物必须是合法 Rust —— 金样比对之前先过这道,免得把坏产物固化成金样
    syn::parse_file(&code).expect("生成代码应是合法 Rust");
    assert_golden("wide", &code);
}

/// 组件调用面单独一份:props / $bindable / snippet 的发射形状
#[test]
fn component_fixture_codegen_is_stable() {
    let child =
        std::fs::read_to_string(fixtures_dir().join("child.svelte")).expect("读 child 失败");
    let parent =
        std::fs::read_to_string(fixtures_dir().join("parent.svelte")).expect("读 parent 失败");

    // caller 侧要知道被调组件的 props 签名(build() 的第一遍扫描做的事,
    // 这里手工模拟成两遍)
    let mut registry = sv_compiler::PropsRegistry::new();
    registry.insert(
        "child",
        sv_compiler::PropsSig {
            fields: Some(vec![
                sv_compiler::PropsSigField {
                    name: "title".into(),
                    has_default: false,
                    bindable: false,
                },
                sv_compiler::PropsSigField {
                    name: "step".into(),
                    has_default: true,
                    bindable: false,
                },
                sv_compiler::PropsSigField {
                    name: "value".into(),
                    has_default: false,
                    bindable: true,
                },
                sv_compiler::PropsSigField {
                    name: "children".into(),
                    has_default: false,
                    bindable: false,
                },
            ]),
        },
    );

    let child_code =
        sv_compiler::compile_sv_with(&child, "child", &registry).expect("child 应编译");
    syn::parse_file(&child_code).expect("child 生成代码应合法");
    assert_golden("child", &child_code);

    let parent_code =
        sv_compiler::compile_sv_with(&parent, "parent", &registry).expect("parent 应编译");
    syn::parse_file(&parent_code).expect("parent 生成代码应合法");
    assert_golden("parent", &parent_code);
}
