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

use crate::paint::PixelImage;

/// 一份已解码的帧序列。
///
/// **注意它是"逐帧独立的成品帧",不是 PAG 容器里那种差分帧。**
/// PAG 的位图序列是关键帧 + 脏矩形差分,必须从最近关键帧重放才能还原一帧
/// (见 `sv-pag` README 的四步流程)。重放是导入侧的事;
/// 到了这里,每一帧都必须是可以直接贴上去的完整画面。
struct Frames {
    frames: Vec<PixelImage>,
}

thread_local! {
    /// 句柄 → 内容。**thread_local 是刻意的**:场景树本来就是单线程模型
    /// (ADR-1),句柄也就没有跨线程的意义
    static STORE: RefCell<HashMap<u64, Frames>> = RefCell::new(HashMap::new());
    /// 句柄分配器。**从 1 起** —— 0 留给 `AnimData::placeholder()`,
    /// 于是"忘了接素材"与"接了但注册表里没有"是两种可区分的状态
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// 注册一段已解码的帧序列,返回给场景树用的句柄。
///
/// 空序列也接受并返回句柄:一个还没加载完的动画是合法状态,
/// 它只是暂时画不出东西 —— 拒绝它会逼调用方自己发明一个"待定"表示。
pub fn register_frames(frames: Vec<PixelImage>) -> u64 {
    let handle = NEXT.with(|n| {
        let h = n.get();
        n.set(h + 1);
        h
    });
    STORE.with(|s| s.borrow_mut().insert(handle, Frames { frames }));
    handle
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

/// 场景树上的一个动画节点当前该画哪张图。
///
/// 矢量档([`sv_ui::AnimSource::Vector`])在这里恒返回 `None` ——
/// 它要走 `sv-lottie` 的 `RenderSink` 直接发路径命令,根本不产生位图,
/// 接线不在这一层。**给它一个占位位图是错的**:那会让"矢量还没接"
/// 看起来像"接了但画错了"。
pub(crate) fn image_for(anim: &sv_ui::AnimData) -> Option<PixelImage> {
    match anim.source {
        sv_ui::AnimSource::Frames { handle } => frame(handle, anim.frame),
        sv_ui::AnimSource::Vector { .. } => None,
    }
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    STORE.with(|s| s.borrow_mut().clear());
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
