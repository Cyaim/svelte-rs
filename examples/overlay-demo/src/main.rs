//! 弹层演示(调研 25 O6):`<overlay>` 内建元素 + Dialog 组件自举
//! ($bindable open)+ 下拉菜单 + @attach 挂 tooltip 原语。
//!
//! 运行:`cargo run -p overlay-demo`
//! 离屏:`cargo run -p overlay-demo -- --png out.png [--menu|--dialog]`

include!(concat!(env!("OUT_DIR"), "/overlay_demo.rs"));
include!(concat!(env!("OUT_DIR"), "/dialog.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "overlay.png".into());
        let doc = sv_ui::Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || overlay_demo(&d, d.root()));
        // 截图前打开对应弹层
        if args.iter().any(|a| a == "--dialog") {
            click_button(&doc, "打开对话框");
        } else {
            click_button(&doc, "下拉菜单");
        }
        sv_shell::render_doc_to_png(&doc, 1100, 700, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 弹层演示", |doc, root| overlay_demo(doc, root)).expect("运行失败");
}

fn click_button(doc: &sv_ui::Doc, label: &str) {
    fn walk(inner: &sv_ui::DocumentInner, id: sv_ui::ViewId, label: &str) -> Option<sv_ui::ViewId> {
        let n = &inner.nodes[id];
        if n.kind == sv_ui::ElementKind::Button && n.text == label {
            return Some(id);
        }
        n.children.iter().find_map(|c| walk(inner, *c, label))
    }
    let found = doc.read(|inner| {
        walk(inner, inner.root, label).or_else(|| {
            // 弹层是游离子树,单独遍历
            inner
                .overlays
                .iter()
                .find_map(|e| walk(inner, e.root, label))
        })
    });
    if let Some(id) = found
        && let Some(h) = doc.click_handler(id)
    {
        h();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_ui::{Doc, Key, KeyEvent, Mods, dispatch_key};

    /// O4/O6 验收:下拉菜单打开 → 方向键导航 → Enter 选中并收起;
    /// Dialog 组件($bindable open)modal 往返
    #[test]
    fn menu_arrow_navigation_and_dialog_roundtrip() {
        let doc = Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || overlay_demo(&d, d.root()));

        // 打开菜单,dump 出现弹层段与菜单项
        click_button(&doc, "下拉菜单");
        let dump = doc.dump();
        assert!(
            dump.contains("== overlay") && dump.contains("新建"),
            "\n{dump}"
        );

        // 方向键导航:焦点进入菜单(Tab 先落进弹层内首项),Down 两次到"保存"
        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE));
        // Tab 全局树序可能落在基础层;直接把焦点定位到菜单第一项验证 O4
        let first_item = doc.read(|inner| {
            let root = inner.overlays[0].root;
            fn first_btn(inner: &sv_ui::DocumentInner, id: sv_ui::ViewId) -> Option<sv_ui::ViewId> {
                let n = &inner.nodes[id];
                if n.kind == sv_ui::ElementKind::Button {
                    return Some(id);
                }
                n.children.iter().find_map(|c| first_btn(inner, *c))
            }
            first_btn(inner, root).unwrap()
        });
        doc.focus(first_item);
        dispatch_key(&doc, &KeyEvent::new(Key::ArrowDown, Mods::NONE));
        dispatch_key(&doc, &KeyEvent::new(Key::ArrowDown, Mods::NONE));
        let focused_label = doc.read(|inner| inner.nodes[doc.focused().unwrap()].text.clone());
        assert_eq!(focused_label, "保存", "Popup 内方向键应导航菜单项");
        // Enter 激活:picked 更新、菜单收起
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        let dump = doc.dump();
        assert!(dump.contains("菜单选择:保存"), "\n{dump}");
        assert!(!dump.contains("== overlay"), "选中后菜单应收起:\n{dump}");

        // Esc 关对话框(Dialog 组件 ondismiss 回写 $bindable open)
        click_button(&doc, "打开对话框");
        assert!(doc.dump().contains("(modal)"), "\n{}", doc.dump());
        dispatch_key(&doc, &KeyEvent::new(Key::Escape, Mods::NONE));
        assert!(
            !doc.dump().contains("(modal)"),
            "Esc 应关掉对话框:\n{}",
            doc.dump()
        );
        // 组件按钮路径:再开一次用"确定"关
        click_button(&doc, "打开对话框");
        click_button(&doc, "确定");
        assert!(!doc.dump().contains("(modal)"));
    }
}
