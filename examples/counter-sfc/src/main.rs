//! 计数器示例(编译器路线:`.svelte` 单文件组件)。
//!
//! UI 定义在 `src/Counter.svelte`,由 build.rs 里的 sv-compiler 编译成
//! `$OUT_DIR/counter.rs`(人类可读的 Rust),这里 include 进来。
//!
//! 运行:`cargo run -p counter-sfc`
//! 离屏验证:`cargo run -p counter-sfc -- --png out.png`

include!(concat!(env!("OUT_DIR"), "/counter.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "counter-sfc.png".into());
        sv_shell::render_to_png(counter, 960, 800, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 计数器(SFC)", counter).expect("运行失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_ui::{Doc, ElementKind};

    /// .svelte 编译产物的端到端行为:点击 → runes 写入 → 精准更新 → {#if} 翻转
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

    /// R1 键盘闭环(调研 20 验收):Tab 树序落焦 → Enter/Space 激活 →
    /// Shift+Tab 环绕 → 按钮免费获得键盘可达性,零 .svelte 改动
    #[test]
    fn sfc_counter_keyboard_roundtrip() {
        use sv_ui::{Key, KeyEvent, Mods, dispatch_key};
        let doc = Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || counter(&d, d.root()));

        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE));
        let focused = doc.focused().expect("Tab 应带来焦点");
        let label = doc.read(|inner| inner.nodes[focused].text.clone());
        assert_eq!(label, "+1", "树序第一个 focusable 应是 +1 按钮");

        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        dispatch_key(&doc, &KeyEvent::new(Key::Space, Mods::NONE));
        assert!(
            doc.dump().contains("Count: 2"),
            "Enter/Space 应各激活一次:\n{}",
            doc.dump()
        );

        // Shift+Tab 从 +1 环绕到树序最后一个按钮(归零),Enter 归零
        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::SHIFT));
        let focused = doc.focused().unwrap();
        let label = doc.read(|inner| inner.nodes[focused].text.clone());
        assert_eq!(label, "归零", "Shift+Tab 应环绕到最后一个 focusable");
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        assert!(doc.dump().contains("Count: 0"), "\n{}", doc.dump());
    }
}
