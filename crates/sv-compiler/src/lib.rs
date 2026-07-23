//! # sv-compiler
//!
//! `.svelte` 单文件组件编译器 — **编译器路线**(相对于 `sv-macro` 的 proc-macro 路线)。
//!
//! 一个 `.svelte` 文件 = `<script>`(Rust + runes)+ 模板(原汁 Svelte 语法):
//!
//! ```text
//! <script>
//! let count = $state(0i32);
//! let double = $derived(count * 2);
//! </script>
//!
//! <view style="padding:24; gap:12">
//!   <text>Count: {count} · 双倍 = {double}</text>
//!   <button onclick={|| count += 1}>+1</button>
//!   {#if count > 5}
//!     <text fg="#ff3e00">超过 5 了!</text>
//!   {/if}
//! </view>
//! ```
//!
//! 编译器做 proc-macro 做不到的事:
//! - **runes 源变换**:整个 script 作用域内,裸 `count` 读改写成 `count.get()`,
//!   `count = x` / `count += x` 改写成 `.set` / `.update` —— Svelte 5 的隐式反应性;
//! - 模板语法不受 Rust tokenizer 约束:免引号文本、`{#if}{:else}{/if}`、`onclick={..}`;
//! - 产物是**人类可读**的 Rust 源码(prettyplease 格式化),定点更新、零 diff。
//!
//! 构建集成:build.rs 里调用 [`build`],生成代码进 OUT_DIR,`include!` 引入。

pub mod check;
mod codegen;
/// 绑定原语调用词汇表:双前端共享 codegen 的最终发射口(见 emit.rs 头部说明)
pub mod emit;
mod script;
mod sfc;
/// 生成 `.rs` ↔ `.svelte` 的位置映射(`sv check` 把 rustc 的诊断搬回 `.svelte` 靠它)
pub mod sourcemap;
mod style;
/// **双前端共享的模板 IR**(ADR-2 内核合并):`.svelte` 的模板 parser 与
/// `view!` 宏的 token parser 都产出这套节点,codegen 只有一份。
/// 表达式载荷是双态 [`template::ExprSrc`]——文本前端带字节偏移,
/// 宏前端带真 span 的 token,span 精度因此不被合并牺牲。
pub mod template;

use std::fmt;
use std::path::Path;

/// 编译错误,带 .svelte 文件内的 1-based 行/列
#[derive(Debug)]
pub struct CompileError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl CompileError {
    pub(crate) fn at_offset(source: &str, offset: usize, message: impl Into<String>) -> Self {
        let (line, col) = line_col(source, offset);
        CompileError {
            message: message.into(),
            line,
            col,
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for CompileError {}

pub(crate) fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.chars().rev().take_while(|c| *c != '\n').count() + 1;
    (line, col)
}

/// 组件 props 签名注册表(caller 侧编译组件标签时查询)。
/// 由 [`build`] 在第一遍扫描时自动构建;单元测试可手工插入。
#[derive(Default)]
pub struct PropsRegistry {
    map: std::collections::HashMap<String, PropsSig>,
}

pub struct PropsSig {
    /// `None` = 该组件没有声明 $props(调用不带 props 参数);
    /// `Some(vec![])` = 声明了空 $props(调用带空结构体)——两者函数签名不同
    pub fields: Option<Vec<PropsSigField>>,
}

pub struct PropsSigField {
    pub name: String,
    /// 有默认值时,caller 侧生成 `XxxProps::default_<name>()` 调用
    /// (默认值表达式本体只存在于 callee 生成代码里,求值语境单一)
    pub has_default: bool,
    /// `$bindable(T)` 双向 prop:caller 传裸句柄,支持 `bind:name={x}` 语法
    pub bindable: bool,
}

impl PropsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, fn_name: impl Into<String>, sig: PropsSig) {
        self.map.insert(fn_name.into(), sig);
    }

    pub(crate) fn get(&self, fn_name: &str) -> Option<&PropsSig> {
        self.map.get(fn_name)
    }
}

/// 编译一份 .svelte 源码(无组件注册表;模板里出现组件标签会报"未知组件")
pub fn compile(source: &str, fn_name: &str) -> Result<String, CompileError> {
    compile_with(source, fn_name, &PropsRegistry::new())
}

/// 双前端共享内核的宏侧入口:模板 IR 节点 → 建树/绑定语句序列(TokenStream)。
///
/// `view!` 宏(sv-macro)把 token 解析成 [`template::Node`](template::Node)
/// 后从这里走**同一份 codegen**。不带 script/样式表/props 上下文:宏模板的
/// 表达式是 [`template::ExprSrc::Tokens`](template::ExprSrc)(用户亲手写的
/// 最终 Rust,带真 span),不过 runes 改写、普通变量预克隆与 sourcemap 记录。
pub fn generate_template(
    nodes: &[template::Node],
) -> Result<proc_macro2::TokenStream, CompileError> {
    codegen::generate_template(nodes)
}

/// 编译一份 .svelte 源码,返回生成的 Rust 源码
/// (`pub fn <fn_name>(doc, parent[, props])`,声明了 $props 时附带 props 结构体)
pub fn compile_with(
    source: &str,
    fn_name: &str,
    registry: &PropsRegistry,
) -> Result<String, CompileError> {
    compile_inner(source, fn_name, registry).map(|(code, _)| code)
}

/// 编译产物 + 位置映射
pub struct Compiled {
    pub code: String,
    pub map: sourcemap::SourceMap,
}

/// 编译并同时产出 source map。
///
/// `sv_path` 会**原样**写进 map 的 `sv` 字段,请传绝对路径:build.rs 的 cwd 是
/// 包根,`sv check` 的 cwd 是 workspace 根,写相对路径两边对不上。
pub fn compile_mapped(
    source: &str,
    fn_name: &str,
    registry: &PropsRegistry,
    sv_path: &str,
) -> Result<Compiled, CompileError> {
    sourcemap::begin(source);
    let r = compile_inner(source, fn_name, registry);
    let out = r.map(|(code, anchors)| {
        let map = sourcemap::finish_map(sv_path.to_string(), source, &code, anchors);
        Compiled { code, map }
    });
    // 记录器是 thread_local 且持有整段 provenance,失败路径也必须收干净
    sourcemap::end();
    out
}

fn compile_inner(
    source: &str,
    fn_name: &str,
    registry: &PropsRegistry,
) -> Result<(String, sourcemap::Anchors), CompileError> {
    let sfc = sfc::split(source)?;
    let script = match &sfc.script {
        Some(block) => script::transform(source, block)?,
        None => script::ScriptOutput::empty(),
    };
    let sheet = match &sfc.style {
        Some(span) => style::parse_style_block(source, span.text(source), span.start)?,
        None => style::StyleSheet::default(),
    };
    let nodes = template::parse(source, &sfc.template)?;
    codegen::generate(source, fn_name, &script, &nodes, registry, &sheet)
}

/// 编译单个 .svelte 文件,组件函数名取自文件名(snake_case 化)
pub fn compile_file(path: &Path) -> Result<String, String> {
    compile_file_with(path, &PropsRegistry::new())
}

pub fn compile_file_with(path: &Path, registry: &PropsRegistry) -> Result<String, String> {
    let source =
        std::fs::read_to_string(path).map_err(|e| format!("{}: 读取失败: {e}", path.display()))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("component");
    let fn_name = sanitize_fn_name(stem);
    compile_with(&source, &fn_name, registry).map_err(|e| format!("{}:{e}", path.display()))
}

fn sanitize_fn_name(stem: &str) -> String {
    let mut out = String::new();
    for (i, ch) in stem.chars().enumerate() {
        if ch.is_alphanumeric() {
            // CamelCase → snake_case
            if ch.is_uppercase() && i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() || out.chars().next().unwrap().is_ascii_digit() {
        format!("component_{out}")
    } else {
        out
    }
}

/// build.rs 入口:递归扫描 `src_dir` 下所有 .svelte,编译到 `$OUT_DIR/<fn_name>.rs`。
/// 两遍:先扫全部文件的 $props 声明建注册表(组件互相引用),再逐个编译。
/// 编译失败直接 panic(cargo 会把错误显示出来,格式 `文件:行:列: 消息`)。
pub fn build(src_dir: impl AsRef<Path>) {
    let out_dir = std::env::var("OUT_DIR").expect("sv-compiler::build 只能在 build.rs 里调用");
    let mut files = Vec::new();
    collect_sv_files(src_dir.as_ref(), &mut files);

    // 第一遍:props 签名注册表
    let mut registry = PropsRegistry::new();
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("component");
        let fn_name = sanitize_fn_name(stem);
        registry.insert(
            fn_name,
            PropsSig {
                fields: props_signature(&source),
            },
        );
    }

    // 第二遍:编译
    for path in files {
        println!("cargo::rerun-if-changed={}", path.display());
        // 报错一律用绝对路径:build.rs 的 cwd 是包根,而看这条错误的
        // (`sv check` / VS Code 的 problemMatcher)cwd 是 workspace 根
        let abs = abs_path(&path);
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => panic!("\n\n.svelte 读取失败\n  --> {abs}: {e}\n"),
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("component");
        let fn_name = sanitize_fn_name(stem);
        let compiled = match compile_mapped(&source, &fn_name, &registry, &abs) {
            Ok(c) => c,
            Err(e) => panic!("\n\n.svelte 编译失败\n  --> {abs}:{e}\n"),
        };
        // 保险丝熔断 = 这个文件的诊断全都回不到 .svelte。它不该只在 .svmap 里留个
        // 字段等人去读:烧了必须有指示灯,否则用户只会看到一片"落在胶水上"
        if let Some(why) = &compiled.map.blown {
            println!(
                "cargo::warning={abs}: source map 锚点并行走失配({why}),该文件的 rustc 诊断将无法回映射到 .svelte,请上报"
            );
        }
        let out = Path::new(&out_dir).join(format!("{fn_name}.rs"));
        write_if_changed(&out, &compiled.code);
        // map 与 .rs **同一次写盘**:分两次生成必然漂移,`sv check` 会靠
        // sv_hash 拒绝用过期的 map
        write_if_changed(&out.with_extension("rs.svmap"), &compiled.map.to_text());
    }
    println!("cargo::rerun-if-changed={}", src_dir.as_ref().display());
}

/// 内容不变就不写盘:build.rs 与编辑器/rust-analyzer 的 watcher 同时盯着
/// OUT_DIR,无谓的 mtime 变化会引发整类"重编译抖动"的玄学问题
fn write_if_changed(path: &Path, content: &str) {
    if std::fs::read_to_string(path).is_ok_and(|old| old == content) {
        return;
    }
    std::fs::write(path, content).expect("写入生成代码失败");
}

/// 绝对路径,并剥掉 Windows 的 `\\?\` UNC 前缀(它进不了 problemMatcher 的正则)
fn abs_path(path: &Path) -> String {
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// 轻量提取一份 .svelte 源码的 $props 签名(build 第一遍用);
/// 返回 None 表示该文件没有声明 $props
fn props_signature(source: &str) -> Option<Vec<PropsSigField>> {
    let sfc = sfc::split(source).ok()?;
    let span = sfc.script.as_ref()?;
    let (_, decl) = script::extract_props(span.text(source), None).ok()?;
    let decl = decl?;
    Some(
        decl.fields
            .iter()
            .map(|f| PropsSigField {
                name: f.name.clone(),
                has_default: f.default.is_some(),
                bindable: f.bindable,
            })
            .collect(),
    )
}

fn collect_sv_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sv_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "svelte") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER: &str = r##"<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<view style="padding:24; gap:12">
  <text style="font-size:28">sv 计数器</text>
  <text>Count: {count} · 双倍 = {double}</text>
  <view style="direction:row; gap:8">
    <button style="bg:#ff3e00; fg:#ffffff; padding:10; radius:8" onclick={|| count += 1}>+1</button>
    <button onclick={|| count = 0}>归零</button>
  </view>
  {#if count > 5}
    <text fg="#ff3e00">超过 5 了!</text>
  {:else if count < 0}
    <text>负数啦</text>
  {:else}
    <text>还早</text>
  {/if}
</view>
"##;

    #[test]
    fn counter_compiles() {
        let code = compile(COUNTER, "counter").expect("应编译成功");
        // runes 源变换
        assert!(
            code.contains("::sv_reactive::state(0i32)"),
            "$state 应展开:\n{code}"
        );
        assert!(
            code.contains("::sv_reactive::derived(move || count.get() * 2)"),
            "$derived 应闭包化且读改写:\n{code}"
        );
        assert!(
            code.contains("count.update(|__v| *__v += __sv_rhs)"),
            "+= 应改写成 RHS 预求值的 update:\n{code}"
        );
        assert!(code.contains("count.set(0)"), "= 应改写成 set:\n{code}");
        // 模板
        assert!(
            code.contains("::sv_ui::if_block"),
            "{{#if}} 应编译成 if_block:\n{code}"
        );
        assert!(
            code.contains("count.get() > 5"),
            "cond 里的读应改写:\n{code}"
        );
        assert!(code.contains("bind_text"), "插值文本应绑定:\n{code}");
        assert!(
            code.contains("create_text(\"sv 计数器\")"),
            "静态文本无绑定:\n{code}"
        );
        // 生成的是合法 Rust
        syn::parse_file(&code).expect("生成代码应能被 syn 解析");
    }

    #[test]
    fn each_block_compiles() {
        let src = r#"<script>
let items = $state(vec![1i32, 2, 3]);
</script>
<view>
  {#each items as n, i}
    <text>{i}: {n}</text>
  {/each}
  <button onclick={|| items.push(9)}>加</button>
</view>
"#;
        // 注:items.push(9) 这种方法调用不改写(v0 限制),用户应写 items = ...;
        // 这里换成合法写法
        let src = src.replace("items.push(9)", "items += vec![]");
        let code = compile(&src, "list").expect("应编译成功");
        assert!(
            code.contains("::sv_ui::each_block"),
            "{{#each}} 应编译成 each_block:\n{code}"
        );
        assert!(code.contains("items.get()"), "列表读应改写:\n{code}");
        syn::parse_file(&code).expect("生成代码应能被 syn 解析");
    }

    #[test]
    fn effect_and_fmt_macro_rewrite() {
        let src = r#"<script>
let count = $state(0i32);
$effect(|| {
    println!("count = {}", count);
});
</script>
<view><text>{count}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("::sv_reactive::effect"),
            "$effect 应展开:\n{code}"
        );
        assert!(
            code.contains("count.get()"),
            "println! 参数里的读也应改写:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn error_reports_line() {
        let src = "<view>\n  {#if count > 5}\n  <text>x</text>\n</view>\n";
        let err = compile(src, "c").unwrap_err();
        assert!(err.message.contains("if"), "应报未闭合 if: {err}");
        assert_eq!(err.line, 2, "错误应定位到 {{#if}} 行: {err}");
    }

    #[test]
    fn unknown_tag_rejected() {
        let err = compile("<div>x</div>", "c").unwrap_err();
        assert!(err.message.contains("div"), "{err}");
    }

    #[test]
    fn fn_name_sanitize() {
        assert_eq!(sanitize_fn_name("Counter"), "counter");
        assert_eq!(sanitize_fn_name("TodoList"), "todo_list");
        assert_eq!(sanitize_fn_name("my-widget"), "my_widget");
    }

    #[test]
    fn each_else_compiles() {
        let src = r#"<script>
let items = $state(Vec::<String>::new());
</script>
<view>
  {#each items as x}
    <text>{x}</text>
  {:else}
    <text>空空如也</text>
  {/each}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("each_block_else"),
            "{{:else}} 应编译成 each_block_else:\n{code}"
        );
        assert!(code.contains("空空如也"));
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn key_block_compiles() {
        let src = r#"<script>
let user = $state(1i32);
</script>
<view>
  {#key user}
    <text>档案面板</text>
  {/key}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("::sv_ui::key_block"),
            "{{#key}} 应编译成 key_block:\n{code}"
        );
        assert!(code.contains("user.get()"), "key 表达式的读应改写:\n{code}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn const_becomes_block_derived() {
        let src = r#"<script>
let count = $state(1i32);
</script>
<view>
  {#if count > 0}
    {@const total = count * 10}
    <text>合计 {total}</text>
  {/if}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("::sv_reactive::derived(move || count.get() * 10)"),
            "{{@const}} 应编译成块级 derived:\n{code}"
        );
        assert!(
            code.contains("(total.get()).to_string()"),
            "后续读 total 应改写:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn style_directive_patches_field() {
        let src = r#"<script>
let size = $state(10.0f32);
</script>
<view>
  <text style="gap:3" style:padding={size * 2.0}>字</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("bind_style_patch"),
            "style: 指令应编译成 patch 绑定:\n{code}"
        );
        assert!(
            code.contains("Edges::all(size.get() * 2.0)"),
            "读应改写:\n{code}"
        );
        assert!(
            code.contains("s.gap = 3f32"),
            "静态 style 属性应共存:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn onkeydown_compiles() {
        let src = r#"<script>
let n = $state(0i32);
</script>
<view onkeydown={|e| n += 1} autofocus>
  <button onfocus={|| n += 10} onblur={|| n += 100}>钮</button>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("set_on_key"),
            "onkeydown 应编译成 set_on_key:\n{code}"
        );
        assert!(
            code.contains("set_focusable"),
            "onkeydown 应自动 set_focusable:\n{code}"
        );
        assert!(
            code.contains("set_on_focus_change"),
            "onfocus/onblur 应合成进 set_on_focus_change:\n{code}"
        );
        assert!(
            code.contains(".focus("),
            "autofocus 应编译成 __doc.focus:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn input_bind_value_two_way() {
        let src = r#"<script>
let name = $state(String::new());
</script>
<view>
  <input placeholder="请输入姓名" bind:value={name} />
  <text>你好,{name}</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("create_text_input"),
            "<input> 应编译成 create_text_input:\n{code}"
        );
        assert!(
            code.contains("set_placeholder"),
            "placeholder 应落地:\n{code}"
        );
        assert!(
            code.contains("set_input_value") && code.contains("set_on_input"),
            "bind:value 应展开为 effect 写 + on_input 读:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn input_events_compile() {
        let src = r#"<script>
let last = $state(String::new());
</script>
<view>
  <input oninput={|v| last = v.to_string()} onsubmit={|v| last = format!("提交:{}", v)} />
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(code.contains("set_on_input"), "oninput:\n{code}");
        assert!(code.contains("set_on_submit"), "onsubmit:\n{code}");
        syn::parse_file(&code).unwrap();
        // bind:value 用在非 input 上应报错
        let bad = "<view><button bind:value={x}>钮</button></view>";
        let err = compile(bad, "c").unwrap_err();
        assert!(err.message.contains("input"), "{err}");
    }

    #[test]
    fn sfc_overflow_and_scroll_bindings_compile() {
        let src = r#"<script>
let y = $state(0.0f32);
</script>
<view style="overflow: scroll; height: 200" bind:scrolly={y}
      onscroll={|_x, _y| ()}>
  <text>长内容</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("Overflow::Scroll"),
            "overflow: scroll 应落 Style.overflow:\n{code}"
        );
        assert!(
            code.contains("bind_scroll_y"),
            "bind:scrolly 应编译成 bind_scroll_y:\n{code}"
        );
        assert!(
            code.contains("set_on_scroll"),
            "onscroll 应编译成 set_on_scroll:\n{code}"
        );
        syn::parse_file(&code).unwrap();
        // overflow: auto 按 scroll 处理;非法值报错
        let auto = compile("<view style=\"overflow: auto\">x</view>", "c").unwrap();
        assert!(auto.contains("Overflow::Scroll"));
        let err = compile("<view style=\"overflow: wrap\">x</view>", "c").unwrap_err();
        assert!(err.message.contains("overflow"), "{err}");
    }

    #[test]
    fn style_c2_flex_keys_compile() {
        let src = r#"<view style="direction:row; justify-content: space-between; align-items: center; flex-wrap: wrap; min-width: 100; max-height: 300">
  <text style="flex-grow: 1; white-space: nowrap; text-align: center">标题</text>
  <view style="align-self: stretch; flex-shrink: 1" />
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        for needle in [
            "JustifyContent::SpaceBetween",
            "AlignItems::Center",
            "FlexWrap::Wrap",
            "min_width",
            "max_height",
            "flex_grow",
            "TextWrap::NoWrap",
            "TextAlign::Center",
            "AlignItems::Stretch",
            "flex_shrink",
        ] {
            assert!(code.contains(needle), "缺 {needle}:\n{code}");
        }
        syn::parse_file(&code).unwrap();
        // 非法值报错
        let err = compile("<view style=\"justify-content: middle\">x</view>", "c").unwrap_err();
        assert!(err.message.contains("justify-content"), "{err}");
    }

    #[test]
    fn sv_overlay_codegen() {
        let src = r#"<script>
let show = $state(false);
</script>
<view>
  <button onclick={|| show = true}>菜单</button>
  <overlay open={show} anchor="below" gap="6" close="outside"
           ondismiss={|| show = false} style="padding:8; bg:#ffffff">
    <text>项目一</text>
  </overlay>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        for needle in [
            "overlay_block",
            "Anchor::Node",
            "Side::Below",
            "CloseBehavior::OnClickOutside",
            "OverlayLayer::Popup",
        ] {
            assert!(code.contains(needle), "缺 {needle}:\n{code}");
        }
        syn::parse_file(&code).unwrap();
        // modal 缺省 close=none;center 锚定
        let src2 = r#"<script>
let show = $state(true);
</script>
<view>
  <overlay open={show} anchor="center" modal>
    <text>对话框</text>
  </overlay>
</view>
"#;
        let code2 = compile(src2, "c").unwrap();
        assert!(code2.contains("Anchor::WindowCenter"));
        assert!(
            code2.contains("CloseBehavior::None"),
            "modal 缺省只能程序关:\n{code2}"
        );
        // open 必填
        let err = compile(
            "<view><overlay anchor=\"below\"><text>x</text></overlay></view>",
            "c",
        )
        .unwrap_err();
        assert!(err.message.contains("open"), "{err}");
    }

    #[test]
    fn aria_label_compiles() {
        let src = r#"<script>
let n = $state(0i32);
</script>
<view>
  <button aria-label="增加计数" onclick={|| n += 1}>+</button>
  <text aria-label={format!("当前 {}", n)}>{n}</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("set_accessible_label"),
            "aria-label 应编译成 set_accessible_label:\n{code}"
        );
        assert!(
            code.contains("::sv_reactive::effect"),
            "动态 aria-label 应走响应式 effect:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn on_keydown_legacy_form_rejected_with_hint() {
        let src = "<view on:keydown={|e| ()}>x</view>";
        let err = compile(src, "c").unwrap_err();
        assert!(
            err.message.contains("onkeydown"),
            "on:keydown 报错应指路属性形态:{err}"
        );
    }

    /// on: 指令已整体移除(对齐 Svelte 5)——on:click 也不再是遗留别名,
    /// 报错必须指路 onclick 属性形态
    #[test]
    fn on_click_legacy_form_rejected_with_hint() {
        let src = "<button on:click={|| ()}>x</button>";
        let err = compile(src, "c").unwrap_err();
        assert!(
            err.message.contains("onclick"),
            "on:click 报错应指路属性形态:{err}"
        );
    }

    #[test]
    fn onclick_svelte5_attr() {
        let src = r#"<script>
let n = $state(0i32);
</script>
<view><button onclick={|| n += 1}>加</button></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(code.contains("set_on_click"), "onclick 应生效:\n{code}");
        assert!(code.contains("n.update(|__v| *__v += __sv_rhs)"));
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn props_callee_generates_struct() {
        let src = r#"<script>
$props {
    label: String,
    times: i32 = 1,
}
let count = $state(0i32);
</script>
<view><text>{label} x {times}</text></view>
"#;
        let code = compile(src, "todo_item").expect("应编译成功");
        assert!(
            code.contains("pub struct TodoItemProps"),
            "$props 应生成结构体:\n{code}"
        );
        assert!(code.contains("pub label: String") && code.contains("pub times: i32"));
        assert!(
            code.contains("props: TodoItemProps"),
            "fn 应带 props 参数:\n{code}"
        );
        assert!(
            code.contains("let TodoItemProps { label, times } = props;"),
            "应解构 props:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn component_call_with_props_and_default() {
        let mut registry = PropsRegistry::new();
        registry.insert(
            "todo_item",
            PropsSig {
                fields: Some(vec![
                    PropsSigField {
                        name: "label".into(),
                        has_default: false,
                        bindable: false,
                    },
                    PropsSigField {
                        name: "times".into(),
                        has_default: true,
                        bindable: false,
                    },
                ]),
            },
        );
        let src = r#"<script>
let name = $state(String::from("洗碗"));
</script>
<view>
  <TodoItem label={name} />
</view>
"#;
        let code = compile_with(src, "app", &registry).expect("应编译成功");
        assert!(
            code.contains("todo_item(") && code.contains("TodoItemProps"),
            "组件标签应编译成函数调用:\n{code}"
        );
        assert!(
            code.contains("label: name.get()"),
            "prop 表达式的读应改写:\n{code}"
        );
        assert!(
            code.contains("times: TodoItemProps::default_times()"),
            "缺省 prop 应调用 callee 侧默认值函数:\n{code}"
        );
        syn::parse_file(&code).unwrap();

        // 缺必填 prop 与未知 prop 都应报错
        let missing = compile_with("<view><TodoItem /></view>", "app", &registry).unwrap_err();
        assert!(missing.message.contains("label"), "{missing}");
        let unknown = compile_with(
            "<view><TodoItem label={1} bogus={2} /></view>",
            "app",
            &registry,
        )
        .unwrap_err();
        assert!(unknown.message.contains("bogus"), "{unknown}");
    }

    #[test]
    fn compound_assign_rhs_preevaluated() {
        // RHS 读同一 signal:必须先求值再进 update 闭包,否则运行期重入 panic
        let src = r#"<script>
let count = $state(1i32);
let bump = || count += count;
</script>
<view><text>{count}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("let __sv_rhs = count.get();"),
            "RHS 应预求值:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn rune_variants() {
        let src = r#"<script>
let a = $state.raw(1i32);
let b = $derived.by(|| {
    let x = a * 2;
    x + 1
});
$effect.pre(|| {
    let _ = b;
});
$inspect(a, b);
let handle_holder = $sig(a);
</script>
<view><text>{b}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("let a = ::sv_reactive::state(1i32)"),
            "$state.raw:\n{code}"
        );
        assert!(
            code.contains("a.get() * 2"),
            "$derived.by 体内读改写:\n{code}"
        );
        assert!(
            code.contains("[inspect]"),
            "$inspect 应生成观察 effect:\n{code}"
        );
        assert!(
            code.contains("let handle_holder = a;"),
            "$sig 应取出裸句柄:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    // ---- 第三批特性:snippet/children/$bindable/keyed each/<style> 块 ----

    #[test]
    fn snippet_and_render_compile() {
        let src = r#"<script>
let count = $state(0i32);
</script>
<view>
  {#snippet badge(label: String, n: i32)}
    <text>{label}: {n}</text>
  {/snippet}
  {@render badge(String::from("计数"), count)}
  {@render badge(String::from("双倍"), count * 2)}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("let badge ="),
            "snippet 应编译成局部闭包:\n{code}"
        );
        assert!(
            code.contains("(badge)(&__doc, __parent, String::from(\"计数\"), count.get())"),
            "{{@render}} 应编译成调用且参数改写:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn component_children_snippet() {
        let mut registry = PropsRegistry::new();
        registry.insert(
            "card",
            PropsSig {
                fields: Some(vec![
                    PropsSigField {
                        name: "title".into(),
                        has_default: false,
                        bindable: false,
                    },
                    PropsSigField {
                        name: "children".into(),
                        has_default: false,
                        bindable: false,
                    },
                ]),
            },
        );
        let src = r#"<script>
let n = $state(1i32);
</script>
<view>
  <Card title={String::from("面板")}>
    <text>内容 {n}</text>
  </Card>
</view>
"#;
        let code = compile_with(src, "app", &registry).expect("应编译成功");
        assert!(
            code.contains("children:") && code.contains("as ::sv_ui::Snippet"),
            "子内容应编译成 children snippet:\n{code}"
        );
        syn::parse_file(&code).unwrap();

        // callee 侧:children prop + {@render children()}
        let callee = r#"<script>
$props { title: String, children: sv_ui::Snippet }
</script>
<view>
  <text>{title}</text>
  {@render children()}
</view>
"#;
        let code2 = compile(callee, "card").expect("callee 应编译成功");
        assert!(code2.contains("(children)(&__doc, __parent)"), "\n{code2}");
        syn::parse_file(&code2).unwrap();
    }

    #[test]
    fn bindable_prop_two_way() {
        // callee:$bindable(T) → Signal<T> 字段 + callee 内隐式反应
        let callee = r#"<script>
$props { value: $bindable(i32), step: i32 = 1 }
</script>
<view>
  <button onclick={|| value += step}>加</button>
  <text>{value}</text>
</view>
"#;
        let code = compile(callee, "stepper").expect("callee 应编译成功");
        assert!(
            code.contains("pub value: ::sv_reactive::Signal<i32>"),
            "$bindable 应展开成 Signal 字段:\n{code}"
        );
        assert!(
            code.contains("value.update(|__v| *__v += __sv_rhs)"),
            "callee 里 bindable 名应参与 runes 改写:\n{code}"
        );
        assert!(code.contains("(value.get()).to_string()"), "\n{code}");
        syn::parse_file(&code).unwrap();

        // caller:bind:value={count} 直接传句柄
        let mut registry = PropsRegistry::new();
        registry.insert(
            "stepper",
            PropsSig {
                fields: Some(vec![
                    PropsSigField {
                        name: "value".into(),
                        has_default: false,
                        bindable: true,
                    },
                    PropsSigField {
                        name: "step".into(),
                        has_default: true,
                        bindable: false,
                    },
                ]),
            },
        );
        let caller = r#"<script>
let count = $state(0i32);
</script>
<view>
  <Stepper bind:value={count} />
  <text>外部视角: {count}</text>
</view>
"#;
        let code2 = compile_with(caller, "app", &registry).expect("caller 应编译成功");
        assert!(
            code2.contains("value: count") && !code2.contains("value: count.get()"),
            "bind: 应传裸句柄而不是快照:\n{code2}"
        );
        syn::parse_file(&code2).unwrap();

        // 非 bindable 字段用 bind: 应报错
        let err = compile_with(
            "<view><Stepper bind:step={1} value={$sig(x)} /></view>",
            "app",
            &registry,
        )
        .unwrap_err();
        assert!(err.message.contains("bindable"), "{err}");
    }

    /// `<textarea>`:与 `<input>` 共用全部输入属性,额外认 rows;
    /// rows 用错地方要报错(而不是静默忽略)
    /// `overflow` 简写写两轴,`overflow-x/-y` 各写一轴(CSS 同款)
    #[test]
    fn overflow_axis_keys_compile() {
        let both = compile("<script></script><view style=\"overflow: scroll\" />", "c")
            .expect("简写应编译成功");
        assert!(
            both.contains("s.overflow = ") && both.contains("s.overflow_x = "),
            "简写应同时写两轴:
{both}"
        );

        let split = compile(
            "<script></script><view style=\"overflow-x: hidden; overflow-y: scroll\" />",
            "c",
        )
        .expect("分轴应编译成功");
        assert!(
            split.contains("s.overflow_x = ::sv_ui::Overflow::Hidden"),
            "
{split}"
        );
        assert!(
            split.contains("s.overflow = ::sv_ui::Overflow::Scroll"),
            "
{split}"
        );
        syn::parse_file(&split).unwrap();

        let err = compile("<script></script><view style=\"overflow-x: 斜着\" />", "c")
            .expect_err("非法值应报错");
        assert!(err.message.contains("overflow-x"), "{}", err.message);
    }

    /// `onkeyup` 与 `onkeydown` 共用 sv-ui 的单一槽位:必须合成一次设入,
    /// 否则后设的把先设的顶掉(R1 档 B)
    #[test]
    fn keyup_and_keydown_share_one_slot() {
        let code = compile(
            "<script></script><view onkeydown={|e| { let _ = e; }} onkeyup={|e| { let _ = e; }} />",
            "c",
        )
        .expect("应编译成功");
        assert_eq!(
            code.matches("set_on_key").count(),
            1,
            "两个回调必须合成一次设入:
{code}"
        );
        assert!(
            code.contains("is_up"),
            "应按相位分派:
{code}"
        );
        assert!(code.contains("set_focusable"), "键盘回调应自动设可获焦");
        syn::parse_file(&code).unwrap();

        // 只写一个也照常工作
        let only_up = compile(
            "<script></script><view onkeyup={|e| { let _ = e; }} />",
            "c",
        )
        .expect("只写 onkeyup 应编译成功");
        assert_eq!(only_up.matches("set_on_key").count(), 1);
    }

    /// `:focus` 伪类(R1 档 B):走焦点链而不是指针;与 onfocus/onblur
    /// **合成一次**设入(sv-ui 只有一个回调槽,分开设会互相覆盖);
    /// 元素自动设为可获焦,否则样式永远不生效
    #[test]
    fn focus_pseudo_class_compiles() {
        let src = r#"<script></script>
<view>
  <view class="card" onfocus={|| {}} />
</view>
<style>
.card { padding: 8; }
.card:focus { background: #eef; }
</style>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("set_focusable"),
            ":focus 应自动设可获焦:
{code}"
        );
        assert_eq!(
            code.matches("set_on_focus_change").count(),
            1,
            ":focus 与 onfocus 必须合成一次设入(否则互相覆盖):
{code}"
        );
        assert!(
            code.contains("__fc"),
            "应有 :focus 状态信号:
{code}"
        );
        syn::parse_file(&code).unwrap();

        // 嵌套形态 &:focus 同样认
        let nested = compile(
            "<script></script><view class=\"b\" /><style>.b { gap: 2; &:focus { gap: 4; } }</style>",
            "c",
        )
        .expect("嵌套 &:focus 应编译成功");
        assert!(nested.contains("__fc"));

        // 未知伪类仍然硬报错(错误信息要提到现在支持哪些)
        let err = compile(
            "<script></script><view class=\"b\" /><style>.b:disabled { gap: 1; }</style>",
            "c",
        )
        .expect_err(":disabled 尚未支持");
        assert!(err.message.contains(":focus"), "{}", err.message);
    }

    #[test]
    fn textarea_compiles() {
        let src = r#"<script>
let note = $state(String::new());
</script>
<view>
  <textarea rows="5" placeholder="写点什么" bind:value={note}
            oninput={|v| { let _ = v; }} />
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("create_text_input") && code.contains("set_multiline"),
            "textarea 应建输入框并开多行:
{code}"
        );
        assert!(
            code.contains("5u16"),
            "rows 应直通:
{code}"
        );
        assert!(code.contains("set_placeholder") && code.contains("set_on_input"));
        syn::parse_file(&code).unwrap();

        // rows 只对 textarea 有意义
        let err = compile("<script></script><view><input rows=\"3\" /></view>", "c")
            .expect_err("input 上写 rows 应报错");
        assert!(err.message.contains("textarea"), "{}", err.message);

        // rows 必须是静态数字
        let err = compile(
            "<script></script><view><textarea rows=\"很多\" /></view>",
            "c",
        )
        .expect_err("非数字 rows 应报错");
        assert!(err.message.contains("整数"), "{}", err.message);
    }

    #[test]
    fn overlay_aria_label_sets_accessible_name() {
        // 真机 C 复核发现:模态对话框读屏只读按钮、不读标题(Dialog 容器无 accessible
        // name)。修复:<overlay aria-label={...}> 给弹层根发 set_accessible_label,
        // a11y 层据此把它作为 Dialog 节点的名称播报。这里验发射;且 title 与正文里的
        // {title} 争用同一普通变量时不该编译失败(借用而非 move)。
        let src = r#"<script>
let open = $state(true);
let title = String::from("确认删除?");
</script>
<view>
  <overlay open={open} anchor="center" modal aria-label={title}>
    <text>{title}</text>
    <text>正文</text>
  </overlay>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功(title 借用不与 {title} 冲突)");
        assert!(
            code.contains("set_accessible_label"),
            "overlay 的 aria-label 应发 set_accessible_label:\n{code}"
        );
        syn::parse_file(&code).unwrap();

        // 未知属性仍报错,且错误信息列出 aria-label
        let err = compile(
            "<script>let o=$state(true);</script><view><overlay open={o} bogus=\"x\"><text>a</text></overlay></view>",
            "c",
        )
        .expect_err("未知 overlay 属性应报错");
        assert!(err.message.contains("aria-label"), "{}", err.message);
    }

    #[test]
    fn animation_compiles() {
        // <animation> 是叶子:建 Animation 节点,认 src/loop/autoplay/label
        let src = r#"<script></script>
<view>
  <animation src="assets/loading.json" loop autoplay label="加载中" />
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("create_animation"),
            "<animation> 应建 Animation 节点:\n{code}"
        );
        // 输出必须是合法 Rust
        syn::parse_file(&code).unwrap();

        // 叶子:带子节点应报错
        let err = compile(
            "<script></script><view><animation>x</animation></view>",
            "c",
        )
        .expect_err("animation 带子节点应报错");
        assert!(err.message.contains("叶子"), "{}", err.message);

        // 未知标签的错误信息里应列出 animation
        let err =
            compile("<script></script><view><anim /></view>", "c").expect_err("未知标签应报错");
        assert!(err.message.contains("animation"), "{}", err.message);
    }

    #[test]
    fn keyed_each_compiles() {
        let src = r#"<script>
let items = $state(vec![(1i32, String::from("甲"))]);
</script>
<view>
  {#each items as it (it.0)}
    <text>{it.1}</text>
  {/each}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("each_block_keyed"),
            "(key) 应走 keyed 版本:\n{code}"
        );
        assert!(code.contains("it.0"), "key 表达式应保留:\n{code}");
        // ADR-7:行拿的是 Signal<T> —— 行内引用改写成 `.get()`(内容变化原地
        // 更新),而 key 闭包拿的仍是裸 `&T`(不能是 `.get()`,那里没有 signal)
        assert!(
            code.contains("it.get().1"),
            "行内绑定名应是反应式(Signal):\n{code}"
        );
        let key_line = code
            .lines()
            .find(|l| l.contains("Clone::clone(__item)"))
            .expect("key 闭包应克隆裸值");
        assert!(
            !key_line.contains(".get()"),
            "key 闭包里的绑定名不该被改写成 .get():\n{key_line}"
        );
        syn::parse_file(&code).unwrap();

        // keyed 行的绑定是 Signal,解构模式没法表达 → 明确报错而不是生成坏代码
        let err = compile(
            "<script>\nlet xs = $state(vec![(1i32, 2i32)]);\n</script>\n\
             <view>{#each xs as (a, b) (a)}<text>{a}</text>{/each}</view>",
            "c",
        )
        .expect_err("keyed + 解构应报错");
        assert!(err.message.contains("单个标识符"), "{}", err.message);

        // keyed + 索引应报错
        let err = compile(
            "<script>\nlet xs = $state(vec![1i32]);\n</script>\n<view>{#each xs as x, i (x)}<text>{x}</text>{/each}</view>",
            "c",
        )
        .unwrap_err();
        assert!(err.message.contains("索引"), "{err}");
    }

    #[test]
    fn style_block_classes() {
        let src = r#"<view>
  <text class="title">标题</text>
  <button class="btn" style="padding:99">按钮</button>
</view>

<style>
.title { font-size: 26; fg: #223344; }
.btn { padding: 8; radius: 6; bg: #ff3e00; }
</style>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("s.font_size = 26f32"),
            "类样式应展开:\n{code}"
        );
        assert!(code.contains("s.corner_radius = 6f32"), "\n{code}");
        // style="" 在类之后:padding 应被 99 覆盖(后写胜出)
        let btn_zone = code.split("create_button").nth(1).unwrap();
        let p8 = btn_zone.find("top: 8f32").expect("类的 padding");
        let p99 = btn_zone.find("top: 99f32").expect("style 的 padding");
        assert!(p8 < p99, "内联 style 应覆盖类:\n{code}");
        syn::parse_file(&code).unwrap();

        // 未知类报错
        let err = compile("<view><text class=\"nope\">x</text></view>", "c").unwrap_err();
        assert!(err.message.contains("nope"), "{err}");
    }

    #[test]
    fn debug_tag_compiles() {
        let src = r#"<script>
let n = $state(1i32);
</script>
<view>
  {@debug n}
  <text>x</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("[debug]") && code.contains("n.get()"),
            "\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    // ---- 第四批特性:补齐矩阵剩余项 ----

    #[test]
    fn comments_and_options_ignored() {
        let src = "<view><!-- 这是注释 --><svelte:options runes />\n<text>x</text></view>";
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            !code.contains("这是注释") && !code.contains("options"),
            "\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn each_without_as() {
        let src = r#"<script>
let n = $state(3usize);
</script>
<view>
  {#each vec![(); n]}
    <text>一行</text>
  {/each}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(code.contains("each_block"), "\n{code}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn shorthand_attr_on_component() {
        let mut registry = PropsRegistry::new();
        registry.insert(
            "card",
            PropsSig {
                fields: Some(vec![PropsSigField {
                    name: "title".into(),
                    has_default: false,
                    bindable: false,
                }]),
            },
        );
        let src = r#"<script>
let title = String::from("你好");
</script>
<view><Card {title} /></view>
"#;
        let code = compile_with(src, "app", &registry).expect("应编译成功");
        assert!(
            code.contains("title: title"),
            "简写 {{title}} 应展开:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn attach_compiles() {
        let src = r#"<script>
let n = $state(0i32);
</script>
<view {@attach |d: &sv_ui::Doc, id: sv_ui::ViewId| { let _ = (d, id, n); }}></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("::sv_reactive::effect"),
            "{{@attach}} 应包进 effect:\n{code}"
        );
        assert!(code.contains("n.get()"), "附着闭包内读应改写:\n{code}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn class_directive_reactive() {
        let src = r#"<script>
let muted = $state(false);
let big = $state(true);
</script>
<view>
  <text class:muted class:big={big} style:padding={4.0f32}>字</text>
</view>

<style>
.muted { fg: #999999; }
.big { font-size: 30; }
</style>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("::sv_ui::bind_style"),
            "有条件类应整体重算:\n{code}"
        );
        assert!(
            code.contains("if muted.get()"),
            "简写条件即同名变量:\n{code}"
        );
        assert!(code.contains("if big.get()"), "\n{code}");
        assert!(
            code.contains("Edges::all(4.0f32)"),
            "style: 指令应并入同一闭包:\n{code}"
        );
        assert!(
            !code.contains("bind_style_patch"),
            "并入后不应再有独立 patch:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn transition_fade_compiles() {
        let src = r#"<view>
  <view transition:fade><text>a</text></view>
  <view in:fade={500u32}><text>b</text></view>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(code.contains("transition_in_fade"), "\n{code}");
        assert!(
            code.contains("200u32") && code.contains("500u32"),
            "\n{code}"
        );
        let err = compile("<view out:fade><text>x</text></view>", "c").unwrap_err();
        assert!(err.message.contains("INERT"), "{err}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn bind_checked_two_way() {
        let src = r#"<script>
let done = $state(false);
</script>
<view><checkbox bind:checked={done} /></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(code.contains("create_checkbox"), "\n{code}");
        assert!(
            code.contains("set_checked(__b_el, __b_sig.get())"),
            "状态→视图:\n{code}"
        );
        assert!(
            code.contains("__b_sig.update(|__v| *__v = !*__v)"),
            "点击→状态:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn await_block_compiles() {
        let src = r#"<script>
let base = $state(1i32);
</script>
<view>
  {#await async move { base + 1 }}
    <text>加载中</text>
  {:then v}
    <text>{v}</text>
  {/await}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(code.contains("::sv_ui::tasks::await_block"), "\n{code}");
        assert!(
            code.contains("base.get() + 1"),
            "future 工厂内读应改写(依赖变化即重启):\n{code}"
        );
        syn::parse_file(&code).unwrap();

        let src2 = r#"<view>
  {#await do_load()}
    <text>...</text>
  {:then v}
    <text>{v}</text>
  {:catch e}
    <text>{e}</text>
  {/await}
</view>
"#;
        let code2 = compile(src2, "c").expect("应编译成功");
        assert!(
            code2.contains("await_block_result"),
            "带 catch 走 Result 版:\n{code2}"
        );
        syn::parse_file(&code2).unwrap();
    }

    #[test]
    fn rune_variants_batch4() {
        let src = r#"<script>
let count = $state(1i32);
let snap = $state.snapshot(count);
let id = $props.id();
let tracking = $effect.tracking();
let stop = $effect.root(|| {
    let _ = count;
});
$inspect(count).with(|vals| {
    let _ = vals;
});
$effect(|| {
    $inspect.trace("主效应");
    let _ = count;
});
</script>
<view><text>{snap} {id}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("let snap = (count.get())"),
            "$state.snapshot:\n{code}"
        );
        assert!(
            code.contains("::sv_reactive::unique_id()"),
            "$props.id:\n{code}"
        );
        assert!(
            code.contains("::sv_reactive::is_tracking()"),
            "$effect.tracking:\n{code}"
        );
        assert!(
            code.contains("__sv_root.dispose()"),
            "$effect.root 返回销毁闭包:\n{code}"
        );
        assert!(code.contains("[trace]"), "$inspect.trace:\n{code}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn writable_derived_assignment() {
        let src = r#"<script>
let a = $state(1i32);
let d = $derived(a * 2);
let optimistic = || d = 99;
</script>
<view><text>{d}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("d.set(99)"),
            "写 derived 应改写(乐观 UI):\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn snippet_as_prop_auto_rc() {
        let mut registry = PropsRegistry::new();
        registry.insert(
            "card",
            PropsSig {
                fields: Some(vec![PropsSigField {
                    name: "body".into(),
                    has_default: false,
                    bindable: false,
                }]),
            },
        );
        let src = r#"<view>
  {#snippet hello()}
    <text>你好</text>
  {/snippet}
  <Card body={hello} />
</view>
"#;
        let code = compile_with(src, "app", &registry).expect("应编译成功");
        assert!(
            code.contains("as ::sv_ui::Snippet"),
            "snippet 名作 prop 应自动包 Rc:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn css_c1_box_model_vars_nesting() {
        let src = r##"<view>
  <button class="btn">按钮</button>
  <text>正文继承字号</text>
</view>

<style>
:root { --accent: hsl(16, 100%, 50%); --pad: 8px 16px; }
text { font-size: 1.25rem; color: #334; }
.btn {
  padding: var(--pad);
  margin: 4px 8px 12px 16px;
  border: 2px solid var(--accent, red);
  cursor: pointer;
  &:hover { opacity: 0.9; }
  &:active { background-color: hwb(16 10% 10%); }
}
</style>
"##;
        let code = compile(src, "c").expect("C1 语法应编译成功");
        // padding 简写 via var():8px 16px → 上下 8 左右 16
        assert!(
            code.contains("top: 8f32") && code.contains("right: 16f32"),
            "var() + 四值简写:\n{code}"
        );
        // margin 四值
        assert!(
            code.contains("bottom: 12f32") && code.contains("left: 16f32"),
            "margin 四值:\n{code}"
        );
        // border + hsl 折叠(hsl(16,100%,50%) ≈ rgb(255,68,0))
        assert!(code.contains("::sv_ui::Border"), "border 简写:\n{code}");
        assert!(
            code.contains("Color::rgba(255u8, 68u8, 0u8, 255u8)"),
            "hsl 编译期折叠:\n{code}"
        );
        // cursor
        assert!(code.contains("Cursor::Pointer"), "cursor:\n{code}");
        // 嵌套伪类:hover + active 双状态接线
        assert!(
            code.contains("__hv") && code.contains("__ac"),
            "嵌套 &:hover/&:active:\n{code}"
        );
        assert!(
            code.contains("set_on_pointer_down") && code.contains("set_on_pointer_up"),
            ":active 应接按压事件:\n{code}"
        );
        // 元素类型规则:rem 折叠(1.25rem=20)+ text 元素打底
        assert!(code.contains("s.font_size = 20f32"), "rem × 16:\n{code}");
        syn::parse_file(&code).unwrap();

        // 未定义变量报错
        let err = compile(
            "<view><text style=\"color: var(--nope)\">x</text></view>",
            "c",
        )
        .unwrap_err();
        assert!(err.message.contains("--nope"), "{err}");
    }

    #[test]
    fn css_compat_names_units_hover() {
        let src = r##"<view>
  <button class="btn">按钮</button>
</view>

<style>
.btn {
  background-color: rgb(255, 62, 0);
  color: white;
  border-radius: 6px;
  padding: 8px;
  flex-direction: row;
}
.btn:hover { background-color: orange; opacity: 0.9; }
</style>
"##;
        let code = compile(src, "c").expect("CSS 语法应编译成功");
        assert!(
            code.contains("s.corner_radius = 6f32"),
            "px 单位应剥离:\n{code}"
        );
        assert!(
            code.contains("Color::rgba(255u8, 62u8, 0u8, 255u8)"),
            "rgb() 应解析:\n{code}"
        );
        assert!(
            code.contains("Color::rgba(255u8, 165u8, 0u8, 255u8)"),
            "颜色名 orange:\n{code}"
        );
        assert!(code.contains("__hv"), ":hover 应生成内部悬停状态:\n{code}");
        assert!(
            code.contains("set_on_pointer_enter") && code.contains("set_on_pointer_leave"),
            ":hover 应自动接线指针事件:\n{code}"
        );
        assert!(
            code.contains("if __hv.get()"),
            ":hover 样式应条件生效:\n{code}"
        );
        syn::parse_file(&code).unwrap();

        // 不支持的单位给出引导
        let err = compile("<view><text style=\"padding: 2em\">x</text></view>", "c").unwrap_err();
        assert!(
            err.message.contains("em") && err.message.contains("px"),
            "{err}"
        );
    }

    // ---- 对抗审查回归测试(2026-07-17,docs 见审查 workflow)----

    #[test]
    fn props_in_comment_or_string_ignored() {
        let src = r##"<script>
// $props { ghost: i32 }
let msg = "$props { fake: i32 }";
let count = $state(0i32);
</script>
<view><text>{count} {msg.len()}</text></view>
"##;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            !code.contains("Props"),
            "注释/字符串里的 $props 不应生成结构体:\n{code}"
        );
        assert!(
            code.contains("$props { fake: i32 }"),
            "字符串内容不应被改动:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn rune_in_string_not_replaced() {
        let src = r#"<script>
let s = "$state 不是 rune";
let count = $state(1i32);
</script>
<view><text>{count}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("\"$state 不是 rune\""),
            "字符串里的 $state 应保留原文:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn plain_var_used_twice_gets_preclones() {
        let src = r#"<script>
$props { label: String }
</script>
<view>
  <text>{label}</text>
  <text>{label}</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        let clones = code.matches("Clone::clone(&label)").count();
        assert!(clones >= 2, "两处使用应各有预克隆(实际 {clones}):\n{code}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn rebuild_closure_preclones_plain_vars() {
        let src = r#"<script>
$props { label: String }
let show = $state(true);
</script>
<view>
  {#if show}
    <text>{label}</text>
  {/if}
  <text>{label}</text>
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        // if 的重建闭包体内应有每次调用的预克隆(否则 Fn 闭包被移出 → E0507)
        let if_part = code.split("if_block").nth(1).expect("应有 if_block");
        assert!(
            if_part.contains("Clone::clone(&label)"),
            "重建闭包体内应预克隆普通变量:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn fn_match_for_patterns_shadow() {
        let src = r#"<script>
let count = $state(0i32);
fn double(count: i32) -> i32 { count * 2 }
let m = match Some(5i32) { Some(count) => count + 1, None => 0 };
$effect(|| {
    for count in 0..3 { let _ = count; }
});
</script>
<view><text>{count}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("count * 2") && !code.contains("count.get() * 2"),
            "fn 参数应遮蔽:\n{code}"
        );
        assert!(
            code.contains("Some(count) => count + 1"),
            "match 模式应遮蔽:\n{code}"
        );
        assert!(
            !code.contains("for count in 0..3 { let _ = count.get()"),
            "for 模式应遮蔽:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn if_let_pattern_shadows() {
        let src = r#"<script>
let count = $state(0i32);
let opt = Some(1i32);
let v = if let Some(count) = opt { count + 1 } else { count };
</script>
<view><text>{v}</text></view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        assert!(
            code.contains("{ count + 1 }"),
            "if let 模式应遮蔽 then 分支:\n{code}"
        );
        assert!(
            code.contains("else { count.get() }"),
            "else 分支仍应改写:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn reactive_in_unknown_macro_is_hard_error() {
        let src = r#"<script>
let count = $state(0i32);
let hit = matches!(count, 1i32);
</script>
<view><text>x</text></view>
"#;
        let err = compile(src, "c").unwrap_err();
        assert!(
            err.message.contains("matches") && err.message.contains("count"),
            "非白名单宏里的反应式变量应硬错误引导:{err}"
        );
    }

    #[test]
    fn utf8_each_header_no_panic() {
        let src = r#"<script>
let 数据 = $state(vec![1i32]);
</script>
<view>
  {#each 数据 as 项, 序
  }
    <text>{项} {序}</text>
  {/each}
</view>
"#;
        // 非 ASCII 标识符 + 换行分隔的 as:不 panic,正常编译
        let code = compile(src, "c").expect("UTF-8 头部应能解析");
        assert!(code.contains("each_block"));
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn glued_block_keyword_not_misparsed() {
        let err = compile("<view>{#iffy}</view>", "c").unwrap_err();
        assert!(
            err.message.contains("未知块类型"),
            "{{#iffy}} 不应被当成 {{#if fy}}: {err}"
        );
    }

    #[test]
    fn char_literal_brace_in_expr() {
        let src = r#"<script>
let ch = $state('x');
</script>
<view>
  <text>{if ch == '}' { "右括号" } else { "其它" }}</text>
</view>
"#;
        let code = compile(src, "c").expect("字符字面量 '}}' 不应干扰配平");
        assert!(code.contains("'}'"), "\n{code}");
        syn::parse_file(&code).unwrap();
    }

    #[test]
    fn empty_props_decl_keeps_contract() {
        // 声明了空 $props:callee 带 props 参数,caller 也要传空结构体
        let callee = compile(
            "<script>\n$props {}\n</script>\n<view><text>x</text></view>",
            "empty_comp",
        )
        .expect("空 $props 应编译成功");
        assert!(
            callee.contains("props: EmptyCompProps"),
            "callee 应带 props 参数:\n{callee}"
        );

        let mut registry = PropsRegistry::new();
        registry.insert(
            "empty_comp",
            PropsSig {
                fields: Some(vec![]),
            },
        );
        let caller = compile_with("<view><EmptyComp /></view>", "app", &registry).unwrap();
        assert!(
            caller.contains("empty_comp(&__doc, __parent, EmptyCompProps {})"),
            "caller 应传空结构体:\n{caller}"
        );

        // 未声明 $props 的组件:不带 props 参数
        let mut registry2 = PropsRegistry::new();
        registry2.insert("bare", PropsSig { fields: None });
        let caller2 = compile_with("<view><Bare /></view>", "app", &registry2).unwrap();
        assert!(caller2.contains("bare(&__doc, __parent);"), "\n{caller2}");
    }

    #[test]
    fn shadowed_each_pattern_not_rewritten() {
        let src = r#"<script>
let count = $state(0i32);
let rows = $derived(vec![count; 3]);
</script>
<view>
  {#each rows as count}
    <text>{count}</text>
  {/each}
</view>
"#;
        let code = compile(src, "c").expect("应编译成功");
        // each 的行内 count 是模式绑定的普通值,不能被改写成 count.get()
        let row_part = code.split("each_block").nth(1).unwrap();
        let after_bind = row_part.split("bind_text").nth(1).unwrap();
        assert!(
            !after_bind.contains("count.get()"),
            "被 pattern 遮蔽的名字不应改写:\n{code}"
        );
        syn::parse_file(&code).unwrap();
    }

    /// **编译器不 panic 的差分 fuzz** —— 与 sv-vap/sv-pag 同款纪律,补上调研 19
    /// 点名的"编译器/解析器无 fuzz"缺口。
    ///
    /// `compile` 的契约是"畸形输入 → `Err(CompileError)`,**绝不 panic**":
    /// `.sv` 是构建期跑的,一个坏文件应当给出可读的编译错误,而不是 `unwrap`
    /// 崩掉 build.rs(那对用户是一句没有上下文的 `thread panicked at ...`)。
    /// 但解析器里有几十处 `unwrap`/切片,没有测试守住这条契约。
    ///
    /// 这里不写用例,喂**语料**:合法样本从每个字节切一刀(截断在标签中间 /
    /// 表达式中间 / `{#if}` 与 `{/if}` 之间 / `<script>` 与 `</script>` 之间),
    /// 外加一批手写的对抗输入(不配对的括号/标签/块、深嵌套、孤立标点)。
    /// 全部只准返回 `Ok`/`Err`,`catch_unwind` 逮到任何 panic 就报出是哪条输入。
    #[test]
    fn compile_never_panics_on_malformed_input() {
        // 一个用到多数语法面的合法样本:script/插值/if/each/事件/输入/style
        const RICH: &str = r##"<script>
let count = $state(0i32);
let items = $state(vec![String::from("甲")]);
let note = $state(String::new());
let double = $derived(count * 2);
</script>
<view style="direction:column; gap:8; padding:12">
  <text>计数 {count} · 翻倍 {double}</text>
  <button onclick={|| count += 1}>加</button>
  <input bind:value={note} placeholder="备注" />
  {#if count > 3}
    <text fg="#f00">超过三</text>
  {:else}
    <text>还早</text>
  {/if}
  {#each items as it, i (i)}
    <text>{i}: {it}</text>
  {/each}
</view>
"##;

        let mut corpus: Vec<String> = Vec::new();
        // 1) 合法样本从每个字节切一刀(截断)——注意按**字符边界**切,避免制造
        //    非法 UTF-8(那考的是别的东西);utf8 边界用 char_indices
        for (cut, _) in RICH.char_indices() {
            corpus.push(RICH[..cut].to_string());
        }
        corpus.push(RICH.to_string());
        // 2) 手写对抗输入:不配对 / 孤立 / 深嵌套 / 空
        let adversarial = [
            "",
            "<",
            "<view",
            "<view>",
            "</view>",
            "<view></nope>",
            "<view>{",
            "<view>{count",
            "<view>{#if}</view>",
            "<view>{#if x}{/each}</view>",
            "<view>{#each}</view>",
            "<script>let x = $state(",
            "<script></script><view>{#if}",
            "{@const x = }",
            "<button onclick={>",
            "<input rows=\"\" />",
            "<view style=\"",
            "<view style=\"padding:\">x</view>",
            "<animation src=",
            "<Comp",
            "<自定义>",
            "{{{{{{{{{{",
            "}}}}}}}}}}",
            "<view><view><view><view><view><view>",
            "<script>\u{0}\u{0}</script>",
        ];
        corpus.extend(adversarial.iter().map(|s| s.to_string()));

        let mut panicked: Vec<String> = Vec::new();
        for input in &corpus {
            let inp = input.clone();
            // compile 只吃 &str(UnwindSafe);catch_unwind 逮住任何 panic
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = compile(&inp, "c");
            }));
            if r.is_err() {
                panicked.push(input.chars().take(60).collect());
            }
        }
        assert!(
            panicked.is_empty(),
            "compile 对以下畸形输入 panic 了(应返回 Err 而非崩):\n{}",
            panicked.join("\n")
        );
    }
}
