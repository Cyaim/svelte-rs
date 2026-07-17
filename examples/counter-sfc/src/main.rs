//! 计数器示例(编译器路线:`.sv` 单文件组件)。
//!
//! UI 定义在 `src/Counter.sv`,由 build.rs 里的 sv-compiler 编译成
//! `$OUT_DIR/counter.rs`(人类可读的 Rust),这里 include 进来。
//!
//! 运行:`cargo run -p counter-sfc`
//! 离屏验证:`cargo run -p counter-sfc -- --png out.png`

include!(concat!(env!("OUT_DIR"), "/counter.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "counter-sfc.png".into());
        sv_shell::render_to_png(|doc, root| counter(doc, root), 960, 800, 2.0, &path)
            .expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 计数器(SFC)", |doc, root| counter(doc, root)).expect("运行失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_ui::{Doc, ElementKind};

    /// .sv 编译产物的端到端行为:点击 → runes 写入 → 精准更新 → {#if} 翻转
    #[test]
    fn sfc_counter_behaves() {
        let doc = Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || counter(&d, d.root()));

        let plus = doc.read(|inner| {
            inner
                .nodes
                .iter()
                .find(|(_, n)| n.kind == ElementKind::Button && n.text == "+1")
                .map(|(id, _)| id)
                .expect("应有 +1 按钮")
        });
        let handler = doc.click_handler(plus).expect("+1 应可点击");
        for _ in 0..6 {
            handler();
        }
        let dump = doc.dump();
        assert!(dump.contains("Count: 6"), "计数应精准更新:\n{dump}");
        assert!(dump.contains("双倍 = 12"), "$derived 应联动:\n{dump}");
        assert!(dump.contains("超过 5 了!"), "{{#if}} 应翻转:\n{dump}");
    }
}
