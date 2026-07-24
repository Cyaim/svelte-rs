//! Divider 组件端到端行为(期望值溯源 divider-token.less)。
//!
//! 结构注意:组件是"恒渲染线-字-线 + 条件类切形态"(if 块包装节点会挡拉伸,
//! 见组件注释),所以纵向态也存在两个被清零的线段和一个空 text。

use sv_arco::{DividerProps, divider};
use sv_ui::{Color, Doc, ElementKind};

fn make(text: &str, vertical: bool) -> Doc {
    let doc = Doc::new();
    let d = doc.clone();
    let props = DividerProps {
        text: text.into(),
        vertical,
    };
    let (_, _scope) = sv_reactive::create_root(move || divider(&d, d.root(), props));
    doc
}

const LINE: Color = Color {
    r: 229,
    g: 230,
    b: 235,
    a: 255,
}; // gray-3

/// 按样式特征找节点(不依赖树结构内部字段)
fn find_view(doc: &Doc, pred: impl Fn(&sv_ui::Style) -> bool) -> sv_ui::ViewId {
    doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::View && pred(&n.style))
            .map(|(id, _)| id)
            .expect("应有目标 view")
    })
}

#[test]
fn plain_horizontal_line() {
    let doc = make("", false);
    let root = find_view(&doc, |s| s.margin.top == 20.0);
    let (m, gap, bg) = doc.read(|i| {
        let s = &i.nodes[root].style;
        (s.margin, s.gap, s.bg)
    });
    assert_eq!((m.top, m.bottom), (20.0, 20.0), "上下 margin = spacing-8");
    assert_eq!(gap, 0.0, "纯线态无 gap,两段线无缝拼合");
    assert_eq!(bg, None, "横向根自身不带色,线色在线段上");
    let lines = doc.read(|i| {
        i.nodes
            .iter()
            .filter(|(_, n)| n.style.bg == Some(LINE))
            .map(|(_, n)| (n.style.height, n.style.flex_grow))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        lines,
        vec![(Some(1.0), 1.0); 2],
        "两段 1px 线各 flex-grow:1"
    );
    let text = doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Text)
            .map(|(_, n)| n.text.clone())
    });
    assert_eq!(text.as_deref(), Some(""), "纯线态文字为空串(零宽)");
}

#[test]
fn vertical_line_is_the_root_itself() {
    let doc = make("忽略我", true);
    let root = find_view(&doc, |s| s.bg == Some(LINE) && s.width == Some(1.0));
    let (h, m) = doc.read(|i| {
        let s = &i.nodes[root].style;
        (s.height, s.margin)
    });
    assert_eq!(h, Some(14.0), "根自己是 1×14 竖线");
    assert_eq!((m.left, m.right), (12.0, 12.0), "左右 margin = spacing-6");
    // 子件全部清零:线段 0×0 不再 grow,text 为空
    let zeroed = doc.read(|i| {
        i.nodes
            .iter()
            .filter(|(_, n)| n.kind == ElementKind::View && n.style.width == Some(0.0))
            .map(|(_, n)| (n.style.width, n.style.height, n.style.flex_grow))
            .collect::<Vec<_>>()
    });
    assert_eq!(zeroed, vec![(Some(0.0), Some(0.0), 0.0); 2], "两线段应清零");
    let text = doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Text)
            .map(|(_, n)| n.text.clone())
    });
    assert_eq!(text.as_deref(), Some(""), "纵向忽略 text");
}

#[test]
fn text_form_is_line_text_line() {
    let doc = make("分组", false);
    let root = find_view(&doc, |s| s.margin.top == 20.0);
    let gap = doc.read(|i| i.nodes[root].style.gap);
    assert_eq!(gap, 16.0, "带字态 gap = spacing-7");
    let lines = doc.read(|i| {
        i.nodes
            .iter()
            .filter(|(_, n)| n.style.bg == Some(LINE))
            .map(|(_, n)| n.style.flex_grow)
            .collect::<Vec<_>>()
    });
    assert_eq!(lines, vec![1.0, 1.0], "两翼线段各 flex-grow:1");
    let (text, fg, fs) = doc.read(|i| {
        let (_, n) = i
            .nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Text)
            .expect("应有文字");
        (n.text.clone(), n.style.fg, n.style.font_size)
    });
    assert_eq!(text, "分组");
    assert_eq!(fg, Some(Color::rgb(29, 33, 41)), "文字 text-1");
    assert_eq!(fs, 14.0, "body-3");
}
