//! Link 组件端到端行为(期望值溯源 link-token.less)。

use std::cell::Cell;
use std::rc::Rc;
use sv_arco::{LinkProps, link};
use sv_ui::{Color, Cursor, Doc, ElementKind};

fn make(status: &str, disabled: bool) -> (Doc, Rc<Cell<u32>>) {
    let clicks = Rc::new(Cell::new(0u32));
    let doc = Doc::new();
    let (d, c) = (doc.clone(), clicks.clone());
    let props = LinkProps {
        label: "链接".into(),
        status: status.into(),
        disabled,
        on_click: Rc::new(move || c.set(c.get() + 1)),
    };
    let (_, _scope) = sv_reactive::create_root(move || link(&d, d.root(), props));
    (doc, clicks)
}

fn the_link(doc: &Doc) -> sv_ui::ViewId {
    doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Button)
            .map(|(id, _)| id)
            .expect("应有 link(button 叶)")
    })
}

#[test]
fn statuses_use_palette_6_no_bg() {
    for (status, rgb) in [
        ("default", (22, 93, 255)), // link-6 = arcoblue-6
        ("success", (0, 180, 42)),
        ("warning", (255, 125, 0)),
        ("danger", (245, 63, 63)),
    ] {
        let (doc, _) = make(status, false);
        let id = the_link(&doc);
        let (fg, bg, cursor) = doc.read(|i| {
            let s = &i.nodes[id].style;
            (s.fg, s.bg, s.cursor)
        });
        assert_eq!(fg, Some(Color::rgb(rgb.0, rgb.1, rgb.2)), "{status} 文字色");
        assert_eq!(bg, None, "常态无底色");
        assert_eq!(cursor, Some(Cursor::Pointer));
    }
}

#[test]
fn disabled_shades_match_token_quirks() {
    // warning 禁用是 light-2(arco 原文,非 light-3)——照抄不修正
    for (status, rgb) in [
        ("default", (148, 191, 255)), // link-3
        ("success", (123, 225, 136)), // success-3
        ("warning", (255, 228, 186)), // warning-2!
        ("danger", (251, 172, 163)),  // danger-3
    ] {
        let (doc, _) = make(status, true);
        let id = the_link(&doc);
        let (fg, cursor) = doc.read(|i| (i.nodes[id].style.fg, i.nodes[id].style.cursor));
        assert_eq!(fg, Some(Color::rgb(rgb.0, rgb.1, rgb.2)), "{status} 禁用色");
        assert_eq!(cursor, Some(Cursor::NotAllowed));
    }
}

#[test]
fn click_fires_and_disabled_swallows() {
    let (doc, clicks) = make("default", false);
    doc.click_handler(the_link(&doc)).expect("应可点击")();
    assert_eq!(clicks.get(), 1);

    let (doc, clicks) = make("default", true);
    if let Some(h) = doc.click_handler(the_link(&doc)) {
        h();
    }
    assert_eq!(clicks.get(), 0, "禁用应短路");
}

#[test]
fn geometry_from_link_tokens() {
    // .link 的几何(字号 body-3 / padding 1px 4px / radius radius-small)
    // 上一批零断言(对抗评审查获)
    let (doc, _) = make("default", false);
    let id = the_link(&doc);
    let (fs, pad, r) = doc.read(|i| {
        let s = &i.nodes[id].style;
        (s.font_size, s.padding, s.corner_radius)
    });
    assert_eq!(fs, 14.0, "字号 = body-3");
    assert_eq!((pad.top, pad.left), (1.0, 4.0), "padding 1px/4px");
    assert_eq!(r, 2.0, "圆角 = radius-small");
}

/// hover/active 底色(fill-2/fill-3)之前编译期被丢弃,本批修 codegen 后生效。
#[test]
fn hover_and_active_underlays() {
    let (doc, _) = make("default", false);
    let id = the_link(&doc);
    let bg = |d: &Doc| d.read(|i| i.nodes[id].style.bg);

    assert_eq!(bg(&doc), None, "常态无底色");
    doc.pointer_enter_handler(id).expect("应接悬停")();
    assert_eq!(
        bg(&doc),
        Some(Color::rgb(242, 243, 245)),
        "hover 底 = fill-2"
    );
    doc.pointer_down_handler(id).expect("应接按压")();
    assert_eq!(
        bg(&doc),
        Some(Color::rgb(229, 230, 235)),
        "active 底 = fill-3"
    );
    doc.pointer_up_handler(id).expect("应接抬起")();
    doc.pointer_leave_handler(id).expect("应接移出")();
    assert_eq!(bg(&doc), None, "移出复位无底色");
}
