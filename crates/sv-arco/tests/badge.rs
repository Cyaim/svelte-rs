//! Badge 组件端到端行为(期望值出自 spec 的亮色 RGB 表,溯源 badge-token.less)。

use sv_arco::{BadgeProps, badge};
use sv_ui::{Color, Doc, ElementKind};

fn make(count: i64, max_count: i64, dot: bool, color: &str) -> Doc {
    let doc = Doc::new();
    let d = doc.clone();
    let props = BadgeProps {
        count,
        max_count,
        dot,
        color: color.into(),
    };
    let (_, _scope) = sv_reactive::create_root(move || badge(&d, d.root(), props));
    doc
}

fn badge_view(doc: &Doc) -> Option<sv_ui::ViewId> {
    doc.read(|i| {
        i.nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::View && n.style.bg.is_some())
            .map(|(id, _)| id)
    })
}

#[test]
fn count_pill_matches_tokens() {
    let doc = make(5, 99, false, "danger");
    let id = badge_view(&doc).expect("count>0 应渲染胶囊");
    let (bg, h, r, minw) = doc.read(|i| {
        let s = &i.nodes[id].style;
        (s.bg, s.height, s.corner_radius, s.min_width)
    });
    assert_eq!(bg, Some(Color::rgb(245, 63, 63)), "默认底 = danger-6");
    assert_eq!(h, Some(20.0), "胶囊高 = size-5");
    assert_eq!(r, 10.0, "radius = 高/2");
    assert_eq!(minw, Some(20.0), "min-width = 高(单字符正圆)");
    let (text, fg, fs) = doc.read(|i| {
        let (_, n) = i
            .nodes
            .iter()
            .find(|(_, n)| n.kind == ElementKind::Text)
            .expect("应有数字文本");
        (n.text.clone(), n.style.fg, n.style.font_size)
    });
    assert_eq!(text, "5");
    assert_eq!(fg, Some(Color::rgb(255, 255, 255)), "白字");
    assert_eq!(fs, 12.0, "body-1");
}

#[test]
fn overflow_shows_max_plus() {
    let doc = make(100, 99, false, "danger");
    assert!(
        doc.dump().contains("99+"),
        "超上限应显示 99+:\n{}",
        doc.dump()
    );
}

#[test]
fn zero_count_renders_nothing() {
    let doc = make(0, 99, false, "danger");
    assert!(badge_view(&doc).is_none(), "count=0 且非 dot 不应渲染");
    let texts = doc.read(|i| {
        i.nodes
            .iter()
            .filter(|(_, n)| n.kind == ElementKind::Text)
            .count()
    });
    assert_eq!(texts, 0, "也不应有文本节点");
}

#[test]
fn dot_is_six_px_circle() {
    let doc = make(0, 99, true, "danger");
    let id = badge_view(&doc).expect("dot 应渲染");
    let (w, h, r) = doc.read(|i| {
        let s = &i.nodes[id].style;
        (s.width, s.height, s.corner_radius)
    });
    assert_eq!((w, h), (Some(6.0), Some(6.0)), "6×6 圆点");
    assert_eq!(r, 3.0);
}

#[test]
fn color_wiring_matches_token_quirks() {
    // 怪癖档:red 与 danger 同值(token L19);green→success-6;arcoblue→primary-6;
    // gray→gray-4(非 6)。加上直通色板档(orangered..magenta,上一批零覆盖)。
    for (color, rgb) in [
        ("red", (245, 63, 63)),
        ("green", (0, 180, 42)),
        ("arcoblue", (22, 93, 255)),
        ("gray", (201, 205, 212)),
        ("orangered", (247, 114, 52)),
        ("orange", (255, 125, 0)),
        ("gold", (247, 186, 30)),
        ("lime", (159, 219, 29)),
        ("cyan", (20, 201, 201)),
        ("purple", (114, 46, 209)),
        ("pinkpurple", (217, 26, 217)),
        ("magenta", (245, 49, 157)),
    ] {
        for dot in [false, true] {
            let doc = make(3, 99, dot, color);
            let id = badge_view(&doc).expect("应渲染");
            let bg = doc.read(|i| i.nodes[id].style.bg);
            assert_eq!(
                bg,
                Some(Color::rgb(rgb.0, rgb.1, rgb.2)),
                "{color} dot={dot}"
            );
        }
    }
}

#[test]
fn count_equals_max_shows_plain_number() {
    // count == max_count 边界:99/99 应显 "99" 而非 "99+"(钉死 > 不是 >=)
    let doc = make(99, 99, false, "danger");
    let dump = doc.dump();
    assert!(dump.contains("99"), "应含 99:\n{dump}");
    assert!(!dump.contains("99+"), "等于上限不应加 +:\n{dump}");
}

#[test]
fn dot_with_positive_count_still_renders_circle() {
    // dot 优先于 count:即便 count>0,dot=true 也应渲染 6×6 圆点(无文本)
    let doc = make(5, 99, true, "danger");
    let id = badge_view(&doc).expect("dot 应渲染");
    let (w, h) = doc.read(|i| (i.nodes[id].style.width, i.nodes[id].style.height));
    assert_eq!((w, h), (Some(6.0), Some(6.0)), "dot 优先,应是圆点不是胶囊");
    let texts = doc.read(|i| {
        i.nodes
            .iter()
            .filter(|(_, n)| n.kind == ElementKind::Text)
            .count()
    });
    assert_eq!(texts, 0, "dot 态无数字文本");
}
