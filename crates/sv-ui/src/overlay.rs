//! 弹层体系(调研 25):**离散层 + 注册序,不做通用 z-index**(egui Order 同构)。
//!
//! - overlay 根是**游离子树**(不挂 doc.root——即传送门语义);渲染壳在基础层
//!   布局后把 overlay 的 Placed 追加在末尾,树序绘制 + `rev()` 命中零改动即得
//!   正确遮挡与优先命中;
//! - 层内叠序 = 注册序(打开序);[`OverlayLayer::Tooltip`] 恒最后画且不可命中;
//! - **on_dismiss 不直接关弹层,只回写 signal** → `open` 翻假走同一条拆除路
//!   ——单一数据源,与 bind:checked 双向绑定同型;
//! - Esc 关闭挂进 [`crate::dispatch_key`] 导航段(LIFO,嵌套弹层逐层关);
//!   click-outside 判定在渲染壳(需要 Placed 几何)。

use std::cell::RefCell;
use std::rc::Rc;

use sv_reactive::{RootHandle, create_root, derived, effect, on_cleanup, untrack};

use crate::{Doc, ViewId};

/// 弹层归属的离散层(Base 即普通场景树)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OverlayLayer {
    /// 弹出层:菜单/下拉/对话框(可交互,注册序叠放)
    #[default]
    Popup,
    /// 提示层:恒画在最上且**不可命中**(egui Tooltip 同款)
    Tooltip,
}

/// 锚定方式(锚点解析在渲染壳布局尾段:越界翻转 + 窗口 clamp)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Anchor {
    /// 锚到某节点的一侧(菜单/下拉/tooltip)
    Node { id: ViewId, side: Side, gap: f32 },
    /// 逻辑坐标绝对点(右键菜单)
    Point(f32, f32),
    /// 窗口居中(对话框)
    WindowCenter,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Below,
    Above,
    Left,
    Right,
}

/// 关闭策略(对齐 Slint close-policy 三值;Esc 对前两者恒生效)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CloseBehavior {
    /// 点弹层外关闭,且该次点击被吞(菜单/下拉惯例)
    #[default]
    OnClickOutside,
    /// 任意点击都关闭(点选项也关),点击继续下传
    OnAnyClick,
    /// 只能程序关(modal 对话框)
    None,
}

/// 一个已注册的弹层(存于 `DocumentInner.overlays`,注册序即层内叠序)
#[derive(Clone)]
pub struct OverlayEntry {
    /// 游离子树根(不在 doc.root 下)
    pub root: ViewId,
    pub anchor: Anchor,
    pub layer: OverlayLayer,
    /// 真:命中测试跳过本弹层之下的一切(渲染壳区间阻断)
    pub modal: bool,
    pub close: CloseBehavior,
    /// 关闭手势回调(**只应回写 open signal**,拆除走 overlay_block 统一路)
    pub on_dismiss: Option<Rc<dyn Fn()>>,
}

/// overlay_block 的选项面
#[derive(Clone, Default)]
pub struct OverlayOpts {
    pub layer: OverlayLayer,
    pub modal: bool,
    pub close: CloseBehavior,
    pub on_dismiss: Option<Rc<dyn Fn()>>,
}

impl Doc {
    /// 建游离弹层根(不挂任何父;渲染壳按 overlays 注册表布局)
    pub fn create_overlay_root(&self) -> ViewId {
        self.create_view()
    }

    pub(crate) fn add_overlay(&self, entry: OverlayEntry) {
        self.with_inner_mut(|inner| inner.overlays.push(entry));
        self.bump();
    }

    pub(crate) fn remove_overlay(&self, root: ViewId) {
        self.with_inner_mut(|inner| inner.overlays.retain(|e| e.root != root));
        self.remove(root);
    }

    fn update_overlay_anchor(&self, root: ViewId, anchor: Anchor) {
        let changed = self.with_inner_mut(|inner| {
            let Some(e) = inner.overlays.iter_mut().find(|e| e.root == root) else {
                return false;
            };
            if e.anchor == anchor {
                return false;
            }
            e.anchor = anchor;
            true
        });
        if changed {
            self.bump();
        }
    }

    /// 节点所在弹层的层(沿父链上溯到弹层根;不在弹层返回 None)。
    /// 菜单方向键导航(调研 25 O4)据此把 ArrowDown/Up 映射为焦点移动
    pub fn overlay_layer_of(&self, id: ViewId) -> Option<OverlayLayer> {
        self.read(|inner| {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if let Some(e) = inner.overlays.iter().find(|e| e.root == c) {
                    return Some(e.layer);
                }
                cur = inner.nodes.get(c).and_then(|n| n.parent);
            }
            None
        })
    }

    /// 关闭指定弹层(渲染壳 click-outside 手势用;只回写 signal)
    pub fn dismiss_overlay(&self, root: ViewId) -> bool {
        let cb = self.read(|inner| {
            inner
                .overlays
                .iter()
                .find(|e| e.root == root)
                .and_then(|e| e.on_dismiss.clone())
        });
        match cb {
            Some(cb) => {
                cb();
                true
            }
            None => false,
        }
    }

    /// Esc/程序性关闭:调最上层**带 on_dismiss** 弹层的回调(LIFO;
    /// 只回写 signal,不直接拆)。注意语义分工:[`CloseBehavior`] 只管
    /// 指针手势(点外/任意点),Esc 看是否提供了 on_dismiss——模态对话框
    /// 惯例是"点外不关、Esc 可关"。返回是否有弹层消费
    pub fn dismiss_topmost_overlay(&self) -> bool {
        let cb = self.read(|inner| {
            inner
                .overlays
                .iter()
                .rev()
                .find_map(|e| e.on_dismiss.clone())
        });
        match cb {
            Some(cb) => {
                cb();
                true
            }
            None => false,
        }
    }
}

/// `<overlay>` 的编译目标(双前端共用;与 if_block 同构的建/拆语义):
/// `open` 翻真 → 独立 root 作用域建子树 + 注册 entry;翻假/外层卸载 → 拆除。
/// modal 打开时把焦点移入弹层内第一个可焦点(焦点陷阱由 focusables 的
/// modal 限定完成),关闭时恢复原焦点。
pub fn overlay_block(
    doc: &Doc,
    open: impl Fn() -> bool + 'static,
    anchor: impl Fn() -> Anchor + 'static,
    opts: OverlayOpts,
    build: impl Fn(&Doc, ViewId) + 'static,
) {
    type Slot = Rc<RefCell<Option<(ViewId, RootHandle, Option<ViewId>)>>>;
    let open_d = derived(open);
    let doc = doc.clone();
    let slot: Slot = Rc::new(RefCell::new(None));
    let slot_cleanup = slot.clone();
    let doc_cleanup = doc.clone();

    effect(move || {
        let is_open = open_d.get();
        let a = anchor(); // anchor 依赖也被追踪(锚点切换即时生效)
        let already = slot.borrow().as_ref().map(|(root, ..)| *root);
        match (is_open, already) {
            (true, Some(root)) => doc.update_overlay_anchor(root, a),
            (true, None) => {
                let root = doc.create_overlay_root();
                let d = doc.clone();
                let b = &build;
                let (_, scope) = create_root(|| untrack(|| b(&d, root)));
                let prev_focus = doc.focused();
                doc.add_overlay(OverlayEntry {
                    root,
                    anchor: a,
                    layer: opts.layer,
                    modal: opts.modal,
                    close: opts.close,
                    on_dismiss: opts.on_dismiss.clone(),
                });
                if opts.modal {
                    // 焦点陷阱入口:焦点移入弹层(focusables 已按 modal 限定)
                    doc.blur();
                    doc.focus_next();
                }
                *slot.borrow_mut() = Some((root, scope, prev_focus));
            }
            (false, Some(_)) => {
                if let Some((root, scope, prev)) = slot.borrow_mut().take() {
                    scope.dispose();
                    doc.remove_overlay(root);
                    // 恢复原焦点(节点可能已消亡:focus 内部世代键自然落空)
                    if let Some(p) = prev {
                        doc.focus(p);
                    }
                }
            }
            (false, None) => {}
        }
    });

    on_cleanup(move || {
        if let Some((root, scope, _)) = slot_cleanup.borrow_mut().take() {
            scope.dispose();
            doc_cleanup.remove_overlay(root);
        }
    });
}

/// tooltip 原语(调研 25 O5):悬停 `delay_ms` 后显示,离开即隐;
/// "悬停代数计数"防延时期间进出错位(egui tooltip_delay 同构,
/// grace_time 列 P2)。挂 Tooltip 层:恒最上、不可命中
pub fn tooltip(doc: &Doc, target: ViewId, delay_ms: u64, build: impl Fn(&Doc, ViewId) + 'static) {
    use std::cell::Cell;
    let open = sv_reactive::state(false);
    let generation: Rc<Cell<u64>> = Rc::new(Cell::new(0));

    let g = generation.clone();
    doc.set_on_pointer_enter(target, move || {
        g.set(g.get() + 1);
        let my_gen = g.get();
        let g2 = g.clone();
        crate::tasks::spawn(
            async move {
                // 后台线程睡延时(tasks 桥的 waker 会踢一帧)
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            },
            move |_| {
                // 代数没变 = 悬停未中断,才真正打开
                if g2.get() == my_gen {
                    open.set(true);
                }
            },
        );
    });
    let g = generation.clone();
    doc.set_on_pointer_leave(target, move || {
        g.set(g.get() + 1);
        open.set(false);
    });

    overlay_block(
        doc,
        move || open.get(),
        move || Anchor::Node {
            id: target,
            side: Side::Below,
            gap: 6.0,
        },
        OverlayOpts {
            layer: OverlayLayer::Tooltip,
            modal: false,
            close: CloseBehavior::None,
            on_dismiss: None,
        },
        build,
    );
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashSet;

    use sv_reactive::{Signal, create_root, state};

    use super::*;

    /// 注册表快照:overlays 的顺序**就是**叠序(渲染壳按它追加 Placed)
    fn overlay_roots(doc: &Doc) -> Vec<ViewId> {
        doc.read(|inner| inner.overlays.iter().map(|e| e.root).collect())
    }

    /// 建一个只含一行文本的弹层,返回它的 open 信号
    fn text_overlay(doc: &Doc, label: &'static str, opts: OverlayOpts) -> Signal<bool> {
        let open = state(false);
        overlay_block(
            doc,
            move || open.get(),
            || Anchor::Point(0.0, 0.0),
            opts,
            move |d, root| {
                let t = d.create_text(label);
                d.append(root, t);
            },
        );
        open
    }

    /// 叠序=注册序(打开序),重开会重新排到最上层——菜单再次打开必须盖住
    /// 先开的东西。同时钉住弹层根是**游离子树**(不挂 doc.root,传送门语义),
    /// 以及 dump 的弹层段序与注册序一致
    #[test]
    fn registration_order_is_stacking_order() {
        let doc = Doc::new();
        let (_, scope) = create_root(|| {
            let a = text_overlay(&doc, "甲", OverlayOpts::default());
            let b = text_overlay(&doc, "乙", OverlayOpts::default());
            let c = text_overlay(&doc, "丙", OverlayOpts::default());
            a.set(true);
            b.set(true);
            c.set(true);
            let roots = overlay_roots(&doc);
            assert_eq!(roots.len(), 3);
            for r in &roots {
                assert!(
                    doc.read(|inner| inner.nodes[*r].parent.is_none()),
                    "弹层根应是游离子树"
                );
            }
            // 关中间再开:重新入队到末尾 = 变成最上层
            b.set(false);
            assert_eq!(overlay_roots(&doc).len(), 2);
            b.set(true);
            let after = overlay_roots(&doc);
            assert_eq!(after[0], roots[0]);
            assert_eq!(after[1], roots[2], "丙 应升为第二层");
            assert_ne!(after[2], roots[2], "重开的乙 应排到最上层");

            let dump = doc.dump();
            let at = |s: &str| dump.find(s).unwrap_or_else(|| panic!("dump 缺 {s}:{dump}"));
            assert!(at("== overlay") < at("甲"), "弹层段应在基础层之后");
            assert!(
                at("甲") < at("丙") && at("丙") < at("乙"),
                "dump 段序=注册序"
            );
        });
        scope.dispose();
    }

    /// Esc 的判据是"有没有 on_dismiss",不是 [`CloseBehavior`]:后者只管指针手势。
    /// 防的退化:把两件事合并成一个开关——那么 `close: None` 的模态对话框会
    /// 变成 Esc 也关不掉,或者不可交互的 tooltip 反倒把 Esc 吞了
    #[test]
    fn esc_lifo_follows_on_dismiss_not_close_behavior() {
        let doc = Doc::new();
        let (_, scope) = create_root(|| {
            let menu = state(true);
            overlay_block(
                &doc,
                move || menu.get(),
                || Anchor::Point(0.0, 0.0),
                OverlayOpts {
                    close: CloseBehavior::OnClickOutside,
                    on_dismiss: Some(Rc::new(move || menu.set(false))),
                    ..Default::default()
                },
                |d, root| {
                    let t = d.create_text("菜单");
                    d.append(root, t);
                },
            );
            let dialog = state(true);
            overlay_block(
                &doc,
                move || dialog.get(),
                || Anchor::WindowCenter,
                OverlayOpts {
                    modal: true,
                    close: CloseBehavior::None, // 点外不关,但 Esc 该关
                    on_dismiss: Some(Rc::new(move || dialog.set(false))),
                    ..Default::default()
                },
                |d, root| {
                    let t = d.create_text("对话框");
                    d.append(root, t);
                },
            );
            let tip = state(true);
            overlay_block(
                &doc,
                move || tip.get(),
                || Anchor::Point(1.0, 1.0),
                OverlayOpts {
                    layer: OverlayLayer::Tooltip,
                    close: CloseBehavior::OnAnyClick,
                    on_dismiss: None, // 没有关闭手势的弹层
                    ..Default::default()
                },
                |d, root| {
                    let t = d.create_text("提示");
                    d.append(root, t);
                },
            );
            assert_eq!(overlay_roots(&doc).len(), 3);

            assert!(doc.dismiss_topmost_overlay());
            assert!(
                !dialog.get_untracked(),
                "Esc 应越过没有 on_dismiss 的 tooltip,关掉 close=None 的模态"
            );
            assert!(tip.get_untracked(), "tooltip 不该被 Esc 关掉");
            assert!(doc.dismiss_topmost_overlay());
            assert!(!menu.get_untracked(), "Esc 逐层往下关");
            assert!(
                !doc.dismiss_topmost_overlay(),
                "只剩没有 on_dismiss 的弹层时不该假装消费掉 Esc(否则 Esc 被黑洞吞掉)"
            );
        });
        scope.dispose();
    }

    /// on_dismiss 是**单一数据源**的关键:它只回写 signal,拆除一律走
    /// `open` 翻假那条路。防的退化:让 dismiss 顺手把弹层拆了——那样 signal
    /// 还是 true,状态与画面就此分叉(再次 set(true) 也打不开)
    #[test]
    fn dismiss_only_writes_signal_teardown_follows_open() {
        let doc = Doc::new();
        let (_, scope) = create_root(|| {
            let open = state(true);
            let hits = Rc::new(Cell::new(0));
            let h = hits.clone();
            overlay_block(
                &doc,
                move || open.get(),
                || Anchor::WindowCenter,
                OverlayOpts {
                    on_dismiss: Some(Rc::new(move || h.set(h.get() + 1))),
                    ..Default::default()
                },
                |d, root| {
                    let t = d.create_text("内容");
                    d.append(root, t);
                },
            );
            let root0 = overlay_roots(&doc)[0];

            assert!(doc.dismiss_overlay(root0));
            assert_eq!(hits.get(), 1);
            assert_eq!(
                overlay_roots(&doc),
                vec![root0],
                "on_dismiss 不该自己拆弹层"
            );
            // 真正翻假才拆,且游离子树被回收
            open.set(false);
            assert!(overlay_roots(&doc).is_empty());
            assert!(
                doc.read(|inner| inner.nodes.get(root0).is_none()),
                "拆除应回收游离子树,否则节点表只涨不落"
            );
            // 已不在注册表:两个 dismiss 入口都应如实报告"没人消费"
            assert!(!doc.dismiss_overlay(root0));
            assert!(!doc.dismiss_topmost_overlay());
            assert_eq!(hits.get(), 1);
        });
        scope.dispose();
    }

    /// 子菜单场景:内层 overlay_block 建在外层的 build 里。外层关闭时内层的
    /// entry 必须跟着消失,否则注册表里留下指向已销毁子树的幽灵条目
    /// (渲染壳照着它布局 = 悬空菜单)
    #[test]
    fn nested_overlay_unregisters_with_outer() {
        let doc = Doc::new();
        let (_, scope) = create_root(|| {
            let outer = state(false);
            let inner = state(true); // 外层一建,内层跟着建
            overlay_block(
                &doc,
                move || outer.get(),
                || Anchor::Point(0.0, 0.0),
                OverlayOpts::default(),
                move |d, root| {
                    let t = d.create_text("外层");
                    d.append(root, t);
                    overlay_block(
                        d,
                        move || inner.get(),
                        || Anchor::Point(5.0, 5.0),
                        OverlayOpts::default(),
                        |d2, r2| {
                            let t = d2.create_text("内层");
                            d2.append(r2, t);
                        },
                    );
                },
            );
            assert!(overlay_roots(&doc).is_empty());
            outer.set(true);
            assert_eq!(overlay_roots(&doc).len(), 2, "内层应随外层一起注册");
            assert!(doc.dump().contains("内层"));
            outer.set(false);
            assert!(
                overlay_roots(&doc).is_empty(),
                "外层拆除应连带拆掉内层 entry"
            );
            assert!(!doc.dump().contains("内层"));
        });
        scope.dispose();
    }

    /// 组件卸载(外层作用域销毁)必须带走弹层:注册表与节点表都不能留残骸。
    /// 防的退化:只在 `open` 翻假时拆,忘了 on_cleanup 那条路
    #[test]
    fn owner_scope_dispose_tears_down_overlay() {
        let doc = Doc::new();
        let open = state(true); // 建在根作用域外,销毁后仍可读
        let (_, scope) = create_root(|| {
            overlay_block(
                &doc,
                move || open.get(),
                || Anchor::WindowCenter,
                OverlayOpts {
                    modal: true,
                    ..Default::default()
                },
                |d, root| {
                    let t = d.create_text("对话框");
                    d.append(root, t);
                },
            );
        });
        let root0 = overlay_roots(&doc)[0];
        assert!(doc.dump().contains("对话框"));
        scope.dispose();
        assert!(overlay_roots(&doc).is_empty(), "组件卸载应带走弹层注册");
        assert!(doc.read(|inner| inner.nodes.get(root0).is_none()));
        assert!(!doc.dump().contains("对话框"));
    }

    /// 锚点变化走**原地更新**:不重建子树(重建 = 弹层里的输入/选中态被清空),
    /// 变了要 bump 版本让壳重新布局,没变则不 bump(防止每帧无谓重绘)
    #[test]
    fn anchor_change_updates_entry_in_place() {
        let doc = Doc::new();
        let builds = Rc::new(Cell::new(0));
        let b = builds.clone();
        let (_, scope) = create_root(|| {
            let open = state(true);
            let x = state(10.0f32);
            overlay_block(
                &doc,
                move || open.get(),
                move || Anchor::Point(x.get(), 0.0),
                OverlayOpts::default(),
                move |d, root| {
                    b.set(b.get() + 1);
                    let t = d.create_text("内容");
                    d.append(root, t);
                },
            );
            let root0 = overlay_roots(&doc)[0];
            assert_eq!(builds.get(), 1);

            let v0 = doc.version();
            x.set(200.0);
            assert_eq!(overlay_roots(&doc), vec![root0], "锚点变化不该重建弹层");
            assert_eq!(builds.get(), 1, "build 不该重跑");
            assert_eq!(
                doc.read(|inner| inner.overlays[0].anchor),
                Anchor::Point(200.0, 0.0)
            );
            assert!(doc.version() > v0, "锚点变了要 bump,否则壳不会重新布局");

            let v1 = doc.version();
            x.set(200.0); // 同值
            assert_eq!(doc.version(), v1, "锚点没变不该催重绘");
        });
        scope.dispose();
    }

    /// build 里读 signal 是家常便饭(初值、条件分支)。它跑在 `untrack` 下,
    /// 这些读取不能记到驱动 effect 头上——否则那个 signal 一变,驱动 effect
    /// 重跑,弹层的子作用域被连带销毁(弹层还在,里面的绑定已经死了)
    #[test]
    fn build_reads_are_untracked() {
        let doc = Doc::new();
        let deaths = Rc::new(Cell::new(0));
        let d = deaths.clone();
        let label = state("初始");
        let (_, scope) = create_root(|| {
            let open = state(true);
            overlay_block(
                &doc,
                move || open.get(),
                || Anchor::WindowCenter,
                OverlayOpts::default(),
                move |doc, root| {
                    let t = doc.create_text(label.get());
                    doc.append(root, t);
                    let d = d.clone();
                    on_cleanup(move || d.set(d.get() + 1));
                },
            );
        });
        assert_eq!(deaths.get(), 0);
        label.set("改了");
        assert_eq!(
            deaths.get(),
            0,
            "build 内的读取不该成为驱动 effect 的依赖(弹层子作用域被误销毁)"
        );
        assert_eq!(overlay_roots(&doc).len(), 1);
        scope.dispose();
    }

    /// 菜单方向键导航(O4)靠 `overlay_layer_of` 判断"焦点在不在弹层里":
    /// 上溯要能穿过任意深度,基础层节点必须如实返回 None(否则基础层的
    /// ArrowDown 会被当成菜单导航吃掉)
    #[test]
    fn overlay_layer_of_walks_up_to_owning_overlay() {
        let doc = Doc::new();
        let base_btn = doc.create_button("底");
        doc.append(doc.root(), base_btn);
        let deep = Rc::new(Cell::new(None));
        let tip_node = Rc::new(Cell::new(None));
        let (dp, tp) = (deep.clone(), tip_node.clone());
        let open = state(true);
        let (_, scope) = create_root(|| {
            overlay_block(
                &doc,
                move || open.get(),
                || Anchor::WindowCenter,
                OverlayOpts::default(),
                move |d, root| {
                    let mid = d.create_view();
                    d.append(root, mid);
                    let btn = d.create_button("深");
                    d.append(mid, btn);
                    dp.set(Some(btn));
                },
            );
            overlay_block(
                &doc,
                move || open.get(),
                || Anchor::Point(0.0, 0.0),
                OverlayOpts {
                    layer: OverlayLayer::Tooltip,
                    ..Default::default()
                },
                move |d, root| {
                    let t = d.create_text("提示");
                    d.append(root, t);
                    tp.set(Some(t));
                },
            );
        });
        assert_eq!(
            doc.overlay_layer_of(deep.get().unwrap()),
            Some(OverlayLayer::Popup),
            "隔了一层容器也要能上溯到弹层根"
        );
        assert_eq!(
            doc.overlay_layer_of(tip_node.get().unwrap()),
            Some(OverlayLayer::Tooltip)
        );
        assert_eq!(doc.overlay_layer_of(base_btn), None, "基础层不是弹层");
        assert_eq!(doc.overlay_layer_of(doc.root()), None);
        scope.dispose();
    }

    /// 嵌套模态:焦点陷阱跟着**最上层** modal 走,逐层关闭时焦点逐层回退
    /// (对话框里再弹确认框是常态)。防的退化:陷阱认第一个/任意一个 modal,
    /// 或关闭时把焦点直接丢回基础层
    #[test]
    fn nested_modal_traps_focus_in_topmost_and_restores_layer_by_layer() {
        let doc = Doc::new();
        let base = doc.create_button("底层");
        doc.append(doc.root(), base);
        let a_btns: Rc<RefCell<Vec<ViewId>>> = Rc::default();
        let b_btns: Rc<RefCell<Vec<ViewId>>> = Rc::default();
        let (aa, bb) = (a_btns.clone(), b_btns.clone());
        let open_a = state(false);
        let open_b = state(false);
        let modal = || OverlayOpts {
            modal: true,
            close: CloseBehavior::None,
            ..Default::default()
        };
        let (_, scope) = create_root(|| {
            overlay_block(
                &doc,
                move || open_a.get(),
                || Anchor::WindowCenter,
                modal(),
                move |d, root| {
                    for label in ["A1", "A2"] {
                        let b = d.create_button(label);
                        d.append(root, b);
                        aa.borrow_mut().push(b);
                    }
                },
            );
            overlay_block(
                &doc,
                move || open_b.get(),
                || Anchor::WindowCenter,
                modal(),
                move |d, root| {
                    for label in ["B1", "B2"] {
                        let b = d.create_button(label);
                        d.append(root, b);
                        bb.borrow_mut().push(b);
                    }
                },
            );
        });
        doc.focus(base);
        open_a.set(true);
        assert_eq!(
            doc.focused(),
            Some(a_btns.borrow()[0]),
            "打开 modal 应把焦点移进弹层"
        );
        open_b.set(true);
        assert_eq!(doc.focused(), Some(b_btns.borrow()[0]), "陷阱应跟到最上层");

        let mut seen = HashSet::new();
        for _ in 0..4 {
            seen.insert(doc.focused().unwrap());
            doc.focus_next();
        }
        assert_eq!(
            seen,
            b_btns.borrow().iter().copied().collect::<HashSet<_>>(),
            "Tab 环应只在最上层 modal 内循环"
        );

        open_b.set(false);
        assert_eq!(
            doc.focused(),
            Some(a_btns.borrow()[0]),
            "关掉上层 modal 应回到下层 modal 的焦点"
        );
        open_a.set(false);
        assert_eq!(doc.focused(), Some(base), "全关后回到底层原焦点");
        scope.dispose();
    }
}
