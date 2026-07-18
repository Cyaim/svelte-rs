//! 待办示例:.sv 特性大集合。
//!
//! 覆盖:组件 + $props(必填/默认值/闭包 prop)、{#each}{:else}、{@const}、
//! {#key}、style: 指令、$inspect、$derived、onclick(Svelte 5 事件属性)。
//!
//! 运行:`cargo run -p todo-sfc`
//! 离屏:`cargo run -p todo-sfc -- --png out.png`

include!(concat!(env!("OUT_DIR"), "/todo.rs"));
include!(concat!(env!("OUT_DIR"), "/todo_item.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "todo.png".into());
        // 先模拟几次交互再截图,展示非空状态
        let doc = sv_ui::Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || todo(&d, d.root()));
        for _ in 0..3 {
            click_first_button(&doc, "添加");
        }
        click_first_button(&doc, "[ ]"); // 勾掉第一项
        sv_shell::render_doc_to_png(&doc, 960, 900, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 待办(.sv 特性演示)", |doc, root| todo(doc, root)).expect("运行失败");
}

/// 按文档顺序找第一个匹配文本的按钮并点击(--png 演示与测试共用)
fn click_first_button(doc: &sv_ui::Doc, label: &str) {
    fn walk(inner: &sv_ui::DocumentInner, id: sv_ui::ViewId, label: &str) -> Option<sv_ui::ViewId> {
        let n = &inner.nodes[id];
        if n.kind == sv_ui::ElementKind::Button && n.text == label {
            return Some(id);
        }
        n.children.iter().find_map(|c| walk(inner, *c, label))
    }
    if let Some(id) = doc.read(|inner| walk(inner, inner.root, label))
        && let Some(h) = doc.click_handler(id)
    {
        h();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_ui::{Doc, ElementKind};

    /// 按文档顺序(而不是 slotmap 槽位顺序)找第一个匹配的按钮,保证确定性
    fn find_button(doc: &Doc, label: &str) -> Option<sv_ui::ViewId> {
        fn walk(
            inner: &sv_ui::DocumentInner,
            id: sv_ui::ViewId,
            label: &str,
        ) -> Option<sv_ui::ViewId> {
            let n = &inner.nodes[id];
            if n.kind == ElementKind::Button && n.text == label {
                return Some(id);
            }
            n.children.iter().find_map(|c| walk(inner, *c, label))
        }
        doc.read(|inner| walk(inner, inner.root, label))
    }

    fn click(doc: &Doc, label: &str) {
        let id = find_button(doc, label).unwrap_or_else(|| panic!("找不到按钮 `{label}`"));
        doc.click_handler(id).expect("按钮应可点击")();
    }

    #[test]
    fn todo_full_feature_roundtrip() {
        let doc = Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || todo(&d, d.root()));

        // {#each}{:else}:初始空状态
        assert!(
            doc.dump().contains("空空如也"),
            "初始应显示空状态:\n{}",
            doc.dump()
        );
        assert!(
            doc.dump().contains("共 0 项"),
            "{{@const}} 摘要:\n{}",
            doc.dump()
        );

        // 添加三项:组件行出现,{@const} 联动
        click(&doc, "添加");
        click(&doc, "添加");
        click(&doc, "添加");
        let dump = doc.dump();
        assert!(
            dump.contains("1. 事项 1") && dump.contains("3. 事项 3"),
            "应有三行:\n{dump}"
        );
        assert!(dump.contains("共 3 项"), "{{@const}} 应联动:\n{dump}");
        assert!(!dump.contains("空空如也"));

        // 组件内局部状态:勾选第一行
        click(&doc, "[ ]"); // 第一个未勾选按钮
        assert!(
            doc.dump().contains("[x]"),
            "行内 done 状态应翻转:\n{}",
            doc.dump()
        );

        // 闭包 prop:删除第一行 → 索引重排
        click(&doc, "删除");
        let dump = doc.dump();
        assert!(
            !dump.contains("事项 1") && dump.contains("1. 事项 2"),
            "删除后应重排:\n{dump}"
        );
        assert!(dump.contains("共 2 项"));

        // 清空 → 回到 {:else}
        click(&doc, "清空");
        assert!(
            doc.dump().contains("空空如也"),
            "清空后应回空状态:\n{}",
            doc.dump()
        );
    }
}
