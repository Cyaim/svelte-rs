//! 动画内容注册表:把 `sv_ui::AnimSource` 里那个不透明句柄换成真正的像素。
//!
//! # 为什么要有这一层
//!
//! sv-ui 是双前端的编译目标,依赖面必须干净 —— 它**不能**认识一张解码后的位图,
//! 也不能认识 `velato::Composition`。所以场景树里只留 `handle: u64`,
//! 内容存在渲染壳这一侧。`text.rs` 的 `FontHandle` 是同款先例。
//!
//! 这层顺带买到一件事:**同一份素材被多个节点引用时只存一份**。
//! 一个列表里 200 行都放同一个 loading 动画,像素只在这里躺一份。
//!
//! # 它**不做**解码
//!
//! 注册表收的是**已经解码好的** [`PixelImage`]。PAG 的位图序列帧在容器里是
//! WebP(见 `sv-pag` 的 README),Lottie 的图像图层是 PNG/JPEG ——
//! 解码要引第三方 crate,而"引哪个解码器"是独立于本文件的一次裁决。
//! 把解码挡在外面,这层就只有一个职责,也不必跟着解码器的版本走。

use std::cell::RefCell;
use std::collections::HashMap;

use crate::paint::{LineCap, LineJoin, Painter, PathCmd, PathFill, PixelImage, StrokeStyle};

/// 一份已解码的帧序列。
///
/// **注意它是"逐帧独立的成品帧",不是 PAG 容器里那种差分帧。**
/// PAG 的位图序列是关键帧 + 脏矩形差分,必须从最近关键帧重放才能还原一帧
/// (见 `sv-pag` README 的四步流程)。重放是导入侧的事;
/// 到了这里,每一帧都必须是可以直接贴上去的完整画面。
struct Frames {
    frames: Vec<PixelImage>,
}

/// 一份已解析的矢量资产(Lottie)。
///
/// 持有 `sv_lottie::Lottie`(内含 velato `Renderer` 的跨帧几何批缓冲),
/// 所以求值一帧要 `&mut`,与位图档"取一张现成的图"不同 —— 矢量档是**每帧现算**。
/// `start_frame` 缓存自时间轴,用来把场景树的帧号(0 基索引)映射到 lottie 帧号。
struct VectorAsset {
    lottie: sv_lottie::Lottie,
    start_frame: f64,
    end_frame: f64,
}

thread_local! {
    /// 句柄 → 内容。**thread_local 是刻意的**:场景树本来就是单线程模型
    /// (ADR-1),句柄也就没有跨线程的意义
    static STORE: RefCell<HashMap<u64, Frames>> = RefCell::new(HashMap::new());
    /// 矢量档(Lottie)注册表,与位图 `STORE` 分开存但**共用 `NEXT` 号段**,
    /// 于是句柄全局唯一;`AnimSource` 的变体本身已能区分该查哪张表
    static VSTORE: RefCell<HashMap<u64, VectorAsset>> = RefCell::new(HashMap::new());
    /// 句柄分配器。**从 1 起** —— 0 留给 `AnimData::placeholder()`,
    /// 于是"忘了接素材"与"接了但注册表里没有"是两种可区分的状态
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_handle() -> u64 {
    NEXT.with(|n| {
        let h = n.get();
        n.set(h + 1);
        h
    })
}

/// 注册一段已解码的帧序列,返回给场景树用的句柄。
///
/// 空序列也接受并返回句柄:一个还没加载完的动画是合法状态,
/// 它只是暂时画不出东西 —— 拒绝它会逼调用方自己发明一个"待定"表示。
pub fn register_frames(frames: Vec<PixelImage>) -> u64 {
    let handle = next_handle();
    STORE.with(|s| s.borrow_mut().insert(handle, Frames { frames }));
    handle
}

/// 注册一份 Lottie 矢量资产,返回给 [`sv_ui::AnimSource::Vector`] 用的句柄。
///
/// 与位图档不同,矢量档不预解码成帧序列 —— 每帧由 velato 现算路径,
/// 经 `render_vector` 直接发到宿主 `Painter`,不落位图(省内存、缩放无损)。
pub fn register_vector(lottie: sv_lottie::Lottie) -> u64 {
    let handle = next_handle();
    let tl = lottie.timeline();
    VSTORE.with(|s| {
        s.borrow_mut().insert(
            handle,
            VectorAsset {
                lottie,
                start_frame: tl.start_frame,
                end_frame: tl.end_frame,
            },
        )
    });
    handle
}

/// 注销一份矢量资产。语义同 [`unregister`]:所有权由调用方显式管理。
pub fn unregister_vector(handle: u64) -> bool {
    VSTORE.with(|s| s.borrow_mut().remove(&handle).is_some())
}

/// 注册一段 PAG 位图序列:**差分帧重放**成逐帧成品图,进位图 `Frames` 注册表,
/// 返回句柄(节点用 `AnimSource::Frames`)。
///
/// `decode` 把每个块的编码字节(WebP)解成 [`sv_pag::DecodedImage`] —— **壳侧不绑
/// 图片解码器**(与 sv-pag 零依赖同因),解码器由调用方注入(平台强相关的一次
/// 独立裁决)。任一帧重放 / 解码 / 构图失败,整段拒绝返回 `None`(不给半张动画)。
pub fn register_pag<F>(seq: &sv_pag::BitmapSequence, decode: F) -> Option<u64>
where
    F: Fn(&[u8]) -> Option<sv_pag::DecodedImage>,
{
    let mut frames = Vec::with_capacity(seq.frames.len());
    for i in 0..seq.frames.len() {
        let img = sv_pag::replay_frame(seq, i, &decode)?;
        // DecodedImage 是直通 RGBA;PixelImage 契约是**预乘** —— 转一下
        // (不透明像素 alpha=255 时是恒等,所以纯不透明素材零损耗)。
        let premul = premultiply(&img.rgba);
        frames.push(PixelImage::new(img.width, img.height, premul)?);
    }
    Some(register_frames(frames))
}

/// 用内置的纯 Rust WebP 解码器(`image-webp`)注册一段 PAG 位图序列。
///
/// 这是 [`register_pag`] 的便利封装:PAG 位图序列帧块导出即 WebP
/// (`sv-pag` README 的核实表),这里把解码器定死为 `image-webp`,调用方不必自带。
/// 需要别的编码(PNG/JPEG)或别的解码器时,直接用 [`register_pag`] 注入自己的。
pub fn register_pag_webp(seq: &sv_pag::BitmapSequence) -> Option<u64> {
    register_pag(seq, decode_webp)
}

/// 单块解码后的最大边(像素)。**这是对不可信输入的护栏**:WebP 的宽高来自
/// 文件头,一个畸形块声称 65535×65535 会让下面 `output_buffer_size` 算出约 16GB,
/// `vec![0u8; …]` 直接把进程 OOM 掉——那是拒绝服务,不是"解不了"。8192 与 vello
/// 图集上限同量级,UI 动画帧远用不到;超了当畸形拒绝(返回 None)。
const MAX_WEBP_DIM: u32 = 8192;

/// WebP 字节 → [`sv_pag::DecodedImage`](直通 RGBA8)。解不了 / 超尺寸返回 `None`。
fn decode_webp(bytes: &[u8]) -> Option<sv_pag::DecodedImage> {
    let mut dec = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let (width, height) = dec.dimensions();
    // 分配前先卡尺寸:不可信的巨大宽高会变成 GB 级分配(内存炸弹)
    if width == 0 || height == 0 || width > MAX_WEBP_DIM || height > MAX_WEBP_DIM {
        return None;
    }
    let mut buf = vec![0u8; dec.output_buffer_size()?];
    dec.read_image(&mut buf).ok()?;
    // has_alpha → 已是 RGBA;否则是 RGB,补 alpha=255 铺成 RGBA
    let rgba = if dec.has_alpha() {
        buf
    } else {
        buf.chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    };
    Some(sv_pag::DecodedImage {
        width,
        height,
        rgba,
    })
}

/// 直通 RGBA8 → 预乘 RGBA8(`c' = c*a/255`,四舍五入)。
fn premultiply(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u16;
            let m = |c: u8| ((c as u16 * a + 127) / 255) as u8;
            [m(p[0]), m(p[1]), m(p[2]), p[3]]
        })
        .collect()
}

/// 注销。**调用方必须自己管**:注册表不知道场景树里还有没有节点引用它。
///
/// 不做引用计数是因为句柄可以被自由复制(它就是个 u64),
/// 计数会立刻变成"谁该减一"的糊涂账。宁可让所有权显式。
pub fn unregister(handle: u64) -> bool {
    STORE.with(|s| s.borrow_mut().remove(&handle).is_some())
}

/// 某个句柄有多少帧(句柄不存在返回 0)
pub fn frame_count(handle: u64) -> u32 {
    STORE.with(|s| {
        s.borrow()
            .get(&handle)
            .map_or(0, |f| f.frames.len().min(u32::MAX as usize) as u32)
    })
}

/// 取某一帧。返回 `PixelImage` 的克隆 —— 像素是 `Arc<[u8]>`,克隆只加引用计数。
///
/// 越界帧号返回 `None` 而**不是**钳到最后一帧:钳会让"帧号算错"表现为
/// "动画卡在最后一帧",那是个会被当成素材问题查半天的假象。
pub fn frame(handle: u64, index: u32) -> Option<PixelImage> {
    STORE.with(|s| {
        s.borrow()
            .get(&handle)
            .and_then(|f| f.frames.get(index as usize))
            .cloned()
    })
}

/// 场景树上的一个动画节点当前该画哪张**位图**。
///
/// 矢量档([`sv_ui::AnimSource::Vector`])在这里恒返回 `None` —— 它不产生位图,
/// 走 [`render_vector`] 每帧现算路径命令。**给它一个占位位图是错的**:
/// 那会让"矢量还没接"看起来像"接上了但内容是灰的",两者查的方向完全不同。
pub(crate) fn image_for(anim: &sv_ui::AnimData) -> Option<PixelImage> {
    match anim.source {
        sv_ui::AnimSource::Frames { handle } => frame(handle, anim.frame),
        sv_ui::AnimSource::Vector { .. } => None,
    }
}

/// `sv_lottie::PathSink` → 宿主 `Painter` 的桥。
///
/// `sv-lottie` 为了不反向依赖 `sv-shell`,自带一套**同名同形**的路径动词
/// (`path.rs` 的注释)。sv-shell 现在已把 `PathCmd`/`PathFill`/… 从
/// `paint` re-export 出来,所以这里就是那句预言里的"纯搬运 `for` 循环":
/// 把 lottie 的动词逐个转成 `Painter` 的同形动词。裁剪成对转发,保证宿主
/// 裁剪栈平衡(velato 每帧开头压一次覆盖画布的裁剪,遮罩也走这里)。
struct PainterSink<'a, P: Painter + ?Sized> {
    painter: &'a mut P,
}

fn conv_cmd(c: &sv_lottie::PathCmd) -> PathCmd {
    match *c {
        sv_lottie::PathCmd::MoveTo(x, y) => PathCmd::MoveTo(x, y),
        sv_lottie::PathCmd::LineTo(x, y) => PathCmd::LineTo(x, y),
        sv_lottie::PathCmd::QuadTo(cx, cy, x, y) => PathCmd::QuadTo(cx, cy, x, y),
        sv_lottie::PathCmd::CubicTo(a, b, c, d, x, y) => PathCmd::CubicTo(a, b, c, d, x, y),
        sv_lottie::PathCmd::Close => PathCmd::Close,
    }
}

fn conv_fill(f: sv_lottie::PathFill) -> PathFill {
    match f {
        sv_lottie::PathFill::NonZero => PathFill::NonZero,
        sv_lottie::PathFill::EvenOdd => PathFill::EvenOdd,
    }
}

fn conv_stroke(s: &sv_lottie::StrokeStyle) -> StrokeStyle {
    StrokeStyle {
        width: s.width,
        cap: match s.cap {
            sv_lottie::LineCap::Butt => LineCap::Butt,
            sv_lottie::LineCap::Round => LineCap::Round,
            sv_lottie::LineCap::Square => LineCap::Square,
        },
        join: match s.join {
            sv_lottie::LineJoin::Miter => LineJoin::Miter,
            sv_lottie::LineJoin::Round => LineJoin::Round,
            sv_lottie::LineJoin::Bevel => LineJoin::Bevel,
        },
        miter_limit: s.miter_limit,
    }
}

impl<P: Painter + ?Sized> sv_lottie::PathSink for PainterSink<'_, P> {
    fn fill_path(
        &mut self,
        path: &[sv_lottie::PathCmd],
        fill: sv_lottie::PathFill,
        color: sv_ui::Color,
    ) {
        let cmds: Vec<PathCmd> = path.iter().map(conv_cmd).collect();
        self.painter.fill_path(&cmds, conv_fill(fill), color);
    }

    fn stroke_path(
        &mut self,
        path: &[sv_lottie::PathCmd],
        style: &sv_lottie::StrokeStyle,
        color: sv_ui::Color,
    ) {
        let cmds: Vec<PathCmd> = path.iter().map(conv_cmd).collect();
        self.painter.stroke_path(&cmds, &conv_stroke(style), color);
    }

    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        // Painter 的 push_clip 带圆角半径;lottie 的裁剪是直角矩形 → radius 0
        self.painter.push_clip(x, y, w, h, 0.0);
    }

    fn pop_clip(&mut self) {
        self.painter.pop_clip();
    }
}

/// 把一个矢量动画节点的当前帧画到 `painter` 的内容盒(物理像素矩形)里。
///
/// - `frame_index`:场景树的 0 基帧号(`AnimData.frame`);内部映射到 lottie 帧号。
/// - `alpha`:节点不透明度(0..=1),乘进每个画刷。
/// - 内容按 `object-fit: contain` 等比居中(见 `Lottie::fit_contain`)。
///
/// 句柄不存在(还没注册 / 已注销)就什么都不画 —— 与位图档"素材没接上"同款静默。
/// 返回是否真的画了(便于测试与将来的脏矩形判断)。
///
/// `rect` 是内容盒 `(x, y, w, h)`(物理像素);内容按 `object-fit: contain` 摆进去。
pub(crate) fn render_vector(
    handle: u64,
    frame_index: u32,
    rect: (f32, f32, f32, f32),
    alpha: f32,
    painter: &mut dyn Painter,
) -> bool {
    let (x, y, w, h) = rect;
    VSTORE.with(|s| {
        let mut store = s.borrow_mut();
        let Some(asset) = store.get_mut(&handle) else {
            return false;
        };
        let place = asset.lottie.fit_contain(x, y, w, h);
        // 帧号映射:场景树给 0 基索引,lottie 时间轴从 start_frame 起。
        // 越界不崩(velato 逐图层按活跃区间判),但钳进 [start, end) 更省无用求值。
        let frame = (asset.start_frame + frame_index as f64)
            .clamp(asset.start_frame, asset.end_frame.max(asset.start_frame));
        let mut sink = PainterSink { painter };
        asset.lottie.render(frame, place, alpha, &mut sink);
        true
    })
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    STORE.with(|s| s.borrow_mut().clear());
    VSTORE.with(|s| s.borrow_mut().clear());
    NEXT.with(|n| n.set(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> PixelImage {
        let px: Vec<u8> = (0..(w * h)).flat_map(|_| rgba).collect();
        PixelImage::new(w, h, px).expect("固件应能构造")
    }

    #[test]
    fn register_lookup_and_unregister() {
        reset_for_test();
        let h = register_frames(vec![
            solid(2, 2, [255, 0, 0, 255]),
            solid(2, 2, [0, 255, 0, 255]),
        ]);
        assert_eq!(frame_count(h), 2);
        assert!(frame(h, 0).is_some());
        assert!(frame(h, 1).is_some());
        // 越界不钳:钳会把"帧号算错"伪装成"动画卡住"
        assert!(frame(h, 2).is_none());
        // 两帧内容确实不同(否则下面的渲染测试等于没测)
        assert_ne!(frame(h, 0).unwrap().id(), frame(h, 1).unwrap().id());

        assert!(unregister(h));
        assert!(!unregister(h), "重复注销应返回 false,不是 panic");
        assert_eq!(frame_count(h), 0);
        assert!(frame(h, 0).is_none());
    }

    #[test]
    fn handles_are_distinct_and_never_zero() {
        reset_for_test();
        let a = register_frames(vec![solid(1, 1, [1, 2, 3, 255])]);
        let b = register_frames(vec![solid(1, 1, [4, 5, 6, 255])]);
        assert_ne!(a, b);
        // 0 是 placeholder 的句柄:注册表永不发这个号,
        // 于是"忘了接素材"与"接了但注册表里没有"可区分
        assert!(a > 0 && b > 0);
        assert_eq!(frame_count(0), 0);
    }

    #[test]
    fn empty_sequence_is_a_valid_state() {
        reset_for_test();
        let h = register_frames(Vec::new());
        assert!(h > 0, "还没加载完的动画是合法状态,不该被拒");
        assert_eq!(frame_count(h), 0);
        assert!(frame(h, 0).is_none());
    }

    // 一个合法的最小 Lottie:200×100 画布,一个填充圆(velato 能解析并渲染)
    const LOTTIE: &str = r#"{
      "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
      "layers": [
        { "ddd": 0, "ty": 4, "ind": 1, "nm": "dot", "sr": 1, "st": 0, "ip": 0, "op": 60,
          "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[100,50]},
                  "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
          "shapes": [
            { "ty": "gr", "nm": "g", "it": [
              { "ty": "el", "nm": "e", "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[60,60]} },
              { "ty": "fl", "nm": "f", "o": {"a":0,"k":100}, "c": {"a":0,"k":[1,0,0]} },
              { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
                "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
            ] }
          ] }
      ]
    }"#;

    #[test]
    fn vector_registers_and_renders_paths_into_the_painter() {
        use crate::paint::{PaintCmd, RecordingPainter};
        reset_for_test();
        let lottie = sv_lottie::Lottie::from_json_str(LOTTIE).expect("固件应是合法 lottie");
        let h = register_vector(lottie);
        assert!(h > 0);

        // 渲染到记录型 Painter:矢量档应发出至少一条 fill 路径命令(那个圆)
        let mut painter = RecordingPainter::default();
        let drawn = render_vector(h, 0, (0.0, 0.0, 200.0, 100.0), 1.0, &mut painter);
        assert!(drawn, "已注册的句柄应当画出东西");
        let fills = painter
            .cmds
            .iter()
            .filter(|c| matches!(c, PaintCmd::Path { .. }))
            .count();
        assert!(fills >= 1, "圆的填充应至少产生一条 Path 命令,实得 {fills}");

        // 裁剪栈必须平衡(velato 每帧开头压一次覆盖画布的裁剪,漏 pop 会污染宿主)
        let pushes = painter
            .cmds
            .iter()
            .filter(|c| matches!(c, PaintCmd::PushClip { .. }))
            .count();
        let pops = painter
            .cmds
            .iter()
            .filter(|c| matches!(c, PaintCmd::PopClip))
            .count();
        assert_eq!(pushes, pops, "push/pop 裁剪必须成对,否则宿主裁剪栈被污染");
    }

    #[test]
    fn pag_sequence_replays_into_frames() {
        use sv_pag::{BitmapFrame, BitmapRect, BitmapSequence, DecodedImage};
        reset_for_test();
        // 假解码器:标记 → 纯色 s×s(脱离真 WebP 解码器验证"重放 → 注册"这条链)
        let decode = |b: &[u8]| -> Option<DecodedImage> {
            let color = match b[0] {
                b'R' => [255u8, 0, 0, 255],
                b'G' => [0, 255, 0, 255],
                _ => return None,
            };
            let s = b[1] as u32;
            Some(DecodedImage {
                width: s,
                height: s,
                rgba: (0..s * s).flat_map(|_| color).collect(),
            })
        };
        // 关键帧全红 4×4 + 差分帧 (1,1) 2×2 绿
        let seq = BitmapSequence {
            width: 4,
            height: 4,
            frame_rate: 30.0,
            frames: vec![
                BitmapFrame {
                    is_keyframe: true,
                    bitmaps: vec![BitmapRect {
                        x: 0,
                        y: 0,
                        bytes: b"R\x04",
                    }],
                },
                BitmapFrame {
                    is_keyframe: false,
                    bitmaps: vec![BitmapRect {
                        x: 1,
                        y: 1,
                        bytes: b"G\x02",
                    }],
                },
            ],
        };
        let h = register_pag(&seq, decode).expect("应重放并注册");
        assert_eq!(frame_count(h), 2, "两帧都该进注册表");
        // 两帧内容不同(差分帧覆盖了绿块)
        assert_ne!(
            frame(h, 0).unwrap().id(),
            frame(h, 1).unwrap().id(),
            "关键帧与差分帧应是不同的图"
        );

        // 解码器不认某块 → 整段拒绝
        let bad = BitmapSequence {
            width: 4,
            height: 4,
            frame_rate: 30.0,
            frames: vec![BitmapFrame {
                is_keyframe: true,
                bitmaps: vec![BitmapRect {
                    x: 0,
                    y: 0,
                    bytes: b"X\x04",
                }],
            }],
        };
        assert!(register_pag(&bad, decode).is_none(), "解不了应整段拒绝");
    }

    #[test]
    fn pag_webp_decodes_real_frames_end_to_end() {
        use sv_pag::{BitmapFrame, BitmapRect, BitmapSequence};
        reset_for_test();

        // 用 image-webp 真编码两张纯色 WebP(2×2 红、2×2 绿)—— 真解码器、真字节,
        // 不是假解码器。验证 decode_webp + register_pag_webp 整条链在真 WebP 上跑通。
        fn webp(color: [u8; 4]) -> Vec<u8> {
            let rgba: Vec<u8> = (0..4).flat_map(|_| color).collect(); // 2×2
            let mut out = Vec::new();
            image_webp::WebPEncoder::new(std::io::Cursor::new(&mut out))
                .encode(&rgba, 2, 2, image_webp::ColorType::Rgba8)
                .expect("编码 WebP 应成功");
            out
        }
        let red = webp([255, 0, 0, 255]);
        let green = webp([0, 255, 0, 255]);

        // 先自证 decode_webp 能把它解回来
        let d = decode_webp(&red).expect("应解出 WebP");
        assert_eq!((d.width, d.height), (2, 2));
        assert_eq!(&d.rgba[0..4], &[255, 0, 0, 255], "解码首像素应红");

        let seq = BitmapSequence {
            width: 2,
            height: 2,
            frame_rate: 30.0,
            frames: vec![
                BitmapFrame {
                    is_keyframe: true,
                    bitmaps: vec![BitmapRect {
                        x: 0,
                        y: 0,
                        bytes: &red,
                    }],
                },
                BitmapFrame {
                    is_keyframe: true,
                    bitmaps: vec![BitmapRect {
                        x: 0,
                        y: 0,
                        bytes: &green,
                    }],
                },
            ],
        };
        let h = register_pag_webp(&seq).expect("真 WebP 应重放并注册");
        assert_eq!(frame_count(h), 2);
        assert_ne!(
            frame(h, 0).unwrap().id(),
            frame(h, 1).unwrap().id(),
            "红帧与绿帧应是不同的图"
        );

        // 非 WebP 字节 → 解码失败 → 整段拒绝
        let bad = BitmapSequence {
            width: 2,
            height: 2,
            frame_rate: 30.0,
            frames: vec![BitmapFrame {
                is_keyframe: true,
                bitmaps: vec![BitmapRect {
                    x: 0,
                    y: 0,
                    bytes: b"not a webp",
                }],
            }],
        };
        assert!(register_pag_webp(&bad).is_none(), "非 WebP 应整段拒绝");
    }

    #[test]
    fn webp_decode_never_panics_on_malformed_input() {
        // decode_webp 吃的是 .pag 里的**不可信** WebP 字节。契约:解不了返回 None,
        // **绝不 panic、绝不内存炸弹**。这里喂对抗语料验证(与 sv-pag/sv-vap 同款纪律)。
        fn webp(color: [u8; 4], w: u32, h: u32) -> Vec<u8> {
            let rgba: Vec<u8> = (0..w * h).flat_map(|_| color).collect();
            let mut out = Vec::new();
            image_webp::WebPEncoder::new(std::io::Cursor::new(&mut out))
                .encode(&rgba, w, h, image_webp::ColorType::Rgba8)
                .expect("编码应成功");
            out
        }

        let valid = webp([1, 2, 3, 255], 4, 4);
        let mut corpus: Vec<Vec<u8>> = Vec::new();
        // 1) 合法 WebP 从每个字节切一刀(截断的头/块)
        for cut in 0..valid.len() {
            corpus.push(valid[..cut].to_vec());
        }
        // 2) 对抗:空、乱码、只有 RIFF 头、伪造巨大尺寸的 RIFF、非 WebP 魔数
        corpus.push(Vec::new());
        corpus.push(b"not a webp at all".to_vec());
        corpus.push(b"RIFF".to_vec());
        corpus.push(b"RIFF\xff\xff\xff\xffWEBP".to_vec());
        corpus.push(b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec());
        // 伪造一个"尺寸巨大"的 RIFF/WEBP 头(内存炸弹意图),必须被尺寸护栏或解码器挡住
        corpus.push(b"RIFFxxxxWEBPVP8L\xff\xff\xff\x0f".to_vec());
        // 3) 字节翻转变异(在合法基础上逐字节 XOR 0xFF 造畸形)
        for i in (0..valid.len()).step_by(3) {
            let mut m = valid.clone();
            m[i] ^= 0xFF;
            corpus.push(m);
        }

        for (idx, bytes) in corpus.iter().enumerate() {
            let b = bytes.clone();
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = decode_webp(&b); // 只准 Some/None,不准 panic/OOM
            }));
            assert!(r.is_ok(), "decode_webp 对第 {idx} 条畸形 WebP panic 了");
        }

        // 合法的仍解得出来(护栏没误伤正常尺寸)
        assert!(decode_webp(&valid).is_some(), "4×4 合法 WebP 应能解");
    }

    #[test]
    fn vector_unknown_handle_draws_nothing() {
        use crate::paint::RecordingPainter;
        reset_for_test();
        let mut painter = RecordingPainter::default();
        let drawn = render_vector(999, 0, (0.0, 0.0, 100.0, 100.0), 1.0, &mut painter);
        assert!(!drawn, "未注册的句柄应静默不画");
        assert!(painter.cmds.is_empty());
    }

    #[test]
    fn vector_source_never_yields_a_bitmap() {
        reset_for_test();
        let anim = sv_ui::AnimData {
            source: sv_ui::AnimSource::Vector { handle: 1 },
            intrinsic: (10.0, 10.0),
            frame_rate: 24.0,
            frame_count: 10,
            frame: 0,
            looped: true,
            playing: true,
        };
        assert!(
            image_for(&anim).is_none(),
            "矢量档不产生位图 —— 给它占位图会把'还没接'伪装成'画错了'"
        );
    }

    #[test]
    fn image_for_follows_the_current_frame() {
        reset_for_test();
        let h = register_frames(vec![
            solid(2, 2, [255, 0, 0, 255]),
            solid(2, 2, [0, 255, 0, 255]),
        ]);
        let mut anim = sv_ui::AnimData {
            source: sv_ui::AnimSource::Frames { handle: h },
            intrinsic: (2.0, 2.0),
            frame_rate: 24.0,
            frame_count: 2,
            frame: 0,
            looped: true,
            playing: true,
        };
        let f0 = image_for(&anim).expect("第 0 帧应存在");
        anim.frame = 1;
        let f1 = image_for(&anim).expect("第 1 帧应存在");
        assert_ne!(f0.id(), f1.id(), "换帧必须换到不同的那张图");
    }
}
