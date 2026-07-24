//! 绑定原语调用词汇表 —— **双前端共享的 codegen 内核**(ADR-2 无悔三步 ①)
//!
//! 背景:`view!` 宏(sv-macro)与 `.svelte` 编译器(本 crate)编译目标相同 ——
//! 都是 sv-ui 的绑定原语。两边曾各自手写同一套 `quote!`,于是"改一个原语签名
//! 要同步改两处 codegen 与两处测试"(CLAUDE.md 里那条纪律就是这么来的)。
//! 本模块把**调用形状与闭包协议**收成一份,两个前端都从这里发射。
//!
//! 边界(刻意不搬的东西):
//! - **解析与 IR 留在各自前端**:`view!` 是 Rust token 语法(表达式带真 span),
//!   `.svelte` 是文本语法(表达式是带偏移的源码串,还要过 runes 改写)。硬合成
//!   一份 IR 会把宏路径的 span 精度赔进去 —— 那正是 ADR-2 保留双前端的理由。
//! - **属性名表与错误信息留在各自前端**:两边的表面语法本就不同
//!   (`on_click(闭包)` vs `onclick={闭包}`)。
//!
//! 于是本模块只认最后一步:**给定元素变量名与已备好的表达式 token,
//! 发射对 sv-ui 的调用**。签名变更从此只改这里一处。
//!
//! 生成代码约定(两个前端都遵守):
//! - sv-ui/sv-reactive 一律走 `::sv_ui::` / `::sv_reactive::` 绝对路径;
//! - 作用域内有 `__doc: ::sv_ui::Doc`(owned)与父节点变量;
//! - 重建闭包(if/each/key)参数是 `&Doc`,体内首行 clone 成 owned,
//!   子节点代码因此在任何层级长一个样。

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

/// 场景树元素种类(对应 `sv_ui::ElementKind` 的构造器)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElemKind {
    View,
    Text,
    Button,
    Checkbox,
    TextInput,
    /// 动画叶子:建一个 `ElementKind::Animation` 节点(素材由壳侧后续注册)
    Animation,
}

/// `let #el = __doc.create_xxx(...);`
/// `label` 只对 Text/Button 有意义(其余忽略)
pub fn create(el: &Ident, kind: ElemKind, label: &str) -> TokenStream {
    match kind {
        ElemKind::View => quote! { let #el = __doc.create_view(); },
        ElemKind::Text => quote! { let #el = __doc.create_text(#label); },
        ElemKind::Button => quote! { let #el = __doc.create_button(#label); },
        ElemKind::Checkbox => quote! { let #el = __doc.create_checkbox(); },
        ElemKind::TextInput => quote! { let #el = __doc.create_text_input(); },
        // 占位载荷:素材由 `sv_shell::register_*` 后经 `set_anim_data(#el, ..)` 接入
        ElemKind::Animation => {
            quote! { let #el = __doc.create_animation(::sv_ui::AnimData::placeholder()); }
        }
    }
}

/// `__doc.append(#parent, #el);`
pub fn append(parent: &Ident, el: &Ident) -> TokenStream {
    quote! { __doc.append(#parent, #el); }
}

/// 文本段:字面量或插值表达式
pub enum TextPart {
    Lit(String),
    Expr(TokenStream),
}

/// 全静态时返回拼接后的字面量(可省掉 `bind_text`),否则 `None`
pub fn static_text(parts: &[TextPart]) -> Option<String> {
    let mut s = String::new();
    for p in parts {
        match p {
            TextPart::Lit(t) => s.push_str(t),
            TextPart::Expr(_) => return None,
        }
    }
    Some(s)
}

/// `::sv_ui::bind_text(&__doc, #el, move || { …拼串… });`
/// 一个文本节点**只出一个绑定**(静态段与插值混排也是),这是"零 diff"的
/// 直接体现:文本变化只重跑这一个闭包并写回该节点
pub fn bind_text(el: &Ident, parts: &[TextPart]) -> TokenStream {
    let pushes = parts.iter().filter_map(|p| match p {
        TextPart::Lit(t) if t.is_empty() => None,
        TextPart::Lit(t) => Some(quote! { __s.push_str(#t); }),
        TextPart::Expr(e) => Some(quote! { __s.push_str(&(#e).to_string()); }),
    });
    quote! {
        ::sv_ui::bind_text(&__doc, #el, move || {
            let mut __s = ::std::string::String::new();
            #(#pushes)*
            __s
        });
    }
}

/// 重建闭包(if 分支 / key 块):`Fn(&Doc, ViewId)`。
/// `prelude` 给前端插自己的东西(`.svelte` 的普通变量预克隆);空体给
/// `|_, _| {}` 免 unused 警告
pub fn rebuild_closure(body: TokenStream, prelude: TokenStream) -> TokenStream {
    if body.is_empty() {
        return quote! { |_, _| {} };
    }
    quote! {
        move |__doc, __parent| {
            let __doc: ::sv_ui::Doc = __doc.clone();
            #prelude
            #body
        }
    }
}

/// `::sv_ui::if_block(&__doc, #parent, #cond_closure, #then_c, #else_c);`
///
/// `cond_closure` 是**已建好**的 `move || 条件` 闭包(由前端负责,因为它可能
/// 需要外层捕获份 —— cond 与 then/else 是同级 move 闭包,同引一个非 Copy
/// plain 变量时各需一份所有权,见 codegen 的 `with_captured_plain`)。
pub fn if_block(
    parent: &Ident,
    cond_closure: TokenStream,
    then_closure: TokenStream,
    else_closure: TokenStream,
) -> TokenStream {
    quote! {
        ::sv_ui::if_block(&__doc, #parent, #cond_closure, #then_closure, #else_closure);
    }
}

/// `::sv_ui::each_block(&__doc, #parent, move || #items, #row);`
/// `row` 是 `Fn(&Doc, ViewId, &Item, usize)`(由前端按自己的模式绑定拼)
pub fn each_block(parent: &Ident, items: TokenStream, row: TokenStream) -> TokenStream {
    quote! {
        ::sv_ui::each_block(&__doc, #parent, move || #items, #row);
    }
}

// ---------------------------------------------------------------------------
// 事件 / 绑定
// ---------------------------------------------------------------------------

/// `__doc.set_on_click(#el, #handler);`
pub fn on_click(el: &Ident, handler: TokenStream) -> TokenStream {
    quote! { __doc.set_on_click(#el, #handler); }
}

/// 键盘回调 **+ 自动设 focusable**:不自动设位回调永远收不到事件
/// (floem 教训,调研 20)。两个前端都必须带上这一行,故收在这里。
///
/// 按下/抬起共用 sv-ui 的**同一个槽位**,所以在这里按相位分派而不是设两次
/// ——设两次只会互相覆盖(与 focus_change 同一类坑)
pub fn key_handlers(el: &Ident, down: Option<TokenStream>, up: Option<TokenStream>) -> TokenStream {
    if down.is_none() && up.is_none() {
        return TokenStream::new();
    }
    let d = down.map_or(quote! { let __kd = |_: &::sv_ui::KeyEvent| {}; }, |e| {
        quote! { let __kd = #e; }
    });
    let u = up.map_or(quote! { let __ku = |_: &::sv_ui::KeyEvent| {}; }, |e| {
        quote! { let __ku = #e; }
    });
    quote! {
        __doc.set_focusable(#el, true);
        {
            #d
            #u
            __doc.set_on_key(#el, move |__e| {
                if __e.is_up() { __ku(__e); } else { __kd(__e); }
            });
        }
    }
}

/// focus/blur 合成进单一 `set_on_focus_change`(sv-ui 只有一个回调槽 ——
/// 分开设会互相覆盖,这也是本函数存在的理由)。缺席的一侧补空闭包;
/// `pseudo_state` 是 `:focus` 伪类的状态信号(有则先写它再调用户回调)
pub fn focus_change(
    el: &Ident,
    on_focus: Option<TokenStream>,
    on_blur: Option<TokenStream>,
    pseudo_state: Option<TokenStream>,
) -> TokenStream {
    let f = on_focus.map_or(quote! { let __uf = || {}; }, |e| quote! { let __uf = #e; });
    let b = on_blur.map_or(quote! { let __ub = || {}; }, |e| quote! { let __ub = #e; });
    let set_state = pseudo_state.map_or(TokenStream::new(), |s| quote! { #s.set(__f); });
    quote! {
        {
            #f
            #b
            __doc.set_on_focus_change(#el, move |__f| {
                #set_state
                if __f { __uf(); } else { __ub(); }
            });
        }
    }
}

/// `__doc.set_placeholder(#el, #value);`
pub fn placeholder(el: &Ident, value: TokenStream) -> TokenStream {
    quote! { __doc.set_placeholder(#el, #value); }
}

/// `bind:value` 双向:effect 写(signal → 树)+ `set_on_input` 读(树 → signal)
pub fn bind_value(el: &Ident, signal: TokenStream) -> TokenStream {
    quote! {
        {
            let __b_sig = #signal;
            let __b_doc = __doc.clone();
            let __b_el = #el;
            ::sv_reactive::effect(move || { __b_doc.set_input_value(__b_el, &__b_sig.get()); });
            __doc.set_on_input(#el, move |__v| __b_sig.set(__v.to_string()));
        }
    }
}

/// `__doc.set_on_input(#el, #handler);`
pub fn on_input(el: &Ident, handler: TokenStream) -> TokenStream {
    quote! { __doc.set_on_input(#el, #handler); }
}

/// `__doc.set_on_submit(#el, #handler);`
pub fn on_submit(el: &Ident, handler: TokenStream) -> TokenStream {
    quote! { __doc.set_on_submit(#el, #handler); }
}

/// `__doc.set_on_scroll(#el, #handler);`(签名 `Fn(f32, f32)`)
pub fn on_scroll(el: &Ident, handler: TokenStream) -> TokenStream {
    quote! { __doc.set_on_scroll(#el, #handler); }
}

/// `::sv_ui::bind_scroll_y(&__doc, #el, #signal);`(纵向滚动双向桥)
pub fn bind_scroll_y(el: &Ident, signal: TokenStream) -> TokenStream {
    quote! { ::sv_ui::bind_scroll_y(&__doc, #el, #signal); }
}

/// 无障碍名称。静态串直接设;表达式包 effect(响应式跟随)
pub fn aria_label(el: &Ident, value: TokenStream, reactive: bool) -> TokenStream {
    if !reactive {
        return quote! { __doc.set_accessible_label(#el, #value); };
    }
    quote! {
        {
            let __a_doc = __doc.clone();
            let __a_el = #el;
            ::sv_reactive::effect(move || {
                __a_doc.set_accessible_label(__a_el, &(#value));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::format_ident;

    /// 词汇表是两个前端的**唯一**发射口:形状变了这里先红,
    /// 而不是等某个前端的端到端测试才发现
    #[test]
    fn emitted_shapes_are_stable() {
        let el = format_ident!("__el0");
        let parent = format_ident!("__parent");
        let s = |ts: TokenStream| ts.to_string();

        assert!(s(create(&el, ElemKind::Text, "hi")).contains("create_text (\"hi\")"));
        assert!(s(create(&el, ElemKind::TextInput, "")).contains("create_text_input ()"));
        assert!(s(append(&parent, &el)).contains("append (__parent , __el0)"));

        // 自动 focusable 是 key_handlers 的一部分,少了它回调收不到事件
        let k = s(key_handlers(
            &el,
            Some(quote! { |e: &::sv_ui::KeyEvent| {} }),
            None,
        ));
        assert!(k.contains("set_focusable (__el0 , true)") && k.contains("set_on_key"));
        // 按下/抬起共用一个槽位,必须在闭包里按相位分派
        assert!(k.contains("is_up ()"));
        assert!(
            s(key_handlers(&el, None, None)).is_empty(),
            "都没有就不该发射"
        );

        // 全静态文本不该产生绑定
        let parts = vec![TextPart::Lit("a".into()), TextPart::Lit("b".into())];
        assert_eq!(static_text(&parts).as_deref(), Some("ab"));
        let mixed = vec![TextPart::Lit("n=".into()), TextPart::Expr(quote! { n })];
        assert!(static_text(&mixed).is_none());
        assert!(s(bind_text(&el, &mixed)).contains("bind_text (& __doc , __el0"));

        // 空分支给 |_, _| {} 免 unused 警告
        assert_eq!(
            s(rebuild_closure(TokenStream::new(), TokenStream::new())),
            "| _ , _ | { }"
        );
        assert!(
            s(rebuild_closure(quote! { let x = 1; }, TokenStream::new()))
                .contains("let __doc : :: sv_ui :: Doc = __doc . clone ()")
        );

        // bind:value 是"effect 写 + on_input 读"一对,缺一边就是单向
        let bv = s(bind_value(&el, quote! { sig }));
        assert!(bv.contains("set_input_value") && bv.contains("set_on_input"));

        // focus_change 带伪类状态时先写状态再调用户回调
        let with_state = s(focus_change(&el, None, None, Some(quote! { __fc })));
        assert!(with_state.contains("__fc . set (__f)"));

        // aria-label 静态/响应式两种形态
        assert!(!s(aria_label(&el, quote! { "x" }, false)).contains("effect"));
        assert!(s(aria_label(&el, quote! { name }, true)).contains("effect"));
    }
}
