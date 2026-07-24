//! sv-arco 组件橱窗(A1 起步:Button 全矩阵)。
//!
//! 组件库对外是 Rust 函数 API(跨 crate 没有 `<Button>` 标签,见 sv-arco
//! lib 文档),这里就用它 —— 也顺带验收这条消费路径。
//!
//! 运行:`cargo run -p arco-gallery`
//! 离屏验证:`cargo run -p arco-gallery -- --png arco-gallery.png`

use std::rc::Rc;
use sv_arco::{ButtonProps, button};
use sv_ui::{AlignItems, Color, Direction, Doc, Edges, Style, ViewId};

fn btn(
    doc: &Doc,
    parent: ViewId,
    label: &str,
    variant: &str,
    status: &str,
    size: &str,
    disabled: bool,
) {
    button(
        doc,
        parent,
        ButtonProps {
            label: label.into(),
            variant: variant.into(),
            status: status.into(),
            size: size.into(),
            disabled,
            on_click: {
                let label = label.to_string();
                Rc::new(move || println!("点击:{label}"))
            },
        },
    );
}

fn section(doc: &Doc, parent: ViewId, title: &str) -> ViewId {
    let block = doc.create_view();
    doc.append(parent, block);
    doc.set_style(
        block,
        Style {
            gap: 10.0,
            ..Style::default()
        },
    );

    let t = doc.create_text(title);
    doc.append(block, t);
    doc.set_style(
        t,
        Style {
            font_size: 14.0,
            fg: Some(Color::rgb(29, 33, 41)), // color-text-1
            ..Style::default()
        },
    );

    let row = doc.create_view();
    doc.append(block, row);
    doc.set_style(
        row,
        Style {
            direction: Direction::Row,
            gap: 12.0,
            align_items: AlignItems::Center,
            ..Style::default()
        },
    );
    row
}

fn gallery(doc: &Doc, root: ViewId) {
    let page = doc.create_view();
    doc.append(root, page);
    doc.set_style(
        page,
        Style {
            padding: Edges::all(24.0),
            gap: 22.0,
            bg: Some(Color::rgb(255, 255, 255)),
            ..Style::default()
        },
    );

    let title = doc.create_text("sv-arco · Button(A1 全矩阵,arco.design 对齐)");
    doc.append(page, title);
    doc.set_style(
        title,
        Style {
            font_size: 20.0,
            fg: Some(Color::rgb(29, 33, 41)),
            ..Style::default()
        },
    );

    let variants = ["primary", "secondary", "outline", "text"];
    let labels = ["Primary", "Secondary", "Outline", "Text"];

    let row = section(doc, page, "变体 × default");
    for (v, l) in variants.iter().zip(labels) {
        btn(doc, row, l, v, "default", "default", false);
    }

    let row = section(doc, page, "变体 × disabled");
    for (v, l) in variants.iter().zip(labels) {
        btn(doc, row, l, v, "default", "default", true);
    }

    for (status, zh) in [("warning", "警告"), ("danger", "危险"), ("success", "成功")] {
        let row = section(doc, page, &format!("状态色:{zh}({status})"));
        for (v, l) in variants.iter().zip(labels) {
            btn(doc, row, l, v, status, "default", false);
        }
    }

    let row = section(doc, page, "尺寸:mini / small / default / large");
    for size in ["mini", "small", "default", "large"] {
        btn(doc, row, size, "primary", "default", size, false);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "arco-gallery.png".into());
        sv_shell::render_to_png(gallery, 1280, 1400, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv-arco 组件橱窗", gallery).expect("运行失败");
}
