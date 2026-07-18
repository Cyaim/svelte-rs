//! 设置面板示例:R2 验收 demo(调研 22/23)。
//!
//! 覆盖:`overflow: scroll` 容器 + `bind:scrolly`、flex 对齐
//! (justify-content/align-items/min-width)、长文本折行(CJK 断点)、
//! `<input>`/`<checkbox>` 表单件、Tab 焦点遍历。
//!
//! 运行:`cargo run -p settings-sfc`
//! 离屏:`cargo run -p settings-sfc -- --png out.png`

include!(concat!(env!("OUT_DIR"), "/settings.rs"));

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let path = args
            .get(i + 1)
            .cloned()
            .unwrap_or_else(|| "settings.png".into());
        let doc = sv_ui::Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || settings(&d, d.root()));
        // 模拟滚动一段距离,截图应可见滚动条与偏移后的内容
        let layout = sv_shell::layout_tree_full(&doc, 520.0, 400.0);
        sv_shell::route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            200.0,
            200.0,
            0.0,
            120.0,
        );
        sv_shell::render_doc_to_png(&doc, 1040, 800, 2.0, &path).expect("离屏渲染失败");
        println!("已渲染到 {path}");
        return;
    }
    sv_shell::run_app("sv 设置面板(R2 验收)", |doc, root| {
        settings(doc, root)
    })
    .expect("运行失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_ui::Doc;

    /// R2 验收(DESIGN.md §5):超一屏可滚 + flex 对齐 + 长文本折行
    #[test]
    fn settings_panel_r2_acceptance() {
        let doc = Doc::new();
        let d = doc.clone();
        let (_, _scope) = sv_reactive::create_root(move || settings(&d, d.root()));

        let layout = sv_shell::layout_tree_full(&doc, 520.0, 400.0);
        // 超一屏:滚动区内容高于视口
        let area = layout.scroll_areas.first().expect("应有滚动容器");
        assert!(
            area.max.1 > 50.0,
            "内容应超出一屏(max_y={}),内容高 {}",
            area.max.1,
            area.content.1
        );
        // 折行:说明文字节点高度应超过两行
        let tall_text = doc.read(|inner| {
            inner
                .nodes
                .iter()
                .filter(|(_, n)| n.kind == sv_ui::ElementKind::Text && n.text.contains("折行"))
                .count()
        });
        assert!(tall_text > 0, "应有折行验收文本");
        let wrapped = layout
            .placed
            .iter()
            .filter(|p| {
                doc.read(|inner| {
                    inner
                        .nodes
                        .get(p.id)
                        .is_some_and(|n| n.text.contains("强制折断"))
                })
            })
            .any(|p| p.rect.h > 40.0);
        assert!(wrapped, "长说明文字应折成多行(高度 > 两行)");

        // 滚动 → bind:scrolly 联动(标题行的滚动位置指示更新)
        sv_shell::route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            200.0,
            200.0,
            0.0,
            80.0,
        );
        assert!(
            doc.dump().contains("滚动位置 80px"),
            "bind:scrolly 应联动:\n{}",
            doc.dump()
        );
    }
}
