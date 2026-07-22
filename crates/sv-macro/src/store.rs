//! `#[derive(Store)]`:结构体 → **字段级信号**(ADR-1 里 Proxy 深层响应的替代品)
//!
//! 问题:`Signal<Settings>` 是一颗粒度过粗的信号 —— 改 `volume` 会把只读
//! `theme` 的 effect 也叫醒。Svelte 用 Proxy 做深层响应,Rust 里没有那个口子
//! (也不想要那份运行时开销),于是走编译期:每个字段一个 `Signal`。
//!
//! ```ignore
//! #[derive(Store, Clone, PartialEq)]
//! struct Settings { theme: String, volume: f32 }
//!
//! let s = SettingsStore::new(Settings { theme: "dark".into(), volume: 0.8 });
//! s.volume.set(0.5);            // 只叫醒读 volume 的 effect
//! let snap: Settings = s.snapshot();  // 读全部字段(会订阅全部)
//! s.apply(other);               // 整体写回,**只写值变了的字段**
//! ```
//!
//! 刻意的边界:
//! - 只支持**具名字段**的结构体(元组结构体/枚举没有可读的字段名,
//!   store 的意义也就没了);
//! - **不做嵌套 store**:嵌套结构体的字段仍是一个整体信号。想更细就给
//!   内层也 derive 一次,自己组合 —— 自动递归会让类型与所有权难以预料。
//! - 字段类型需 `Clone + PartialEq + 'static`:前者用于快照,后者是
//!   `apply` 的剪枝依据(没有剪枝,字段级信号就白做了)。

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields};

pub fn derive(input: DeriveInput) -> Result<TokenStream, syn::Error> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(Store)] 只支持结构体",
        ));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[derive(Store)] 只支持具名字段的结构体(元组结构体没有字段名可用)",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "#[derive(Store)] 暂不支持泛型结构体",
        ));
    }

    let name = &input.ident;
    let store = format_ident!("{name}Store");
    let vis = &input.vis;
    let fields: Vec<_> = named.named.iter().collect();
    let fnames: Vec<_> = fields.iter().map(|f| f.ident.clone().unwrap()).collect();
    let ftys: Vec<_> = fields.iter().map(|f| &f.ty).collect();
    let doc = format!("[`{name}`] 的字段级信号 store(由 `#[derive(Store)]` 生成)");

    Ok(quote! {
        #[doc = #doc]
        #[derive(Clone, Copy)]
        #vis struct #store {
            #( #vis #fnames: ::sv_reactive::Signal<#ftys> ),*
        }

        impl #store {
            /// 用一个初值建 store:每个字段一个独立信号
            pub fn new(value: #name) -> Self {
                Self {
                    #( #fnames: ::sv_reactive::state(value.#fnames) ),*
                }
            }

            /// 读回整值。**会订阅所有字段** —— 只想跟一个字段就直接读那个信号
            pub fn snapshot(&self) -> #name {
                #name {
                    #( #fnames: self.#fnames.get() ),*
                }
            }

            /// 整体写回,**只写值变了的字段**:没变的字段一个 effect 都不会醒。
            /// 这正是字段级信号相对 `Signal<整个结构体>` 的意义所在
            pub fn apply(&self, value: #name) {
                ::sv_reactive::batch(|| {
                    #(
                        if self.#fnames.with(|old| *old != value.#fnames) {
                            self.#fnames.set(value.#fnames);
                        }
                    )*
                });
            }
        }

        impl #name {
            /// 转成字段级信号 store(等价于 `XxxStore::new(self)`)
            pub fn into_store(self) -> #store {
                #store::new(self)
            }
        }
    })
}
