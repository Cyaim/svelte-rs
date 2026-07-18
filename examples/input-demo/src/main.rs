//! 文本输入 / IME / 剪贴板手测台(调研 21 §3:真输入法全流程只能真机验证)。
//!
//! 运行:`cargo run -p input-demo`
//! 离屏:`cargo run -p input-demo -- --png out.png`
//! 手测清单见本目录 README.md(三平台 × 各一款输入法)。

include!(concat!(env!("OUT_DIR"), "/input_demo.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "input-demo.png".into());
        let doc = sv_ui::Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || input_demo(&d, d.root()));
        // 模拟一次键入 + 组合中状态,截图展示光标/预编辑下划线
        use sv_ui::{Key, KeyEvent, Mods, dispatch_key};
        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE));
        if let Some(input) = doc.focused() {
            for (k, t) in [(Key::Char('你'), "你"), (Key::Char('好'), "好")] {
                dispatch_key(&doc, &KeyEvent::new(k, Mods::NONE).with_text(t));
            }
            sv_ui::handle_ime(
                &doc,
                input,
                sv_ui::ImeEvent::Preedit("shijie".into(), Some((6, 6))),
            );
        }
        sv_shell::render_doc_to_png(&doc, 1100, 800, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 输入手测台", |doc, root| input_demo(doc, root)).expect("运行失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_ui::{Doc, ElementKind, Key, KeyEvent, Mods, dispatch_key};

    /// 手测台自身的自动化底座:键入 → 实时值 → 提交 → 历史 → 清空
    #[test]
    fn input_demo_keyboard_roundtrip() {
        let doc = Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || input_demo(&d, d.root()));

        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE));
        let input = doc.focused().expect("Tab 应聚焦输入框");
        assert!(doc.read(|inner| inner.nodes[input].kind == ElementKind::TextInput));

        for (k, t) in [(Key::Char('h'), "h"), (Key::Char('i'), "i")] {
            dispatch_key(&doc, &KeyEvent::new(k, Mods::NONE).with_text(t));
        }
        assert!(doc.dump().contains("实时值(bind:value):hi"));
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        let dump = doc.dump();
        assert!(
            dump.contains("上次提交:hi") && dump.contains("1. hi"),
            "\n{dump}"
        );
        assert_eq!(doc.input_value(input).unwrap(), "", "提交后清空");
    }
}
