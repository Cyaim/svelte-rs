//! 重新生成 `src/generated.rs`:`cargo run -p sv-arco-tokens --bin gen_tokens`。
//!
//! arco 升版流程:更新 `assets/*.less` → 跑本命令 → `git diff` 审查 →
//! `cargo test -p sv-arco-tokens`(金样 + 同步测试)。

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/generated.rs");
    std::fs::write(path, sv_arco_tokens::generator::generate()).expect("写 generated.rs 失败");
    println!("已生成 {path}");
}
