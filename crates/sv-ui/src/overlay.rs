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

    /// Esc/程序性关闭:调最上层可自动关闭弹层的 on_dismiss(LIFO;
    /// 只回写 signal,不直接拆)。返回是否有弹层消费
    pub fn dismiss_topmost_overlay(&self) -> bool {
        let cb = self.read(|inner| {
            inner
                .overlays
                .iter()
                .rev()
                .find(|e| e.close != CloseBehavior::None)
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
