//! 极简动画驱动(进场过渡与平滑滚动的载体)。
//!
//! UI 线程集中管理进行中的动画;shell 每帧调 [`pump`] 推进并在仍有动画时
//! 继续请求重绘;无窗测试用合成时间调 pump。
//! 通道:opacity(`transition:fade` / `in:fade`)与**滚动偏移**(S6 平滑滚动)。
//! 出场(out:)需要 INERT 延迟销毁机制,推迟(见 SVELTE-SUPPORT)。

use std::cell::RefCell;

use crate::{Doc, ViewId};

struct Anim {
    doc: Doc,
    node: ViewId,
    channel: Channel,
    from: f32,
    to: f32,
    /// NAN = 尚未起步,第一次 pump 时以当时时间为起点
    start_ms: f64,
    dur_ms: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    Opacity,
    /// 纵向滚动偏移(S6):同一容器再次滚动时**改目标不新开**,
    /// 否则连续滚轮会叠出一堆互相打架的动画
    ScrollY,
}

thread_local! {
    static ANIMS: RefCell<Vec<Anim>> = const { RefCell::new(Vec::new()) };
}

/// 平滑滚动时长(ms)。取值参考浏览器 smooth behavior 的量级:
/// 短到不拖沓、长到能看出方向
const SCROLL_MS: f32 = 140.0;

/// 平滑滚到 `target`(纵向)。同一容器重复调用只改目标与起点,
/// 连续滚轮因此是"追着新目标跑"而不是叠动画
pub fn scroll_y_to(doc: &Doc, node: ViewId, target: f32) {
    let current = doc.scroll_of(node).1;
    if (current - target).abs() < 0.5 {
        return;
    }
    ANIMS.with(|a| {
        let mut anims = a.borrow_mut();
        if let Some(an) = anims
            .iter_mut()
            .find(|an| an.node == node && an.channel == Channel::ScrollY)
        {
            an.from = current;
            an.to = target;
            an.start_ms = f64::NAN;
            return;
        }
        anims.push(Anim {
            doc: doc.clone(),
            node,
            channel: Channel::ScrollY,
            from: current,
            to: target,
            start_ms: f64::NAN,
            dur_ms: SCROLL_MS,
        });
    });
    // 立刻 bump:动画尚未起步,但要先请求一帧把循环带起来
    doc.bump();
}

/// 某容器**正在进行中**的滚动目标(没有则取当前偏移)。
/// 连续滚轮要在目标上累加,而不是在"这一帧画到哪儿"上累加——
/// 后者会让快速滚动越滚越慢
pub fn scroll_y_target(doc: &Doc, node: ViewId) -> f32 {
    ANIMS
        .with(|a| {
            a.borrow()
                .iter()
                .find(|an| an.node == node && an.channel == Channel::ScrollY)
                .map(|an| an.to)
        })
        .unwrap_or_else(|| doc.scroll_of(node).1)
}

/// 进场淡入:创建时透明,`dur` 毫秒内淡到不透明
pub fn transition_in_fade(doc: &Doc, node: ViewId, dur: u32) {
    doc.update_style(node, |s| s.opacity = 0.0);
    ANIMS.with(|a| {
        a.borrow_mut().push(Anim {
            doc: doc.clone(),
            node,
            channel: Channel::Opacity,
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
            match an.channel {
                Channel::Opacity => an.doc.update_style(an.node, |s| s.opacity = v),
                Channel::ScrollY => {
                    let x = an.doc.scroll_of(an.node).0;
                    // 收尾一帧写精确目标:缓动函数在 t=1 才严格等于 to,
                    // 浮点上差一点点会留下半像素错位
                    let v = if t >= 1.0 { an.to } else { v };
                    an.doc.set_scroll(an.node, x, v);
                }
            }
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
