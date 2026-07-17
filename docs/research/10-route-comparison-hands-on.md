# 10 · 双路线动手实证:view! proc-macro vs .sv 独立编译器(2026-07-17)

> 本仓库把两条路线**都做成了可运行原型**,同一个计数器、同一套运行时(sv-reactive +
> sv-ui + sv-shell),可直接并排对比。本文记录实证数据;联网调研见 06–09 号报告;
> 最终判决见 DESIGN.md ADR-2(修订版)。

## 1. 两个原型

| | proc-macro 路线 | 编译器路线 |
|---|---|---|
| 实现 | `crates/sv-macro`(view! 宏,parse/ir/codegen 分层,12 测试) | `crates/sv-compiler`(.sv SFC 编译器,sfc/script/template/style/codegen,7 测试 + 1 端到端行为测试) |
| 示例 | `examples/counter`(模板内嵌 main.rs) | `examples/counter-sfc`(src/Counter.sv + build.rs → OUT_DIR + include!) |
| 状态 | 全绿,窗口/离屏渲染均通过 | 全绿,窗口/离屏渲染均通过 |

## 2. 同一个计数器的两种写法

**view! 宏**(模板受 Rust token 约束:文本要引号、事件是 `on_click(...)` 属性、
控制流用 Rust `if/for` 形态、读写全显式):

```rust
view! { doc, root =>
    <text style(|s| s.font_size = 20.0)>"Count: " {count.get()}</text>
    <button style(btn) on_click(move || count.update(|c| *c += 1))>"+1"</button>
    if count.get() > 5 {
        <text style(|s| s.fg = Some(Color::rgb(255, 62, 0)))>"超过 5 了!"</text>
    }
}
```

**.sv 文件**(不受 Rust tokenizer 约束:免引号文本、原汁 `{#if}{:else if}`、
`on:click`、字符串样式语言,以及 **runes 隐式反应性**):

```text
<script>
let count = $state(0i32);
let double = $derived(count * 2);
$effect(|| println!("count 变为 {}", count));
</script>

<text font-size="20">Count: {count} · 双倍 = {double}</text>
<button style="bg:#ff3e00; fg:#fff" on:click={|| count += 1}>+1</button>
{#if count > 5}
  <text fg="#ff3e00">超过 5 了!</text>
{/if}
```

编译器对 script 块做了 proc-macro 拿不到整个作用域、做不了的源变换(实测生效):

| 用户写 | 编译产物 |
|---|---|
| `let count = $state(0i32)` | `let count = ::sv_reactive::state(0i32);` |
| `let double = $derived(count * 2)` | `derived(move \|\| count.get() * 2)`(裸读自动 `.get()` + 自动闭包化) |
| `count += 1` | `count.update(\|__v\| *__v += 1)` |
| `count = 0` | `count.set(0)` |
| `println!("{}", count)` | `println!("{}", count.get())`(常见宏参数也改写) |
| 引用反应式变量的闭包 | 自动注入 `move`(句柄 Copy,零成本) |
| `{#each rows as count}` 行内 | pattern 遮蔽正确处理,行内 `count` **不**被改写 |

生成代码经 prettyplease 格式化,完全可读(见 OUT_DIR/counter.rs,144 行,
与手写的"编译产物形态"一致)。

## 3. 决定性实验:同一个类型错误的落点

在插值里制造 `i32 + &str` 类型错误:

**proc-macro 路线** —— 错误精确指向用户源码的出错字符,原行原样渲染:

```text
error[E0277]: cannot add `&str` to `i32`
  --> examples\counter\src\main.rs:27:68
   |
27 |  <text style(...)>"Count: " {count.get() + "类型错误"}</text>
   |                                           ^ no implementation for `i32 + &str`
```

**编译器路线** —— 错误落在 OUT_DIR 生成文件,用户需自行映射回 .sv:

```text
error[E0277]: cannot add `&str` to `i32`
  --> C:/cargo-target/.../out/counter.rs:39:44
   |
39 |  __s.push_str(&(count.get() + "类型错误").to_string());
```

这是编译器路线的**结构性代价**:rustc 没有 `#line` 机制,span 无法指回 .sv。
生成代码可读 + 头部注释指回源文件只能止血;根治需要 svelte-check 式诊断映射器
(cargo check JSON 诊断 → sourcemap → .sv 位置)与 Volar 式 LSP(见 07 号报告)。
补充:sv-compiler 自身的**模板/script 语法错误**已带 `.sv 文件:行:列` 精确定位
(实测:未闭合 `{#if}` 报在 if 行),问题只出在"穿透到 rustc 的类型错误"这一层。

## 4. 能力差异小结(实证部分)

| 维度 | proc-macro | 编译器 | 备注 |
|---|---|---|---|
| rustc 错误落点 | ✅ 用户源码精确 span | ❌ OUT_DIR 生成文件 | 上面实验 |
| 模板语法自由度 | 受 token 约束(文本必须引号等) | ✅ 100% Svelte 语法 | |
| runes 隐式反应性 | ❌ 只能显式 `.get()/.update()` | ✅ 全 script 源变换 | 核心 DX 差异 |
| rust-analyzer 补全 | 模板内 Rust 表达式可用(宏展开) | ❌ .sv 内无,需自建 LSP | |
| 语法错误定位 | ✅ span 保真 | ✅ 自带 行:列(编译器自己报) | 打平 |
| 构建集成 | 零配置 | build.rs 三行 + include! 一行 | 均可接受 |
| 增量构建 | 每次重编译宏调用所在 crate | rerun-if-changed 精确到 .sv 文件 | 本项目规模下无感差异,规模化测量待 M1 |
| 热重载潜力 | 需 Dioxus 式宏内容 diff | ✅ 模板数据化后天然适配 | 见 09 号报告 |
| 实现成本(实测) | ~700 行(含测试) | ~1100 行(含测试) | 同一人一天内各自完成,都不贵 |

## 5. 关键架构事实

两条路线**共享同一个编译目标**(sv-ui 绑定原语)和同一套运行时,生成代码形态
几乎相同——证明了"编译器核心独立、多前端"的架构是成立的:proc-macro 和 .sv
编译器可以长期共存,共享 IR 与 codegen(当前两者是独立实现,合并 IR 是 M1 工作)。
