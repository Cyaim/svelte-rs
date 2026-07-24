//! 组件编译:`components/*.svelte` → 注入 arco 令牌 → sv-compiler。
//!
//! `:root` 变量是**每文件独立**的(sv-compiler style.rs 的作用域规则),而
//! 令牌有三百多个,不可能手抄进每个组件 —— 这里把每个组件 `<style>` 里的
//! `/*ARCO_TOKENS*/` 标记替换为 `sv_arco_tokens::CSS_ROOT_LIGHT`(整块
//! `:root {}`),staged 到 OUT_DIR 再交 `sv_compiler::build`。组件里写
//! `var(--primary-6)` 即可;忘写标记会因 `var()` 未定义在编译期报错,
//! 不会静默出错色。

use std::fs;
use std::path::Path;

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("build.rs 应有 OUT_DIR");
    let staged = Path::new(&out_dir).join("arco-staged");
    // 全量重建 staging,防组件改名后留下陈尸文件被继续编译
    if staged.exists() {
        fs::remove_dir_all(&staged).expect("清理 staging 失败");
    }
    fs::create_dir_all(&staged).expect("建 staging 目录失败");

    println!("cargo:rerun-if-changed=components");
    for entry in fs::read_dir("components").expect("components/ 应存在") {
        let path = entry.expect("读目录项失败").path();
        if path.extension().and_then(|e| e.to_str()) != Some("svelte") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let src = fs::read_to_string(&path).expect("读组件源失败");
        let injected = src.replace("/*ARCO_TOKENS*/", sv_arco_tokens::CSS_ROOT_LIGHT);
        fs::write(staged.join(path.file_name().expect("应有文件名")), injected)
            .expect("写 staging 失败");
    }
    sv_compiler::build(&staged);
}
