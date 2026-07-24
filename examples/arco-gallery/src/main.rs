//! sv-arco 组件橱窗(A1 静态件七件:Button/Tag/Badge/Divider/Alert/Typography/Link)。
//!
//! 组件库对外是 Rust 函数 API(跨 crate 没有 `<Button>` 标签,见 sv-arco
//! lib 文档),这里就用它 —— 也顺带验收这条消费路径。
//!
//! 运行:`cargo run -p arco-gallery`
//! 离屏验证:`cargo run -p arco-gallery -- --png arco-gallery.png`

use std::rc::Rc;
use sv_arco::{
    AlertProps, BadgeProps, ButtonProps, DividerProps, LinkProps, TagProps, TypographyProps, alert,
    badge, button, divider, link, tag, typography,
};
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

    let title = doc.create_text("sv-arco · A1 静态件(七组件,arco.design 对齐)");
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

    // ---- Typography ----
    let row = section(doc, page, "Typography:字号阶梯 + 色档");
    for kind in [
        "display-1",
        "title-3",
        "title-2",
        "title-1",
        "body",
        "caption",
    ] {
        typography(
            doc,
            row,
            TypographyProps {
                text: kind.into(),
                kind: kind.into(),
            },
        );
    }
    let row = section(doc, page, "");
    for kind in ["secondary", "primary", "success", "warning", "danger"] {
        typography(
            doc,
            row,
            TypographyProps {
                text: kind.into(),
                kind: kind.into(),
            },
        );
    }

    // ---- Tag ----
    let row = section(doc, page, "Tag:14 色板");
    doc.set_style(
        row,
        Style {
            direction: Direction::Row,
            gap: 12.0,
            align_items: AlignItems::Center,
            flex_wrap: sv_ui::FlexWrap::Wrap,
            max_width: Some(560.0),
            ..Style::default()
        },
    );
    for color in [
        "gray",
        "red",
        "orangered",
        "orange",
        "gold",
        "yellow",
        "lime",
        "green",
        "cyan",
        "blue",
        "arcoblue",
        "purple",
        "pinkpurple",
        "magenta",
    ] {
        tag(
            doc,
            row,
            TagProps {
                label: color.into(),
                color: color.into(),
                size: "default".into(),
            },
        );
    }
    let row = section(doc, page, "Tag:四档尺寸");
    for size in ["small", "default", "medium", "large"] {
        tag(
            doc,
            row,
            TagProps {
                label: size.into(),
                color: "arcoblue".into(),
                size: size.into(),
            },
        );
    }

    // ---- Badge ----
    let row = section(doc, page, "Badge:count / 99+ / dot / 换色");
    badge(
        doc,
        row,
        BadgeProps {
            count: 5,
            max_count: 99,
            dot: false,
            color: "danger".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 42,
            max_count: 99,
            dot: false,
            color: "danger".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 100,
            max_count: 99,
            dot: false,
            color: "danger".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 7,
            max_count: 99,
            dot: false,
            color: "arcoblue".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 7,
            max_count: 99,
            dot: false,
            color: "green".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 7,
            max_count: 99,
            dot: false,
            color: "gray".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 0,
            max_count: 99,
            dot: true,
            color: "danger".into(),
        },
    );
    badge(
        doc,
        row,
        BadgeProps {
            count: 0,
            max_count: 99,
            dot: true,
            color: "arcoblue".into(),
        },
    );

    // ---- Divider(横向形态要吃满宽,放 stretch 容器)----
    let block = doc.create_view();
    doc.append(page, block);
    doc.set_style(
        block,
        Style {
            align_items: AlignItems::Stretch,
            width: Some(560.0),
            ..Style::default()
        },
    );
    let t = doc.create_text("Divider:纯线 / 带字 / 纵向");
    doc.append(block, t);
    doc.set_style(
        t,
        Style {
            font_size: 14.0,
            fg: Some(Color::rgb(29, 33, 41)),
            ..Style::default()
        },
    );
    typography(
        doc,
        block,
        TypographyProps {
            text: "上一段".into(),
            kind: "body".into(),
        },
    );
    divider(
        doc,
        block,
        DividerProps {
            text: String::new(),
            vertical: false,
        },
    );
    typography(
        doc,
        block,
        TypographyProps {
            text: "下一段".into(),
            kind: "body".into(),
        },
    );
    divider(
        doc,
        block,
        DividerProps {
            text: "分组标题".into(),
            vertical: false,
        },
    );
    let vrow = doc.create_view();
    doc.append(block, vrow);
    doc.set_style(
        vrow,
        Style {
            direction: Direction::Row,
            align_items: AlignItems::Center,
            ..Style::default()
        },
    );
    typography(
        doc,
        vrow,
        TypographyProps {
            text: "左".into(),
            kind: "body".into(),
        },
    );
    divider(
        doc,
        vrow,
        DividerProps {
            text: String::new(),
            vertical: true,
        },
    );
    typography(
        doc,
        vrow,
        TypographyProps {
            text: "右".into(),
            kind: "body".into(),
        },
    );

    // ---- Alert(同样吃满宽)----
    let block = doc.create_view();
    doc.append(page, block);
    doc.set_style(
        block,
        Style {
            align_items: AlignItems::Stretch,
            width: Some(560.0),
            gap: 10.0,
            ..Style::default()
        },
    );
    let t = doc.create_text("Alert:四状态 + 有/无标题");
    doc.append(block, t);
    doc.set_style(
        t,
        Style {
            font_size: 14.0,
            fg: Some(Color::rgb(29, 33, 41)),
            ..Style::default()
        },
    );
    alert(
        doc,
        block,
        AlertProps {
            title: String::new(),
            content: "info:这是一条无标题提示".into(),
            status: "info".into(),
        },
    );
    alert(
        doc,
        block,
        AlertProps {
            title: "成功".into(),
            content: "success:操作已完成,数据已保存。".into(),
            status: "success".into(),
        },
    );
    alert(
        doc,
        block,
        AlertProps {
            title: "注意".into(),
            content: "warning:磁盘空间不足 10%。".into(),
            status: "warning".into(),
        },
    );
    alert(
        doc,
        block,
        AlertProps {
            title: "出错了".into(),
            content: "error:网络连接失败,请稍后重试。".into(),
            status: "error".into(),
        },
    );

    // ---- Link ----
    let row = section(doc, page, "Link:四状态 + 禁用");
    for status in ["default", "success", "warning", "danger"] {
        link(
            doc,
            row,
            LinkProps {
                label: format!("链接-{status}"),
                status: status.into(),
                disabled: false,
                on_click: Rc::new(|| {}),
            },
        );
    }
    for status in ["default", "warning"] {
        link(
            doc,
            row,
            LinkProps {
                label: format!("禁用-{status}"),
                status: status.into(),
                disabled: true,
                on_click: Rc::new(|| {}),
            },
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "arco-gallery.png".into());
        sv_shell::render_to_png(gallery, 1280, 3400, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv-arco 组件橱窗", gallery).expect("运行失败");
}
