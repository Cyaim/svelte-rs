//! Alert 组件端到端行为(期望值溯源 alert-token.less)。

use sv_arco::{AlertProps, alert};
use sv_ui::{Color, Doc, ElementKind};

fn make(title: &str, content: &str, status: &str) -> Doc {
    let doc = Doc::new();
    let d = doc.clone();
    let props = AlertProps {
        title: title.into(),
        content: content.into(),
        status: status.into(),
    };
    let (_, _scope) = sv_reactive::create_root(move || alert(&d, d.root(), props));
    doc
}

fn container(doc: &Doc) -> sv_ui::ViewId {
    doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::View && n.style.bg.is_some())
            .map(|(id, _)| id)
            .expect("应有 alert 容器")
    })
}

#[test]
fn four_statuses_use_palette_step_1() {
    for (status, rgb) in [
        ("info", (232, 243, 255)),
        ("success", (232, 255, 234)),
        ("warning", (255, 247, 232)),
        ("error", (255, 236, 232)),
    ] {
        let doc = make("", "正文", status);
        let bg = doc.read(|i| i.nodes[container(&doc)].style.bg);
        assert_eq!(
            bg,
            Some(Color::rgb(rgb.0, rgb.1, rgb.2)),
            "{status} 底色应是状态-1"
        );
    }
}

#[test]
fn container_radius_and_body_gap() {
    // 圆角(radius-small)与标题-正文 gap(spacing-2=4px)上一批零断言
    let doc = make("标题", "正文", "info");
    let id = container(&doc);
    let r = doc.read(|i| i.nodes[id].style.corner_radius);
    assert_eq!(r, 2.0, "圆角 = radius-small");
    // 文字区(alert-body)是容器唯一的 View 子节点,gap = 4px
    let gap = doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::View && n.style.gap > 0.0)
            .map(|(_, n)| n.style.gap)
            .expect("应有文字区")
    });
    assert_eq!(gap, 4.0, "标题-正文 gap = spacing-2");
}

#[test]
fn untitled_form() {
    let doc = make("", "只有正文", "info");
    let id = container(&doc);
    let pad = doc.read(|i| i.nodes[id].style.padding);
    assert_eq!(
        (pad.top, pad.left),
        (10.0, 17.0),
        "无标题 padding 9/16 + 1px 边框折算"
    );
    let (fg, fs) = doc.read(|i| {
        let (_, n) = i
            .nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Text)
            .expect("应有正文");
        (n.style.fg, n.style.font_size)
    });
    assert_eq!(fg, Some(Color::rgb(29, 33, 41)), "无标题正文 = text-1");
    assert_eq!(fs, 14.0, "body-3");
}

#[test]
fn titled_form_demotes_content_color() {
    let doc = make("提示", "正文内容", "info");
    let id = container(&doc);
    let pad = doc.read(|i| i.nodes[id].style.padding);
    assert_eq!(
        (pad.top, pad.left),
        (17.0, 17.0),
        "有标题 padding 16 全向 + 1px 折算"
    );
    let texts = doc.read(|i| {
        i.nodes
            .iter()
            .filter(|(_, n)| n.kind == ElementKind::Text)
            .map(|(_, n)| (n.text.clone(), n.style.fg, n.style.font_size))
            .collect::<Vec<_>>()
    });
    assert_eq!(texts.len(), 2, "标题 + 正文两条");
    let title = texts.iter().find(|(t, _, _)| t == "提示").expect("标题在");
    assert_eq!(
        (title.1, title.2),
        (Some(Color::rgb(29, 33, 41)), 16.0),
        "标题 text-1 + title-1 字号"
    );
    let content = texts
        .iter()
        .find(|(t, _, _)| t == "正文内容")
        .expect("正文在");
    assert_eq!(
        (content.1, content.2),
        (Some(Color::rgb(78, 89, 105)), 14.0),
        "有标题时正文降 text-2"
    );
}
