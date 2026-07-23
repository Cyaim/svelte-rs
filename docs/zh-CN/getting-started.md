**中文** | [English](../en/getting-started.md)

# 快速上手

svelte-rs(工作代号 `sv`)是一个 Svelte 风格 Rust 桌面 UI 库的**探索原型**。核心思路是把 Svelte 5 的编译哲学搬到原生桌面:模板在编译期变成对 retained 场景树的定点更新代码——运行时没有虚拟 DOM,没有 diff。Windows / Linux / macOS 现在就能跑(基于 winit 的窗口壳),鸿蒙(HarmonyOS NEXT)在路线图上(见 [DESIGN.md](../DESIGN.md))。现阶段 API 随时会变,没有任何稳定性承诺。

四行代码看完整个响应式模型:

```rust
let count = state(0);                          // $state
let double = derived(move || count.get() * 2); // $derived
effect(move || println!("{}", double.get()));  // $effect
count.set(1);                                  // 精准触发,无 diff
```

## 前置条件

- **Rust stable。**工作区声明 `edition = "2024"`(见 `Cargo.toml`),没有 `rust-toolchain` 版本钉死,较新的 stable 工具链即可。
- 一个桌面环境。原型壳是纯 CPU 渲染(winit + softbuffer + tiny-skia + swash),不需要 GPU。可选的 GPU 后端藏在 `backend-vello` cargo feature 后面(见下文)。

## 克隆与测试

```sh
git clone https://github.com/Cyaim/svelte-rs
cd svelte-rs
cargo test          # 跑全部工作区测试
```

> **检出目录在 OneDrive / Dropbox 等同步盘里?**把 `.cargo/config.toml` 里注释掉的 `target-dir` 打开并按机器调整路径。同步器会锁构建产物(Windows 上表现为链接失败、增量构建损坏),`target/` 还会产生数 GB 无意义同步流量:
>
> ```toml
> [build]
> # target-dir = "C:/cargo-target/svelte-rs"
> ```

dev profile 为 UI 迭代做了调优:工作区代码 `opt-level = 1`、依赖 `opt-level = 2`,debug 模式跑 demo 不至于太卡,又不用付出 release 的编译时间。

## 运行示例

```sh
cargo run -p showcase       # 特性橱窗 —— 从这里开始
cargo run -p counter        # 计数器,view! 宏路线
cargo run -p counter-sfc    # 计数器,.svelte 单文件组件路线
cargo run -p todo-sfc       # 待办,更大的 .svelte 特性面
```

| 示例 | 前端 | 演示内容 |
|---|---|---|
| `showcase` | `.svelte` 编译器 | **推荐先看**:`$bindable` 双向绑定、children snippet、`{#snippet}/{@render}`、keyed `{#each}`(重排保状态)、scoped `<style>`、`{#await}`、`in:fade` |
| `counter` | `view!` 宏 | 最小计数器,直接用 proc-macro 模板写在 Rust 里 |
| `counter-sfc` | `.svelte` 编译器 | 同一个计数器,UI 放在 `src/Counter.svelte`,由 build.rs 编译成 `$OUT_DIR` 里人类可读的 Rust |
| `todo-sfc` | `.svelte` 编译器 | 组件 + `$props`、`{#each}{:else}`、`{@const}`、`{#key}`、`style:` 指令、`$inspect` |
| `membench` | 直接调 `sv-ui` API | 内存 / 帧时间基准测试台(不支持 `--png`,自带 CLI,见下文) |

窗口以 480×400 逻辑尺寸打开,HiDPI 自适应。两个计数器示例用两条模板前端搭出同一个 UI——对照读它们的源码,能快速体会两条路线的手感。

`.svelte` 路线长这样(script 里的 `count += 1` 会被编译器自动改写成句柄操作):

```text
<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<text>Count: {count} · 双倍 = {double}</text>
<button style="bg:#ff3e00; fg:#fff" onclick={|| count += 1}>+1</button>
```

## 离屏渲染:`--png`

四个开窗示例都支持 `--png [路径]`——不开窗渲染一帧存成 PNG,适合 CI 和快速目检:

```sh
cargo run -p showcase -- --png out.png
cargo run -p counter -- --png out.png
```

省略路径时默认文件名分别是 `showcase.png`、`counter.png`、`counter-sfc.png`、`todo.png`。`showcase` 和 `todo-sfc` 会在截图前先模拟几次点击(步进器 +2、勾选一行、反转列表),让 PNG 呈现非空状态。`membench` **不**支持 `--png`。

## 基准测试台:`membench`

`membench` 构建 N 个控件的合成界面(每行 = view 容器 + checkbox + 两个 text + button,5 节点一行,行内含响应式绑定),可选渲染若干离屏帧,打印一行统计后驻留,供外部工具采样进程内存:

```sh
cargo run -p membench -- --controls 3000 --frames 3
cargo run --release -p membench --features backend-vello -- --backend vello
cargo run -p membench -- --windowed --mutate      # 真窗口模式,配合 SV_SHOW_FPS=1
```

| 参数 | 默认值 | 含义 |
|---|---|---|
| `--controls N` | 3000 | 控件总数(每行 5 个) |
| `--frames N` | 3 | 预热 1 帧后的计时帧数 |
| `--hold-secs N` | 6 | 打印后驻留秒数,供外部采样内存 |
| `--backend cpu\|vello` | `cpu` | 离屏渲染后端(`vello` 需要 `backend-vello` feature) |
| `--no-render` | 关 | 只建树,不渲染 |
| `--mutate` | 关 | 帧间驱动增量更新 |
| `--virtual` | 关 | 虚拟列表工况:N 逻辑行只实例化 30 行 |
| `--windowed` | 关 | 开真窗口而不是离屏 |

输出是一行机器可读的统计:`READY backend=… mutate=… virtual=… nodes=… signals=… build_ms=… warmup_ms=… frame_avg_ms=… p99_ms=… low1_fps=… fps=… frames=…`。实测数据见 [research/16-memory-benchmarks.md](../research/16-memory-benchmarks.md) 与 [research/17-backend-memory-fps.md](../research/17-backend-memory-fps.md)。

## 工作区结构

| 路径 | 职责 |
|---|---|
| `crates/sv-reactive` | runes 响应式内核:push-pull 三态脏标记、effect 所有权树 |
| `crates/sv-ui` | retained 场景树(桌面版 DOM)+ 细粒度绑定原语 |
| `crates/sv-macro` | `view!` proc-macro 模板前端(只剩 parser,模板 IR 与 codegen 共享 sv-compiler 内核) |
| `crates/sv-compiler` | `.svelte` 单文件组件编译器 + 双前端共享的模板 IR/codegen 内核(runes 源变换 + Svelte 模板语法) |
| `crates/sv-shell` | winit 窗口 + CPU 光栅壳;可选 vello/wgpu 后端在 `backend-vello` feature 后面 |
| `examples/*` | 上面列出的五个示例 |

数据流:`state`/`derived`(sv-reactive)→ effect 精准改场景树(sv-ui)→ 版本号 bump → `on_mutate` → 重绘(sv-shell)。细节见[架构](./architecture.md)。

## 一分钟尝鲜 GPU 后端

默认渲染器是 CPU 栈。vello/wgpu 后端由 `backend-vello` feature 编译进来——示例中只有 `showcase` 和 `membench` 转发了这个 feature:

```sh
cargo run -p showcase --features backend-vello
```

启动时的后端选择逻辑(`SV_RENDERER` 环境变量):

| `SV_RENDERER` | 行为 |
|---|---|
| 未设置 | 开了 feature:探测 GPU adapter,有则用 vello,否则打警告回退 CPU;没开 feature:CPU |
| `cpu` | 强制 CPU 后端 |
| `vello` | 不预探测直接用 vello;建 surface 失败再回退 CPU(feature 没编译进来时打警告用 CPU) |

`SV_SHOW_FPS=1` 会关掉静止帧短路、连续重绘,并每 30 帧打印一次 `FPS …`,用于诊断和基准测试:

```powershell
# PowerShell
$env:SV_RENDERER = "vello"; $env:SV_SHOW_FPS = "1"
cargo run -p showcase --features backend-vello
```

```sh
# bash
SV_RENDERER=vello SV_SHOW_FPS=1 cargo run -p showcase --features backend-vello
```

后端架构与回退链详见[渲染后端](./rendering-backends.md)。

## 接下来读什么

- [架构](./architecture.md) —— crate 分层与无 VDOM 数据流
- [响应式](./reactivity.md) —— `state` / `derived` / `effect` 与单线程运行时规则
- [.svelte 组件](./sv-components.md) —— 单文件组件格式与 build.rs 集成
- [渲染后端](./rendering-backends.md) —— CPU 栈、vello 与迁移计划
- [DESIGN.md](../DESIGN.md) —— ADR 决策记录、路线图、风险清单
- [SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) —— 77 项 Svelte 5 语法支持矩阵
- [CSS-SUPPORT.md](../CSS-SUPPORT.md) —— 91 项现代 CSS 差距矩阵
