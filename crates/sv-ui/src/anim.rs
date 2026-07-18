//! 极简动画驱动(进场过渡的载体)。
//!
//! UI 线程集中管理进行中的动画;shell 每帧调 [`pump`] 推进并在仍有动画时
//! 继续请求重绘;无窗测试用合成时间调 pump。
//! v0 只有 opacity 通道(`transition:fade` / `in:fade`);出场(out:)需要
//! INERT 延迟销毁机制,推迟(见 SVELTE-SUPPORT)。

use std::cell::RefCell;

use crate::{Doc, ViewId};

struct Anim {
    doc: Doc,
    node: ViewId,
    from: f32,
    to: f32,
    /// NAN = 尚未起步,第一次 pump 时以当时时间为起点
    start_ms: f64,
    dur_ms: f32,
}

thread_local! {
    static ANIMS: RefCell<Vec<Anim>> = const { RefCell::new(Vec::new()) };
}

/// 进场淡入:创建时透明,`dur` 毫秒内淡到不透明
pub fn transition_in_fade(doc: &Doc, node: ViewId, dur: u32) {
    doc.update_style(node, |s| s.opacity = 0.0);
    ANIMS.with(|a| {
        a.borrow_mut().push(Anim {
            doc: doc.clone(),
            node,
            from: 0.0,
            to: 1.0,
            start_ms: f64::NAN,
            dur_ms: dur.max(1) as f32,
        })
    });
    // update_style 已 bump 版本 → on_mutate → shell 请求重绘,动画由帧循环接力
}

/// 是否有进行中的动画(shell 决定是否继续排帧)
pub fn active() -> bool {
    ANIMS.with(|a| !a.borrow().is_empty())
}

/// 推进所有动画到 `now_ms`(单调毫秒,起点任意)。返回是否仍有进行中
pub fn pump(now_ms: f64) -> bool {
    ANIMS.with(|a| {
        let mut anims = a.borrow_mut();
        anims.retain_mut(|an| {
            // 节点已被销毁:动画随之丢弃
            if an.doc.read(|inner| inner.nodes.get(an.node).is_none()) {
                return false;
            }
            if an.start_ms.is_nan() {
                an.start_ms = now_ms;
            }
            let t = (((now_ms - an.start_ms) as f32) / an.dur_ms).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t); // ease-out quad
            let v = an.from + (an.to - an.from) * eased;
            an.doc.update_style(an.node, |s| s.opacity = v);
            t < 1.0
        });
        !anims.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opacity_of(doc: &Doc, id: ViewId) -> f32 {
        doc.read(|inner| inner.nodes[id].style.opacity)
    }

    #[test]
    fn fade_in_progresses_and_finishes() {
        let doc = Doc::new();
        let t = doc.create_text("你好");
        doc.append(doc.root(), t);
        transition_in_fade(&doc, t, 100);
        assert_eq!(opacity_of(&doc, t), 0.0, "起点应全透明");

        assert!(pump(1000.0), "刚起步应仍在动画中");
        let mid_early = opacity_of(&doc, t);
        assert!(pump(1050.0));
        let mid = opacity_of(&doc, t);
        assert!(
            mid > mid_early && mid < 1.0,
            "中途应介于两端:{mid_early} → {mid}"
        );
        assert!(!pump(1200.0), "超时应完成并出队");
        assert_eq!(opacity_of(&doc, t), 1.0);
        assert!(!active());
    }

    #[test]
    fn removed_node_drops_animation() {
        let doc = Doc::new();
        let t = doc.create_text("x");
        doc.append(doc.root(), t);
        transition_in_fade(&doc, t, 1000);
        assert!(pump(0.0));
        doc.remove(t);
        assert!(!pump(10.0), "节点销毁后动画应被丢弃");
    }
}
