//! AccessKit 语义树映射(调研 24 §4.1):场景树 → TreeUpdate 纯函数。
//!
//! 与 RecordingPainter 同哲学:**零窗口零平台即可金样测试**。
//! - NodeId = ViewId 的 slotmap ffi 值(含世代号,删后不复用,天然满足
//!   AccessKit 的 id 稳定性要求);
//! - bounds 取 `Placed.rect`(与命中测试同源,天然一致);逻辑 px 上报,
//!   乘 scale 出物理——adapter 侧期望窗口坐标(平台实测校准项,调研 24 风险 5);
//! - 父子结构回 `DocumentInner` 读(Placed 是平铺画序);
//! - `TreeUpdate.focus` 每次必填(无焦点填 root)——焦点链(R1)在此兑现;
//! - v1 全量 TreeUpdate(协议合法;虚拟化后节点数小),增量列档 B。

use std::collections::HashMap;

use accesskit::{Action, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};
use sv_ui::{Doc, ElementKind, view_id_ffi};

use crate::render::{Placed, Rect};

fn node_id(id: sv_ui::ViewId) -> NodeId {
    NodeId(view_id_ffi(id))
}

/// 全量语义树(逻辑坐标 × scale = 物理 px bounds)
pub fn build_tree_update(doc: &Doc, placed: &[Placed], scale: f32) -> TreeUpdate {
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
            let role = match n.kind {
                ElementKind::View => Role::GenericContainer,
                ElementKind::Text => Role::Label,
                ElementKind::Button => Role::Button,
                ElementKind::Checkbox => Role::CheckBox,
                ElementKind::TextInput => Role::TextInput,
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
            if let Some(r) = rects.get(&id) {
                node.set_bounds(accesskit::Rect {
                    x0: (r.x * scale) as f64,
                    y0: (r.y * scale) as f64,
                    x1: ((r.x + r.w) * scale) as f64,
                    y1: ((r.y + r.h) * scale) as f64,
                });
            }
            node.set_children(n.children.iter().map(|c| node_id(*c)).collect::<Vec<_>>());
            out.push((node_id(id), node));
            for c in &n.children {
                walk(inner, *c, rects, scale, out);
            }
        }
        walk(inner, inner.root, &rects, scale, &mut nodes);

        let mut tree = Tree::new(node_id(inner.root));
        tree.toolkit_name = Some("sv".into());
        TreeUpdate {
            nodes,
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            // focus 每次必填:无焦点时填 root(调研 24 §4.2)
            focus: inner.focused.map(node_id).unwrap_or(node_id(inner.root)),
        }
    })
}

/// AccessKit 动作回派(纯逻辑,离屏可测):Click → 点击回调,
/// Focus/Blur → 焦点链。未知动作返回 false
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
        _ => false,
    }
}
