**中文** | [English](../en/performance.md)

# 性能

> 状态:探索原型。本页所有数字均为 2026 年 7 月在同一台开发机上实测(Windows 11、DX12、release 构建、1920×1080 离屏),完整记录见[调研 18](../research/18-million-controls-144fps.md)。API 仍在快速演进,请把本页当作模型快照,而非基准承诺。

sv 的性能路线靠架构而非常数调优:模板编译为对保留场景树的定点更新,每帧成本只随**变化量**走,与**界面规模**无关。没有虚拟 DOM,运行时没有 diff。

## 性能模型

数据流:`state`/`derived` → effect 精准修改场景树 → 版本号 bump → `on_mutate` → 重绘。在这条链路之上,靠以下机制压低帧成本:

| 机制 | 位置 | 作用 |
|---|---|---|
| 定点更新 | sv-reactive + sv-ui 绑定原语(`bind_text` 等) | 一次信号写入只修补绑定到它的节点,从不做全树 diff |
| 版本键控布局缓存 | `sv_shell::render::layout_tree_cached(doc, logical_w, logical_h)` | 按(doc 身份、doc 版本、尺寸)缓存布局结果;静止帧的 measure/place 成本归零 |
| 静止帧短路 | sv-shell 窗口循环 | (版本、尺寸、缩放)与上一帧相同且无动画时整帧跳过——静止功耗归零 |
| vello 场景编码跳过 | vello 后端的 `render_cached(doc, scale, scene_unchanged)` | 静止帧只重呈现 surface,不重编码 vello 场景 |
| 字形覆盖度缓存 + 分代淘汰 | sv-shell 绘制层 | swash alpha 覆盖度位图分两代缓存(hot/cold,各 2048 条)。热代满则整代降冷(旧冷代丢弃),冷代命中零成本晋升回热代。单帧最多重光栅"整代未用"的字形——不像整体清空那样把活跃工作集也打掉,不会产生伤 1% low 的帧时长尖峰。内存上界 2 × 2048 条 ≈ 2.6 MB@16px |

一个注意点:布局缓存是全有全无的——任何版本 bump 都会全树重布局(局部布局在计划中,见下文)。虚拟化下活跃树很小,目前无感。

## 虚拟化:`sv_ui::virtual_list`

全量建树的帧成本随树规模线性增长(见下表),规模化的答案只能是削减每帧工作集。原语定义在 `crates/sv-ui/src/lib.rs`:

```rust
pub fn virtual_list<T: Clone + 'static>(
    doc: &Doc,
    parent: ViewId,
    count: impl Fn() -> usize + 'static,
    offset: sv_reactive::Signal<usize>,
    viewport_rows: usize,
    item_at: impl Fn(usize) -> T + 'static,
    row: impl Fn(&Doc, ViewId, sv_reactive::Signal<Option<T>>, usize) + 'static,
)
```

工作方式:

- **固定槽位** — `viewport_rows` 个真实行只建一次;每槽持一个 `Signal<Option<T>>`,行构建器绑定到它上面。
- **滚动 = 数据置换** — `offset`(或 `count`)变化时,一个 effect 逐槽 `.set()`,行内绑定原地更新。零节点创建/销毁、零结构变化——这正是 1% low 稳定的来源(没有"滚动帧偶发重建"的长尾)。
- **懒取数** — `item_at(i)` 只对可见下标调用,百万行列表永不物化整表。
- 槽值为 `None` 表示越界空槽,行内自行渲染空态。

单测 `virtual_list_million_rows_few_nodes` 把这些钉死:100 万逻辑行 + 30 槽视口,场景树至多 34 个节点;`offset.set(500_000)` 原地更新,节点数一个不增不减。

```rust
use sv_reactive::{create_root, state};
use sv_ui::{Doc, bind_text, virtual_list};

let doc = Doc::new();
let offset = state(0usize);
let (_, _scope) = create_root(|| {
    virtual_list(
        &doc,
        doc.root(),
        || 1_000_000,            // 逻辑行数(闭包,可以是响应式的)
        offset,                  // 滚动位,单位是行
        30,                      // 视口槽位:真实存在的行只有这些
        |i| format!("第 {i} 行"), // 懒取数,只对可见下标调用
        |doc, parent, slot, _i| {
            let label = doc.create_text("");
            doc.append(parent, label);
            bind_text(doc, label, move || slot.get().unwrap_or_default());
        },
    );
});

offset.set(500_000); // 滚动:30 次槽写入,零节点创建/销毁
```

(在 `sv_shell::run_app` 里,构建闭包本身已运行在 root 作用域中;显式 `create_root` 只在独立使用时需要。)

**何时该用**:几千行以上的列表。全量建树在 3000 控件档 CPU 后端就已花掉 ≈7.2 ms/帧——正好吃光 144 Hz 预算;到 10000 连 60 Hz 都保不住。几千以下,全量建树完全够用。

## 百万控件实测

工况(membench,调研 18):20 万逻辑行 × 每行 5 控件(行容器 + 复选框 + 两个文本 + 按钮)= 100 万逻辑控件;视口 30 行;`--mutate` **每帧**滚动一行——连续滚动,虚拟化的最坏工况。

| 口径 | 帧均 | p99 | **1% low** | WorkingSet |
|---|---|---|---|---|
| CPU 离屏 1920×1080,500 帧 | 3.41 ms | **5.28 ms** | **174 fps** | 28.2 MB |
| CPU 窗口(softbuffer 无 vsync 封顶) | — | — | ~800 fps(FPS 计数) | 37.9 MB |
| vello 离屏(≈7 ms 回读地板) | 9.26 ms | 16.3 ms | 56 fps | 633 MB |
| vello 窗口 | — | — | 60 fps(vsync 钉死) | 598 MB |

144 Hz 的帧预算是 6.94 ms。CPU 口径 p99 = 5.28 ms、1% low = 174 fps 达标,且 28 MB 工作集不随逻辑控件数增长。两行窗口口径量的是呈现而非能力:softbuffer 没有 vsync 封顶(所以 ~800 fps),而 vello surface 走 AutoVsync 被钉在 60——突破它需要 mailbox/immediate 呈现模式(ADR-6,未实现)。vello 离屏还背着窗口路径没有的 ≈7 ms 纹理回读地板,扣除后 ≈2.3 ms/帧。

## 为什么要虚拟化:全量建树的成本

同一行构成、不开虚拟化,CPU 后端(tiny-skia + swash):

| 控件数 | 帧均 | p99 | 1% low | WorkingSet |
|---|---|---|---|---|
| 0 | 2.2 ms | 3.6 ms | 275 fps | 27.4 MB |
| 1 000 | 5.0 ms | 6.1 ms | 163 fps | 28.4 MB |
| 3 000 | 7.2 ms | 9.1 ms | 110 fps | 29.7 MB |
| 10 000 | 17.5 ms | 20.5 ms | 49 fps | 32.2 MB |
| 30 000 | 46.7 ms | 51.9 ms | 19 fps | 40.2 MB |
| 100 000 | 150.8 ms | 154.8 ms | 6 fps | 68.8 MB |

帧成本随树规模线性增长,因为每帧都在全量重布局、全量重编码。常数优化只能平移曲线、改不了斜率——swash 迁移把 CPU 光栅路径提速约一倍,100k 控件仍要 150.8 ms。这个基准里 vello 全量建树只会更差(3000 档离屏 ≈23.2 ms;100 000 档 ≈452.5 ms——慢而正确,抬高 wgpu 存储缓冲上限后不再崩溃);GPU 后端真正划算的场景见[渲染后端](./rendering-backends.md)。

## 内存经验值

| 项目 | 成本 |
|---|---|
| UI 侧(场景树 + 信号) | ≈0.5 KB/控件;10 万控件全量建树也只比基线多 ≈41 MB |
| 文本栈基线(swash) | `--controls 0` 时 WorkingSet ≈27 MB(Private 20.5 MB,首帧 warmup 11 ms);fontdue 时代是 ~200 MB |
| vello 设备成本 | 开发机上 ≈600 MB 的 wgpu/DX12 管线固定成本,与内容无关 |
| 字形覆盖度缓存 | ≤ 2 代 × 2048 条 ≈ 2.6 MB@16px |

## membench 基准测试台

`examples/membench` 构建 N 个控件的典型混合界面,可选渲染若干计时离屏帧,打印一行统计后驻留,供外部工具(如 PowerShell)采样进程 WorkingSet/Private。

```sh
# 100 万逻辑控件,虚拟化 + 连续滚动,500 计时帧
cargo run -p membench --release -- --controls 1000000 --virtual --mutate --frames 500

# vello 后端(需要 feature)
cargo run -p membench --release --features backend-vello -- --controls 3000 --backend vello

# 窗口冒烟 + 实时 FPS 计数
SV_SHOW_FPS=1 cargo run -p membench --release -- --controls 1000000 --virtual --mutate --windowed
```

| 参数 | 默认 | 含义 |
|---|---|---|
| `--controls N` | 3000 | 逻辑控件总数;行数 = N / 5(每行 view + 复选框 + 2 文本 + 按钮) |
| `--backend cpu\|vello` | `cpu` | 离屏渲染后端;`vello` 需要 `--features backend-vello` |
| `--frames N` | 3 | 计时帧数,在 1 帧不计时的预热帧(字体解析、管线编译)之后测 |
| `--no-render` | 关 | 只建树不渲染——测构建耗时与内存用 |
| `--mutate` | 关 | 每帧一次突变:普通模式改一行信号,`--virtual` 模式滚动一行 |
| `--virtual` | 关 | 用 `virtual_list`(30 行视口)代替全量建树 |
| `--windowed` | 关 | 开真实窗口(真实呈现路径);配 `SV_SHOW_FPS=1` 与 `SV_RENDERER=cpu\|vello` 用 |
| `--hold-secs N` | 6 | 打印 `READY` 后驻留,供外部采样内存 |

输出为单行(随后进程驻留):

```
READY backend=<cpu|vello> mutate=<bool> virtual=<bool> nodes=<n> signals=<n> build_ms=<n> warmup_ms=<n> frame_avg_ms=<f> p99_ms=<f> low1_fps=<f> fps=<f> frames=<n>
```

| 字段 | 含义 |
|---|---|
| `nodes` | 场景树节点数 |
| `signals` | 存活的响应式图节点数(`sv_reactive::debug_node_count()`) |
| `build_ms` | 建树耗时 |
| `warmup_ms` | 不计时的首帧(含字体解析/管线编译) |
| `frame_avg_ms` / `fps` | 计时帧均值及其换算帧率 |
| `p99_ms` | 帧时长 99 分位 |
| `low1_fps` | 最差 1% 帧的均值换算帧率——144 Hz 目标的验收口径 |

注意:vello 离屏计时含纹理回读(≈7 ms),相对窗口路径略高估帧成本。

## 计划中,尚未实现

以下均不存在于当前代码,路线图与 ADR 见 [DESIGN.md](../DESIGN.md):

- **帧调度(ADR-6)** — vsync 对齐的批量 flush + mailbox/immediate 呈现模式选项。目前写入即同步 flush;正是缺这个呈现模式选项,vello 窗口才被钉在 60 fps。
- **增量场景编码** — RecordingPainter 命令流 diff,让全量建树档的帧成本也降下来。
- **局部布局** — dirty 子树 relayout。目前任何版本 bump 都全树重布局,缓存只对完全静止的帧有效。
- **滚动物理** — 惯性、像素级 offset,落在 `virtual_list` 之上。

## 相关阅读

- [渲染后端](./rendering-backends.md) — CPU 与 vello 对比、后端选择、`SV_RENDERER`。
- [调研 18:百万控件 @ 144fps](../research/18-million-controls-144fps.md) — 本页数字背后的完整测量记录。
- [DESIGN.md](../DESIGN.md) — ADR-9(规模策略)与 ADR-6(帧调度)。
