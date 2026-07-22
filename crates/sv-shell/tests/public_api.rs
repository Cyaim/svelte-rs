//! **从外部 crate 的视角**检查公开 API 是不是真的可用。
//!
//! 集成测试是独立 crate,所以它看到的就是下游看到的 —— 这是单元测试
//! (在 crate 内部,私有模块随便访问)**结构上看不见**的一类缺陷。
//!
//! 起因:`paint.rs` 里的 `PathCmd` / `PathFill` / `StrokeStyle` / `LineCap` /
//! `LineJoin` 全都是 `pub`,`mod paint` 却是私有的,而 `lib.rs` 的 re-export
//! 列表漏了它们。于是 `Painter::fill_path`(pub trait 的 pub 方法)对外
//! **完全不可调用** —— 参数类型叫不出名字。
//! crate 内的 50 多条测试没有一条能发现它,因为它们都在 crate 内部。

use sv_shell::{
    LineCap, LineJoin, PaintCmd, Painter, PathCmd, PathFill, RecordingPainter, StrokeStyle,
};

/// 外部调用方能不能真的走完"构造路径 → 填充 → 描边"这条路
#[test]
fn path_api_is_usable_from_outside_the_crate() {
    let path = [
        PathCmd::MoveTo(0.0, 0.0),
        PathCmd::LineTo(10.0, 0.0),
        PathCmd::CubicTo(10.0, 5.0, 5.0, 10.0, 0.0, 10.0),
        PathCmd::Close,
    ];
    let mut p = RecordingPainter::default();
    p.fill_path(&path, PathFill::EvenOdd, sv_ui::Color::rgb(1, 2, 3));
    p.stroke_path(
        &path,
        &StrokeStyle {
            width: 2.0,
            cap: LineCap::Round,
            join: LineJoin::Bevel,
            miter_limit: 4.0,
        },
        sv_ui::Color::rgb(4, 5, 6),
    );

    // 命令流里两条都在,且能被外部模式匹配(PaintCmd 的字段也得是可达的)
    let kinds: Vec<&str> = p
        .cmds
        .iter()
        .map(|c| match c {
            PaintCmd::Path { .. } => "fill",
            PaintCmd::StrokePath { .. } => "stroke",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["fill", "stroke"]);
}
