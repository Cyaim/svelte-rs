//! Typography 组件端到端行为(期望值溯源 typography-token.less)。

use sv_arco::{TypographyProps, typography};
use sv_ui::{Color, Doc, ElementKind};

fn make(kind: &str) -> Doc {
    let doc = Doc::new();
    let d = doc.clone();
    let props = TypographyProps {
        text: "文本".into(),
        kind: kind.into(),
    };
    let (_, _scope) = sv_reactive::create_root(move || typography(&d, d.root(), props));
    doc
}

fn style_of(doc: &Doc) -> (Option<Color>, f32) {
    doc.read(|i| {
        let (_, n) = i
            .nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Text)
            .expect("应有 text 叶");
        (n.style.fg, n.style.font_size)
    })
}

const TEXT1: Color = Color {
    r: 29,
    g: 33,
    b: 41,
    a: 255,
};

#[test]
fn size_ladder_matches_tokens() {
    for (kind, fs) in [
        ("caption", 12.0),
        ("body", 14.0),
        ("title-1", 16.0),
        ("title-2", 20.0),
        ("title-3", 24.0),
        ("display-1", 36.0),
        ("display-2", 48.0),
        ("display-3", 56.0),
    ] {
        let (fg, size) = style_of(&make(kind));
        assert_eq!(size, fs, "{kind} 字号");
        assert_eq!(fg, Some(TEXT1), "{kind} 色应保持 text-1");
    }
}

#[test]
fn color_kinds_match_tokens() {
    for (kind, rgb) in [
        ("secondary", (78, 89, 105)), // text-2(token L17 原文,非 text-3)
        ("primary", (22, 93, 255)),
        ("success", (0, 180, 42)),
        ("warning", (255, 125, 0)),
        ("danger", (245, 63, 63)),
    ] {
        let (fg, size) = style_of(&make(kind));
        assert_eq!(fg, Some(Color::rgb(rgb.0, rgb.1, rgb.2)), "{kind} 色");
        assert_eq!(size, 14.0, "{kind} 字号保持 body-3");
    }
}
