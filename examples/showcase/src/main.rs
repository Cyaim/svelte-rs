//! sv 特性橱窗:第三批特性全家桶演示。
//!
//! 覆盖:`$bindable` 双向绑定(bind:value)、组件 children snippet、
//! `{#snippet}/{@render}`、keyed `{#each}`(重排保状态)、`<style>` 块
//! (scoped 类,两个文件同名 .btn 互不干扰)、`{@const}`、`{#key}`、`{@debug}`。
//!
//! 运行:`cargo run -p showcase`
//! 离屏:`cargo run -p showcase -- --png out.png`

include!(concat!(env!("OUT_DIR"), "/showcase.rs"));
include!(concat!(env!("OUT_DIR"), "/card.rs"));
include!(concat!(env!("OUT_DIR"), "/stepper.rs"));
include!(concat!(env!("OUT_DIR"), "/task_row.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args.get(i + 1).cloned().unwrap_or_else(|| "showcase.png".into());
        let doc = sv_ui::Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || showcase(&d, d.root()));
        // 演示交互:计数 +2 两次、勾掉第一行、反转顺序
        click(&doc, "+2");
        click(&doc, "+2");
        click(&doc, "[ ]");
        click(&doc, "反转顺序");
        sv_shell::render_doc_to_png(&doc, 1100, 1100, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 特性橱窗", |doc, root| showcase(doc, root)).expect("运行失败");
}

/// 按文档顺序找第一个匹配文本的按钮并点击
fn click(doc: &sv_ui::Doc, label: &str) {
    fn walk(
        inner: &sv_ui::DocumentInner,
        id: sv_ui::ViewId,
        label: &str,
    ) -> Option<sv_ui::ViewId> {
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

    #[test]
    fn showcase_full_roundtrip() {
        let doc = sv_ui::Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || showcase(&d, d.root()));

        // $bindable:Stepper 内点击 → 外部 stat snippet 联动
        click(&doc, "+2");
        click(&doc, "+2");
        let dump = doc.dump();
        assert!(dump.contains("外部读同一信号"), "\n{dump}");
        assert!(dump.contains("\"4\""), "bind:value 双向生效,count=4:\n{dump}");
        assert!(dump.contains("\"8\""), "$derived 双倍=8:\n{dump}");
        assert!(dump.contains("3 项任务 · 计数 4"), "{{@const}} 联动:\n{dump}");

        // keyed:勾选第一行(#1),反转后状态跟着行走
        click(&doc, "[ ]");
        assert!(doc.dump().contains("[x]"));
        click(&doc, "反转顺序");
        let dump = doc.dump();
        let x_pos = dump.find("[x]").expect("勾选状态应保留");
        let one_pos = dump.find("#1 学 Rust").expect("行应还在");
        let three_pos = dump.find("#3 发布").expect("行应还在");
        assert!(three_pos < one_pos, "顺序应反转:\n{dump}");
        assert!(x_pos > three_pos, "[x] 应跟着 #1 行走到后面:\n{dump}");

        // 删除第一行(反转后是 #3)
        click(&doc, "删");
        let dump = doc.dump();
        assert!(!dump.contains("#3") && dump.contains("2 项任务"), "\n{dump}");

        // 清空到 {#if} 空状态
        click(&doc, "删");
        click(&doc, "删");
        assert!(doc.dump().contains("没有任务了"), "\n{}", doc.dump());
    }
}
