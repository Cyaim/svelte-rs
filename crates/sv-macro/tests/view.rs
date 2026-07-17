//! `view!` 宏端到端测试:用 `Doc::dump()`(缩进文本)与 `doc.read(...)`
//! 断言生成代码的结构与细粒度更新行为。

use sv_macro::view;
use sv_reactive::state;
use sv_ui::{Doc, ElementKind, ViewId};

/// 遍历场景树,收集指定类型节点的 id(宏返回 (),测试用这个反查节点)
fn find_kind(doc: &Doc, kind: ElementKind) -> Vec<ViewId> {
    doc.read(|inner| {
        inner
            .nodes
            .iter()
            .filter(|(_, n)| n.kind == kind)
            .map(|(id, _)| id)
            .collect()
    })
}

// ---------------------------------------------------------------------------
// 1. 计数器:文本插值 + on_click
// ---------------------------------------------------------------------------

#[test]
fn counter() {
    let doc = Doc::new();
    let count = state(0);
    view! { &doc, doc.root() =>
        <text>"Count: " {count.get()}</text>
        <button on_click(move || count.update(|c| *c += 1))>"+1"</button>
    };
    assert!(doc.dump().contains("Count: 0"), "初始 dump:\n{}", doc.dump());
    assert!(doc.dump().contains("[button \"+1\"]"));

    // 模拟点击:遍历 nodes 找 Button 节点取 handler
    let buttons = find_kind(&doc, ElementKind::Button);
    assert_eq!(buttons.len(), 1);
    let h = doc.click_handler(buttons[0]).unwrap();
    h();
    h();
    assert!(doc.dump().contains("Count: 2"), "点击后 dump:\n{}", doc.dump());
}

// ---------------------------------------------------------------------------
// 2. 静态文本合并:无绑定 effect,版本稳定
// ---------------------------------------------------------------------------

#[test]
fn static_text_merges_without_binding() {
    let doc = Doc::new();
    view! { &doc, doc.root() => <text>"a" "b"</text> };
    assert!(doc.dump().contains("\"ab\""), "dump:\n{}", doc.dump());

    let v = doc.version();
    let unrelated = state(0);
    unrelated.set(1); // 静态文本没有任何 effect,树不应被触碰
    assert_eq!(doc.version(), v, "静态文本不应产生绑定 effect");
}

#[test]
fn text_runs_split_by_elements() {
    let doc = Doc::new();
    view! { &doc, doc.root() =>
        <view>
            "a" "b"
            <text>"x"</text>
            "c"
        </view>
    };
    let dump = doc.dump();
    assert!(dump.contains("\"ab\"") && dump.contains("\"x\"") && dump.contains("\"c\""));
    // 元素打断合并:view 下应是 3 个子节点("ab"、"x"、"c")
    let child_count = doc.read(|inner| {
        let view = inner.nodes[inner.root].children[0];
        inner.nodes[view].children.len()
    });
    assert_eq!(child_count, 3);
}

// ---------------------------------------------------------------------------
// 3. 动态文本:signal 变化后精准更新
// ---------------------------------------------------------------------------

#[test]
fn dynamic_text_updates() {
    let doc = Doc::new();
    let name = state(String::from("world"));
    view! { &doc, doc.root() => <text>"hello " {name.get()}</text> };
    assert!(doc.dump().contains("\"hello world\""));

    let v = doc.version();
    name.set("svelte".into());
    assert!(doc.dump().contains("\"hello svelte\""));
    assert!(!doc.dump().contains("world"));
    assert!(doc.version() > v, "文本变化应 bump 版本");

    // set 不做相等剪枝,但 set_text 写入相同文本不 bump 版本
    let v = doc.version();
    name.set("svelte".into());
    assert_eq!(doc.version(), v, "相同文本不应触发树变更");
}

// ---------------------------------------------------------------------------
// 4. if / else if / else:分支切换与内部状态销毁
// ---------------------------------------------------------------------------

#[test]
fn if_else_chain_switches() {
    let doc = Doc::new();
    let n = state(0);
    view! { &doc, doc.root() =>
        if n.get() > 10 {
            <text>"big"</text>
        } else if n.get() > 5 {
            <text>"mid"</text>
        } else {
            <text>"small"</text>
        }
    };
    assert!(doc.dump().contains("small"));

    n.set(7);
    let dump = doc.dump();
    assert!(dump.contains("mid") && !dump.contains("small") && !dump.contains("big"));

    n.set(42);
    let dump = doc.dump();
    assert!(dump.contains("big") && !dump.contains("mid"));

    n.set(0);
    assert!(doc.dump().contains("small"));
}

#[test]
fn if_inner_bindings_disposed() {
    let doc = Doc::new();
    let show = state(true);
    let ticker = state(0);
    view! { &doc, doc.root() =>
        if show.get() {
            <text>"tick " {ticker.get()}</text>
        }
    };
    ticker.set(1);
    assert!(doc.dump().contains("tick 1"));

    show.set(false); // 分支销毁,内部 bind_text effect 应一并销毁
    assert!(!doc.dump().contains("tick"));
    let v = doc.version();
    ticker.set(2); // 不应再有 effect 去改树
    assert_eq!(doc.version(), v, "分支销毁后其内部绑定不应再触发树变更");
}

// ---------------------------------------------------------------------------
// 5. for:带索引与不带索引,增删元素
// ---------------------------------------------------------------------------

#[test]
fn for_with_index() {
    let doc = Doc::new();
    let items = state(vec!["a".to_string(), "b".to_string()]);
    view! { &doc, doc.root() =>
        for item, i in items.get() {
            <text>{i} ":" {item}</text>
        }
    };
    let dump = doc.dump();
    assert!(dump.contains("0:a") && dump.contains("1:b"), "dump:\n{dump}");

    items.update(|v| v.push("c".into()));
    assert!(doc.dump().contains("2:c"));

    items.update(|v| {
        v.remove(0);
    });
    let dump = doc.dump();
    assert!(!dump.contains(":a") && dump.contains("0:b") && dump.contains("1:c"), "dump:\n{dump}");
}

#[test]
fn for_without_index() {
    let doc = Doc::new();
    let items = state(vec![1, 2, 3]);
    view! { &doc, doc.root() =>
        for n in items.get() {
            <text>"#" {n}</text>
        }
    };
    let dump = doc.dump();
    assert!(dump.contains("#1") && dump.contains("#2") && dump.contains("#3"));

    items.update(|v| v.retain(|x| x % 2 == 1));
    let dump = doc.dump();
    assert!(dump.contains("#1") && !dump.contains("#2") && dump.contains("#3"));
}

// ---------------------------------------------------------------------------
// 6. 嵌套:view 里嵌 if,if 里嵌 for
// ---------------------------------------------------------------------------

#[test]
fn nested_view_if_for() {
    let doc = Doc::new();
    let show = state(true);
    let items = state(vec![10, 20]);
    view! { &doc, doc.root() =>
        <view>
            "header"
            if show.get() {
                for n, i in items.get() {
                    <text>"item" {i} "=" {n}</text>
                }
            } else {
                <text>"collapsed"</text>
            }
        </view>
    };
    let dump = doc.dump();
    assert!(dump.contains("header") && dump.contains("item0=10") && dump.contains("item1=20"), "dump:\n{dump}");

    show.set(false);
    let dump = doc.dump();
    assert!(dump.contains("collapsed") && !dump.contains("item0"), "dump:\n{dump}");

    items.update(|v| v.push(30)); // 隐藏期间更新列表:each 已随分支销毁,不应有影响
    assert!(!doc.dump().contains("item2"));

    show.set(true); // 重建分支,each 读到最新列表
    let dump = doc.dump();
    assert!(dump.contains("item0=10") && dump.contains("item2=30"), "dump:\n{dump}");
}

// ---------------------------------------------------------------------------
// 7. style:响应式样式与嵌套闭包捕获
// ---------------------------------------------------------------------------

#[test]
fn reactive_style() {
    let doc = Doc::new();
    let count = state(0);
    view! { &doc, doc.root() =>
        <view style(move |s| { s.padding = count.get() as f32; })>
            <text>"styled"</text>
        </view>
    };
    let vid = doc.read(|inner| inner.nodes[inner.root].children[0]);
    assert_eq!(doc.read(|inner| inner.nodes[vid].style.padding), 0.0);
    count.set(4);
    assert_eq!(doc.read(|inner| inner.nodes[vid].style.padding), 4.0, "样式应随 signal 更新");
}

#[test]
fn style_in_each_row_captures_row_scope() {
    let doc = Doc::new();
    let unit = state(1.0f32);
    view! { &doc, doc.root() =>
        for _n, i in vec![0, 0] {
            <view style(move |s| { s.padding = unit.get() * (i as f32 + 1.0); }) />
        }
    };
    // root -> each 容器 -> 两个行 view
    let rows: Vec<ViewId> = doc.read(|inner| {
        let container = inner.nodes[inner.root].children[0];
        inner.nodes[container].children.clone()
    });
    assert_eq!(rows.len(), 2);
    let pads = |doc: &Doc| {
        doc.read(|inner| rows.iter().map(|id| inner.nodes[*id].style.padding).collect::<Vec<_>>())
    };
    assert_eq!(pads(&doc), vec![1.0, 2.0], "style 闭包应捕获行索引");
    unit.set(2.0);
    assert_eq!(pads(&doc), vec![2.0, 4.0], "行内 style 绑定应随 signal 更新");
}

// ---------------------------------------------------------------------------
// 补充:自闭合与空 label 按钮
// ---------------------------------------------------------------------------

#[test]
fn self_closing_elements() {
    let doc = Doc::new();
    let clicked = state(false);
    view! { &doc, doc.root() =>
        <view />
        <button on_click(move || clicked.set(true)) />
    };
    assert!(doc.dump().contains("[button \"\"]"), "自闭合 button 的 label 应为空串:\n{}", doc.dump());
    assert_eq!(doc.read(|inner| inner.nodes[inner.root].children.len()), 2);

    let buttons = find_kind(&doc, ElementKind::Button);
    let h = doc.click_handler(buttons[0]).unwrap();
    h();
    assert!(clicked.get());
}
