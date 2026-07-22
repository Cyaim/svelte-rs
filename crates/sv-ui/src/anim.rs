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

    /// 多个动画并存时各按各的时长推进,先到期的单独出队。
    /// 防的退化:pump 里图省事"有一个完成就整体收尾/整体保留"——
    /// 列表逐项淡入(错峰进场)会立刻穿帮
    #[test]
    fn multiple_animations_advance_independently() {
        let doc = Doc::new();
        let fast = doc.create_text("快");
        doc.append(doc.root(), fast);
        let slow = doc.create_text("慢");
        doc.append(doc.root(), slow);
        transition_in_fade(&doc, fast, 100);
        transition_in_fade(&doc, slow, 400);

        assert!(pump(0.0));
        assert!(pump(100.0), "慢的没完,应继续排帧");
        assert_eq!(opacity_of(&doc, fast), 1.0, "快的应已到终点");
        let mid = opacity_of(&doc, slow);
        assert!(mid > 0.0 && mid < 1.0, "慢的应还在半路:{mid}");
        assert!(!pump(400.0), "都完成后应停排帧");
        assert_eq!(opacity_of(&doc, slow), 1.0);
        assert!(!active());
    }

    /// 同一节点上 opacity 与 ScrollY 是两条独立通道:滚动重定向按
    /// (node, channel) 匹配。防的退化:匹配时漏掉 channel——那么给一个
    /// 正在淡入的容器发滚动,会把它的淡入动画改成"透明度滚到 100"
    #[test]
    fn scroll_and_opacity_channels_coexist_on_same_node() {
        let doc = Doc::new();
        let list = doc.create_view();
        doc.append(doc.root(), list);
        transition_in_fade(&doc, list, 200);
        scroll_y_to(&doc, list, 100.0);
        assert_eq!(scroll_y_target(&doc, list), 100.0, "滚动通道应独立记目标");

        assert!(pump(0.0));
        assert!(pump(100.0));
        let (op, sy) = (opacity_of(&doc, list), doc.scroll_of(list).1);
        assert!(op > 0.0 && op < 1.0, "淡入应在半路:{op}");
        assert!(sy > 0.0 && sy < 100.0, "滚动应在半路:{sy}");
        assert!(!pump(500.0));
        assert_eq!(opacity_of(&doc, list), 1.0);
        assert_eq!(doc.scroll_of(list).1, 100.0);
    }

    /// 连续滚轮:同一容器重复 `scroll_y_to` 只改目标+重置起点,不叠动画。
    /// 叠出两个动画的话它们每帧互相覆写(后写的赢),而 `scroll_y_target`
    /// 读到的是**第一个**的旧目标 —— 快速滚动就会累加错、看起来回弹
    #[test]
    fn scroll_retarget_reuses_single_animation() {
        let doc = Doc::new();
        let list = doc.create_view();
        doc.append(doc.root(), list);
        scroll_y_to(&doc, list, 50.0);
        assert!(pump(0.0));
        assert!(pump(70.0));
        let half = doc.scroll_of(list).1;
        assert!(half > 0.0 && half < 50.0, "应滚到一半:{half}");

        scroll_y_to(&doc, list, 120.0); // 半路改目标
        assert_eq!(scroll_y_target(&doc, list), 120.0, "目标应就地改写");
        assert!(pump(80.0), "改目标后从当前位置重新计时");
        assert!(doc.scroll_of(list).1 >= half, "不该跳回起点");
        assert!(
            !pump(80.0 + SCROLL_MS as f64),
            "只该剩一个动画,一次收尾就结束"
        );
        assert_eq!(doc.scroll_of(list).1, 120.0);
        assert!(!active());
    }

    /// 收尾帧写**精确目标**:缓动在 t=1 数学上等于 to,浮点上却可能差一点,
    /// 留下半像素错位(滚到底时最明显——底部永远差一丝)
    #[test]
    fn scroll_snaps_exactly_to_target_on_final_frame() {
        let doc = Doc::new();
        let list = doc.create_view();
        doc.append(doc.root(), list);
        // 这对数值是挑过的:f32 下 `from + (to - from)` 恰好落不到 to 上
        doc.set_scroll(list, 0.0, 21.91);
        scroll_y_to(&doc, list, 101.766);
        assert!(pump(0.0));
        assert!(!pump(SCROLL_MS as f64 + 1.0));
        assert_eq!(doc.scroll_of(list).1, 101.766, "收尾必须落在精确目标上");
    }

    /// 目标与当前位置差得可以忽略时直接返回:既不开动画也不催帧。
    /// 防的退化:去掉这道阈值——滚到边界后每个滚轮事件都开一个零位移动画,
    /// shell 就永远停不下来重绘(白耗一核)
    #[test]
    fn scroll_to_current_position_is_noop() {
        let doc = Doc::new();
        let list = doc.create_view();
        doc.append(doc.root(), list);
        doc.set_scroll(list, 0.0, 40.0);
        let v0 = doc.version();
        scroll_y_to(&doc, list, 40.2);
        assert!(!active(), "微小差值不该开动画");
        assert_eq!(doc.version(), v0, "也不该催重绘");
        assert_eq!(
            scroll_y_target(&doc, list),
            40.0,
            "没有在途动画时,目标就是当前偏移"
        );
        scroll_y_to(&doc, list, 41.0);
        assert!(active(), "超过阈值才开动画");
    }
}
