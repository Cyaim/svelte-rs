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

#[test]
fn text_variant_wires_colors() {
    // text 变体上一批零覆盖(对抗评审查获):透明底、文字 primary-6,
    // hover/active 换 fill 底
    let (doc, _) = make("text", "default", "default", false);
    let id = the_button(&doc);
    let (bg, fg) = doc.read(|i| (i.nodes[id].style.bg, i.nodes[id].style.fg));
    assert_eq!(bg, None, "text 变体常态无底色");
    assert_eq!(fg, Some(Color::rgb(22, 93, 255)), "text 文字应是 primary-6");
}

#[test]
fn secondary_status_uses_light_1_bg() {
    // secondary×status 是另一套接线(bg=状态-1、文字=状态-6),与 primary
    // 的 bg=状态-6 不同——上一批只测了 primary×danger
    let (doc, _) = make("secondary", "danger", "default", false);
    let id = the_button(&doc);
    let (bg, fg) = doc.read(|i| (i.nodes[id].style.bg, i.nodes[id].style.fg));
    assert_eq!(
        bg,
        Some(Color::rgb(255, 236, 232)),
        "secondary danger 底 = danger-1"
    );
    assert_eq!(
        fg,
        Some(Color::rgb(245, 63, 63)),
        "secondary danger 文字 = danger-6"
    );
}

#[test]
fn secondary_disabled_and_remaining_sizes() {
    let (doc, _) = make("secondary", "default", "default", true);
    let id = the_button(&doc);
    let (bg, fg) = doc.read(|i| (i.nodes[id].style.bg, i.nodes[id].style.fg));
    assert_eq!(
        bg,
        Some(Color::rgb(247, 248, 250)),
        "secondary 禁用底 = fill-1/gray-1"
    );
    assert_eq!(
        fg,
        Some(Color::rgb(201, 205, 212)),
        "secondary 禁用文字 = text-4/gray-4"
    );

    for (size, h) in [("small", 28.0), ("large", 36.0)] {
        let (doc, _) = make("primary", "default", size, false);
        let got = doc.read(|i| i.nodes[the_button(&doc)].style.height);
        assert_eq!(got, Some(h), "{size} 高度");
    }
}

/// hover/active 三态色梯之前**编译期被丢弃**(条件类上的 :active),本批修了
/// codegen 才真正生效——这里离屏直调指针 handler 验证按压/悬停真的换色。
#[test]
fn hover_and_active_state_transitions() {
    let (doc, _) = make("primary", "default", "default", false);
    let id = the_button(&doc);
    let bg = |d: &Doc| d.read(|i| i.nodes[id].style.bg);

    assert_eq!(bg(&doc), Some(Color::rgb(22, 93, 255)), "初始 = arcoblue-6");

    doc.pointer_enter_handler(id).expect("应接悬停")();
    assert_eq!(
        bg(&doc),
        Some(Color::rgb(64, 128, 255)),
        "hover = arcoblue-5"
    );

    doc.pointer_down_handler(id).expect("应接按压")();
    assert_eq!(
        bg(&doc),
        Some(Color::rgb(14, 66, 210)),
        "active 压过 hover = arcoblue-7"
    );

    doc.pointer_up_handler(id).expect("应接抬起")();
    assert_eq!(
        bg(&doc),
        Some(Color::rgb(64, 128, 255)),
        "抬起后仍悬停 = arcoblue-5"
    );

    doc.pointer_leave_handler(id).expect("应接移出")();
    assert_eq!(
        bg(&doc),
        Some(Color::rgb(22, 93, 255)),
        "移出复位 = arcoblue-6"
    );
}

#[test]
fn disabled_button_ignores_hover() {
    // 禁用臂的 hover/active 被 !disabled 门控:禁用按钮悬停不应换色
    let (doc, _) = make("primary", "default", "default", true);
    let id = the_button(&doc);
    let disabled_bg = Some(Color::rgb(148, 191, 255)); // arcoblue-3
    if let Some(enter) = doc.pointer_enter_handler(id) {
        enter();
    }
    assert_eq!(
        doc.read(|i| i.nodes[id].style.bg),
        disabled_bg,
        "禁用态悬停不应换色"
    );
}
