//! Button 组件端到端行为:建真 Doc、查节点、断样式与点击(离屏,不开窗)。

use std::cell::Cell;
use std::rc::Rc;
use sv_arco::{ButtonProps, button};
use sv_ui::{Color, Doc, ElementKind};

fn make(variant: &str, status: &str, size: &str, disabled: bool) -> (Doc, Rc<Cell<u32>>) {
    let clicks = Rc::new(Cell::new(0u32));
    let doc = Doc::new();
    let (d, c) = (doc.clone(), clicks.clone());
    let props = ButtonProps {
        label: "按钮".into(),
        variant: variant.into(),
        status: status.into(),
        size: size.into(),
        disabled,
        on_click: Rc::new(move || c.set(c.get() + 1)),
    };
    let (_, _scope) = sv_reactive::create_root(move || button(&d, d.root(), props));
    (doc, clicks)
}

fn the_button(doc: &Doc) -> sv_ui::ViewId {
    doc.read(|inner| {
        inner
            .nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Button)
            .map(|(id, _)| id)
            .expect("应有 button 节点")
    })
}

#[test]
fn primary_colors_come_from_arco_tokens() {
    let (doc, _) = make("primary", "default", "default", false);
    let id = the_button(&doc);
    let (bg, fg, h) = doc.read(|i| {
        (
            i.nodes[id].style.bg,
            i.nodes[id].style.fg,
            i.nodes[id].style.height,
        )
    });
    // arcoblue-6 #165DFF(sv-arco-tokens 金样钉过的值)
    assert_eq!(
        bg,
        Some(Color::rgb(22, 93, 255)),
        "primary 底色应是 arcoblue-6"
    );
    assert_eq!(fg, Some(Color::rgb(255, 255, 255)), "primary 文字应是白色");
    assert_eq!(h, Some(32.0), "default 尺寸高度应是 size-default");
}

#[test]
fn secondary_and_sizes_resolve() {
    let (doc, _) = make("secondary", "default", "mini", false);
    let id = the_button(&doc);
    let (bg, fg, h, fs) = doc.read(|i| {
        let s = &i.nodes[id].style;
        (s.bg, s.fg, s.height, s.font_size)
    });
    assert_eq!(
        bg,
        Some(Color::rgb(242, 243, 245)),
        "secondary 底色应是 fill-2/gray-2"
    );
    assert_eq!(
        fg,
        Some(Color::rgb(78, 89, 105)),
        "secondary 文字应是 text-2/gray-8"
    );
    assert_eq!(
        h,
        Some(24.0),
        "mini 高度应是 size-mini(条件类应压过静态 sz-default)"
    );
    assert_eq!(fs, 12.0, "mini 字号应是 body-1");
}

#[test]
fn danger_status_switches_palette() {
    let (doc, _) = make("primary", "danger", "default", false);
    let id = the_button(&doc);
    let bg = doc.read(|i| i.nodes[id].style.bg);
    assert_eq!(bg, Some(Color::rgb(245, 63, 63)), "danger 底色应是 red-6");
}

#[test]
fn outline_has_border_and_transparent_bg() {
    let (doc, _) = make("outline", "default", "default", false);
    let id = the_button(&doc);
    let (bg, border) = doc.read(|i| (i.nodes[id].style.bg, i.nodes[id].style.border));
    assert_eq!(bg, None, "outline 不应有底色");
    let b = border.expect("outline 应有边框");
    assert_eq!(
        b.color,
        Color::rgb(22, 93, 255),
        "outline 边框应是 primary-6"
    );
    assert_eq!(b.width, 1.0);
}

#[test]
fn click_fires_and_disabled_swallows() {
    let (doc, clicks) = make("primary", "default", "default", false);
    let handler = doc.click_handler(the_button(&doc)).expect("按钮应可点击");
    handler();
    handler();
    assert_eq!(clicks.get(), 2, "启用态两次点击都应触发");

    let (doc, clicks) = make("primary", "default", "default", true);
    if let Some(handler) = doc.click_handler(the_button(&doc)) {
        handler();
    }
    assert_eq!(clicks.get(), 0, "禁用态点击应被短路");
}

#[test]
fn disabled_recolors_via_conditional_class() {
    let (doc, _) = make("primary", "default", "default", true);
    let id = the_button(&doc);
    let (bg, cursor) = doc.read(|i| (i.nodes[id].style.bg, i.nodes[id].style.cursor));
    // primary-light-3 = arcoblue-3 #94BFFF
    assert_eq!(
        bg,
        Some(Color::rgb(148, 191, 255)),
        "禁用底色应是 arcoblue-3"
    );
    assert_eq!(
        cursor,
        Some(sv_ui::Cursor::NotAllowed),
        "禁用应显示 not-allowed 指针"
    );
}
