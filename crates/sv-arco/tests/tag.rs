//! Tag 组件端到端行为(期望值出自 spec 的亮色 RGB 表,溯源 tag-token.less)。

use sv_arco::{TagProps, tag};
use sv_ui::{Color, Doc, ElementKind};

fn make(color: &str, size: &str) -> Doc {
    let doc = Doc::new();
    let d = doc.clone();
    let props = TagProps {
        label: "标签".into(),
        color: color.into(),
        size: size.into(),
    };
    let (_, _scope) = sv_reactive::create_root(move || tag(&d, d.root(), props));
    doc
}

/// tag 容器 = 第一个带底色的 view(root 无 bg)
fn tag_view(doc: &Doc) -> sv_ui::ViewId {
    doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::View && n.style.bg.is_some())
            .map(|(id, _)| id)
            .expect("应有 tag 容器")
    })
}

#[test]
fn default_gray_tag_matches_tokens() {
    let doc = make("gray", "default");
    let id = tag_view(&doc);
    let (bg, fg, h, fs, r) = doc.read(|i| {
        let s = &i.nodes[id].style;
        (s.bg, s.fg, s.height, s.font_size, s.corner_radius)
    });
    assert_eq!(bg, Some(Color::rgb(242, 243, 245)), "默认底应是 fill-2");
    assert_eq!(fg, Some(Color::rgb(29, 33, 41)), "默认字应是 text-1");
    assert_eq!(h, Some(24.0), "default 档高 = size-6");
    assert_eq!(fs, 12.0, "default 档字号 = body-1");
    assert_eq!(r, 2.0, "圆角 = radius-small");
}

#[test]
fn colored_tags_use_palette_1_and_6() {
    for (color, bg, fg) in [
        ("arcoblue", (232, 243, 255), (22, 93, 255)),
        // blue 与 arcoblue 是 arco 里真实存在的易混对,必须分别钉死
        ("blue", (232, 247, 255), (52, 145, 250)),
        ("red", (255, 236, 232), (245, 63, 63)),
        ("orangered", (255, 243, 232), (247, 114, 52)),
        ("orange", (255, 247, 232), (255, 125, 0)),
        ("gold", (255, 252, 232), (247, 186, 30)),
        ("lime", (252, 255, 232), (159, 219, 29)),
        ("green", (232, 255, 234), (0, 180, 42)),
        ("cyan", (232, 255, 251), (20, 201, 201)),
        ("purple", (245, 232, 255), (114, 46, 209)),
        ("pinkpurple", (255, 232, 251), (217, 26, 217)),
        ("magenta", (255, 232, 241), (245, 49, 157)),
        // yellow 是通式补齐档(arco 预设无 yellow)
        ("yellow", (254, 255, 232), (250, 220, 25)),
    ] {
        let doc = make(color, "default");
        let id = tag_view(&doc);
        let (b, f) = doc.read(|i| (i.nodes[id].style.bg, i.nodes[id].style.fg));
        assert_eq!(b, Some(Color::rgb(bg.0, bg.1, bg.2)), "{color} 底色");
        assert_eq!(f, Some(Color::rgb(fg.0, fg.1, fg.2)), "{color} 文字色");
    }
}

#[test]
fn four_sizes_resolve() {
    for (size, h, fs) in [
        ("small", 20.0, 12.0),
        ("default", 24.0, 12.0),
        ("medium", 28.0, 14.0),
        ("large", 32.0, 14.0),
    ] {
        let doc = make("gray", size);
        let id = tag_view(&doc);
        let (height, font) = doc.read(|i| (i.nodes[id].style.height, i.nodes[id].style.font_size));
        assert_eq!(height, Some(h), "{size} 高度");
        assert_eq!(font, fs, "{size} 字号");
    }
}

#[test]
fn label_lands_in_text_child() {
    let doc = make("gray", "default");
    assert!(doc.dump().contains("标签"), "label 应渲染:\n{}", doc.dump());
}
