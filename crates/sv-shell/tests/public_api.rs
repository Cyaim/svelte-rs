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
    LineCap, LineJoin, PaintCmd, Painter, PathCmd, PathFill, PixelImage, RecordingPainter,
    StrokeStyle,
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

/// **下游能不能自己实现一个后端** —— 上一条只查"名字叫得出",这条查"trait 实现得了"。
///
/// 为什么要单独一条:`Painter` 的动词一律**没有默认实现**(刻意的,免得新后端
/// 静默不画)。于是每加一个动词,只要它的参数类型没被 re-export,
/// 下游就**再也无法 `impl Painter`** —— 而只 import 类型名的测试看不见这一点:
/// 光 import 是能过的,直到你真的去写 `impl`。
///
/// 这一类洞已经连着出现两次(`PathCmd` 那五个、`PixelImage` 一个),
/// 两次都是"crate 内 50+ 条测试全绿、第一个外部消费者当场编译失败"。
/// 补这条编译型守卫,是想让第三次不要再发生。
#[test]
fn painter_can_be_implemented_by_a_downstream_crate() {
    /// 一个什么都不画的后端。**存在的意义就是"它能编译过"**
    struct CountingBackend {
        images: usize,
        paths: usize,
    }

    impl Painter for CountingBackend {
        fn caps(&self) -> sv_shell::PainterCaps {
            sv_shell::PainterCaps::default()
        }
        fn fill_rounded_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _radius: f32,
            _color: sv_ui::Color,
        ) {
        }
        #[allow(clippy::too_many_arguments)]
        fn stroke_rounded_rect(
            &mut self,
            _x: f32,
            _y: f32,
            _w: f32,
            _h: f32,
            _radius: f32,
            _width: f32,
            _color: sv_ui::Color,
        ) {
        }
        fn glyph_run(
            &mut self,
            _font: sv_shell::FontHandle,
            _glyphs: &[sv_shell::GlyphPos],
            _color: sv_ui::Color,
        ) {
        }
        fn push_clip(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _radius: f32) {}
        fn pop_clip(&mut self) {}
        fn fill_path(&mut self, _path: &[PathCmd], _fill: PathFill, _color: sv_ui::Color) {
            self.paths += 1;
        }
        fn stroke_path(&mut self, _path: &[PathCmd], _style: &StrokeStyle, _color: sv_ui::Color) {
            self.paths += 1;
        }
        fn draw_image(&mut self, _x: f32, _y: f32, _w: f32, _h: f32, _img: &PixelImage) {
            self.images += 1;
        }
    }

    let mut b = CountingBackend {
        images: 0,
        paths: 0,
    };
    let img = PixelImage::new(1, 1, vec![255u8, 0, 0, 255]).expect("1×1 应能构造");
    b.draw_image(0.0, 0.0, 1.0, 1.0, &img);
    b.fill_path(
        &[PathCmd::MoveTo(0.0, 0.0)],
        PathFill::NonZero,
        sv_ui::Color::rgb(0, 0, 0),
    );
    assert_eq!((b.images, b.paths), (1, 1));
}
