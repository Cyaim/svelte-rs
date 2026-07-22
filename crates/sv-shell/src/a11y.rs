//! AccessKit 语义树映射(调研 24 §4.1):场景树 → TreeUpdate 纯函数。
//!
//! 与 RecordingPainter 同哲学:**零窗口零平台即可金样测试**。
//! - NodeId = ViewId 的 slotmap ffi 值(含世代号,删后不复用,天然满足
//!   AccessKit 的 id 稳定性要求);
//! - bounds 取 `Placed.rect`(与命中测试同源,天然一致);逻辑 px 上报,
//!   乘 scale 出物理——adapter 侧期望窗口坐标(平台实测校准项,调研 24 风险 5);
//! - 父子结构回 `DocumentInner` 读(Placed 是平铺画序);
//! - `TreeUpdate.focus` 每次必填(无焦点填 root)——焦点链(R1)在此兑现;
//! - **增量推送(P6)**:映射照旧全量算(纯函数、便宜),但只把**内容真变了**
//!   的节点交给平台。省的是屏幕阅读器侧的重处理与 IPC —— 一次键入本该只动
//!   一个节点,全量推会让 AT 重扫整棵树。[`build_tree_update`] 保留为全量
//!   形态(懒激活首帧与测试金样用)。

use std::collections::HashMap;

use accesskit::{Action, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};
use sv_ui::{Doc, ElementKind, Overflow, OverlayLayer, view_id_ffi};

use crate::render::{Placed, Rect};

fn node_id(id: sv_ui::ViewId) -> NodeId {
    NodeId(view_id_ffi(id))
}

/// 全量语义树(逻辑坐标 × scale = 物理 px bounds)
pub fn build_tree_update(doc: &Doc, placed: &[Placed], scale: f32) -> TreeUpdate {
    let (nodes, root, focus) = collect(doc, placed, scale);
    TreeUpdate {
        nodes,
        tree: Some(tree_of(root)),
        tree_id: TreeId::ROOT,
        focus,
    }
}

fn tree_of(root: NodeId) -> Tree {
    let mut tree = Tree::new(root);
    tree.toolkit_name = Some("sv".into());
    tree
}

/// 增量推送的记忆:上一次交给平台的节点内容
#[derive(Default)]
pub struct A11yCache {
    sent: HashMap<NodeId, Node>,
    tree_sent: bool,
}

/// 增量 `TreeUpdate`:只带**内容真变了**的节点(新增/改动)。
/// `focus` 协议要求每次必填,故恒带;`tree` 只在首次推送时带。
/// 删除的节点不必显式上报——父节点的 children 变了会被一并带上,
/// AccessKit 按可达性回收
pub fn incremental_tree_update(
    cache: &mut A11yCache,
    doc: &Doc,
    placed: &[Placed],
    scale: f32,
) -> TreeUpdate {
    let (all, root, focus) = collect(doc, placed, scale);
    let mut changed = Vec::new();
    let mut next = HashMap::with_capacity(all.len());
    for (id, node) in all {
        if cache.sent.get(&id) != Some(&node) {
            changed.push((id, node.clone()));
        }
        next.insert(id, node);
    }
    cache.sent = next;
    let tree = (!cache.tree_sent).then(|| {
        cache.tree_sent = true;
        tree_of(root)
    });
    TreeUpdate {
        nodes: changed,
        tree,
        tree_id: TreeId::ROOT,
        focus,
    }
}

/// 场景树 → (节点表, 根 id, 焦点 id)。两个入口共用的纯映射
fn collect(doc: &Doc, placed: &[Placed], scale: f32) -> (Vec<(NodeId, Node)>, NodeId, NodeId) {
    let rects: HashMap<sv_ui::ViewId, Rect> = placed.iter().map(|p| (p.id, p.rect)).collect();
    doc.read(|inner| {
        let mut nodes: Vec<(NodeId, Node)> = Vec::new();
        fn walk(
            inner: &sv_ui::DocumentInner,
            id: sv_ui::ViewId,
            rects: &HashMap<sv_ui::ViewId, Rect>,
            scale: f32,
            out: &mut Vec<(NodeId, Node)>,
        ) {
            let Some(n) = inner.nodes.get(id) else {
                return;
            };
            // 角色:**只用树里确实存在的信息**,不猜。
            // 容器的语义有两个真实来源 —— 它是不是弹层根(层 + modal),
            // 以及它可不可滚;两者都不满足才退回 GenericContainer
            let scrollable =
                n.style.overflow == Overflow::Scroll || n.style.overflow_x == Overflow::Scroll;
            let role = match n.kind {
                ElementKind::View => match overlay_role(inner, id) {
                    Some(r) => r,
                    None if scrollable => Role::ScrollView,
                    None => Role::GenericContainer,
                },
                ElementKind::Text => Role::Label,
                ElementKind::Button => Role::Button,
                ElementKind::Checkbox => Role::CheckBox,
                ElementKind::TextInput => {
                    if n.input.as_deref().is_some_and(|i| i.multiline) {
                        Role::MultilineTextInput
                    } else {
                        Role::TextInput
                    }
                }
            };
            let mut node = Node::new(role);
            // 名称:aria-label 覆盖优先,否则取节点文本
            let label = n
                .accessible_label
                .clone()
                .or_else(|| (!n.text.is_empty()).then(|| n.text.clone()));
            match n.kind {
                // 输入框:文本是"值",占位符当名称兜底
                ElementKind::TextInput => {
                    node.set_value(n.text.clone());
                    if let Some(l) = n.accessible_label.clone().or_else(|| {
                        n.input.as_deref().and_then(|i| {
                            (!i.placeholder.is_empty()).then(|| i.placeholder.clone())
                        })
                    }) {
                        node.set_label(l);
                    }
                }
                _ => {
                    if let Some(l) = label {
                        node.set_label(l);
                    }
                }
            }
            if n.kind == ElementKind::Checkbox {
                node.set_toggled(if n.checked {
                    Toggled::True
                } else {
                    Toggled::False
                });
            }
            if n.on_click.is_some() || matches!(n.kind, ElementKind::Button | ElementKind::Checkbox)
            {
                node.add_action(Action::Click);
            }
            if n.focusable {
                node.add_action(Action::Focus);
            }
            // 可滚容器:报当前偏移与范围,并接受屏幕阅读器的滚动请求
            // (AT 把焦点移到视口外的元素时会主动要求滚动)
            if scrollable {
                node.set_scroll_y(f64::from(n.scroll_y));
                node.set_scroll_y_min(0.0);
                node.set_scroll_x(f64::from(n.scroll_x));
                node.set_scroll_x_min(0.0);
                node.add_action(Action::ScrollUp);
                node.add_action(Action::ScrollDown);
                node.add_action(Action::SetScrollOffset);
            }
            if n.style.overflow != Overflow::Visible || n.style.overflow_x != Overflow::Visible {
                node.set_clips_children();
            }
            if let Some(r) = rects.get(&id) {
                node.set_bounds(accesskit::Rect {
                    x0: (r.x * scale) as f64,
                    y0: (r.y * scale) as f64,
                    x1: ((r.x + r.w) * scale) as f64,
                    y1: ((r.y + r.h) * scale) as f64,
                });
            }
            // 弹层是**游离子树**(不挂任何父,渲染壳按注册表布局)。
            // 语义树里必须把它们接到 root 名下,否则对话框/菜单对屏幕阅读器
            // 整个不存在 —— 可达性以 children 为准
            let mut kids: Vec<NodeId> = n.children.iter().map(|c| node_id(*c)).collect();
            if id == inner.root {
                kids.extend(inner.overlays.iter().map(|e| node_id(e.root)));
            }
            node.set_children(kids);
            if let Some(e) = inner.overlays.iter().find(|e| e.root == id)
                && e.modal
            {
                // 模态:AT 应把注意力限制在这棵子树内(与命中层的区间阻断同源)
                node.set_modal();
            }
            out.push((node_id(id), node));
            for c in &n.children {
                walk(inner, *c, rects, scale, out);
            }
            if id == inner.root {
                let roots: Vec<sv_ui::ViewId> = inner.overlays.iter().map(|e| e.root).collect();
                for r in roots {
                    walk(inner, r, rects, scale, out);
                }
            }
        }
        walk(inner, inner.root, &rects, scale, &mut nodes);
        (
            nodes,
            node_id(inner.root),
            // focus 每次必填:无焦点时填 root(调研 24 §4.2)
            inner.focused.map(node_id).unwrap_or(node_id(inner.root)),
        )
    })
}

/// 弹层根的角色:层与 modal 位是树里**确实存在**的信息,不是猜的
/// (调研 25:离散层 Base→Popup→Tooltip)
fn overlay_role(inner: &sv_ui::DocumentInner, id: sv_ui::ViewId) -> Option<Role> {
    let e = inner.overlays.iter().find(|e| e.root == id)?;
    Some(match e.layer {
        OverlayLayer::Tooltip => Role::Tooltip,
        OverlayLayer::Popup if e.modal => Role::Dialog,
        OverlayLayer::Popup => Role::Menu,
    })
}

/// 一次 AT 滚动请求走多少(逻辑 px)。与滚轮的行滚一致,手感统一
const A11Y_SCROLL_STEP: f32 = 40.0;

/// AccessKit 动作回派(纯逻辑,离屏可测):Click → 点击回调,
/// Focus/Blur → 焦点链,Scroll* → 滚动偏移。未知动作返回 false
pub fn dispatch_action(doc: &Doc, action: Action, target: NodeId) -> bool {
    let id = sv_ui::view_id_from_ffi(target.0);
    match action {
        Action::Click => {
            if let Some(h) = doc.click_handler(id) {
                h();
                true
            } else {
                false
            }
        }
        Action::Focus => {
            if doc.focusable(id) {
                doc.focus(id);
                true
            } else {
                false
            }
        }
        Action::Blur => {
            if doc.focused() == Some(id) {
                doc.blur();
                true
            } else {
                false
            }
        }
        // AT 主动滚动(把视口外的元素带进来)。上界钳制由布局侧负责,
        // 这里只按步长推 —— 与滚轮同一条写入口
        Action::ScrollUp | Action::ScrollDown => {
            let (x, y) = doc.scroll_of(id);
            let dy = if action == Action::ScrollDown {
                A11Y_SCROLL_STEP
            } else {
                -A11Y_SCROLL_STEP
            };
            doc.set_scroll(id, x, (y + dy).max(0.0));
            true
        }
        _ => false,
    }
}
