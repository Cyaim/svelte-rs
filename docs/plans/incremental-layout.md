# 增量布局落地方案(原路线图条目"低层 trait 增量布局")

> 状态:方案待裁决。作者实测于 2026-07-22,代码基线 `1a07b06`。
> 本文回答 DESIGN.md §5 R2 留下的那个问号:**30k 全量档越了 2ms 触发线,
> 要不要上增量布局、上哪一种。**

## 0. 结论先行

**三条裁决,按确定性排序:**

1. **原条目的名字是错的,方向也偏了。** "低层 trait 增量布局"暗示增量能力来自
   实现 taffy 的 `LayoutPartialTree` 等低层 trait —— 不是。增量的机制是 taffy 的
   **每节点 9 槽缓存 + `mark_dirty` 沿父链清缓存**,高层 `TaffyTree` 已经全都有
   (`taffy-0.12.2/src/tree/cache.rs:11`、`tree/taffy_tree.rs:873`)。我们现在拿不到
   增量,唯一原因是**每个变更帧把整棵 `TaffyTree` 扔掉重建**
   (`crates/sv-shell/src/render.rs:487` `layout_tree_full`)。
   建议把条目改名为 **"增量布局(持久 TaffyTree + 变更分级)"**。

2. **瓶颈不在树重建,在 measure。** 实测 30k 树一次全量布局 ≈ 311ms,其中
   建树 17ms、walk 3ms、**compute 292ms —— 而 compute 里 40378 次
   parley 重排是主因**(`text::measure` 的两代缓存 `CAP = 4096`
   装不下 18004 个活跃键,`crates/sv-shell/src/text.rs:160`)。
   把 CAP 提到 32768 这一个常数,同一棵树掉到 **69ms**;再加叶内 memo 掉到 **61ms**。
   **1 人周不到拿 4.5–5 倍,这是全案性价比最高的一段,且与增量布局正交。**

3. **持久 TaffyTree 值得做,低层 trait 不值得。** 实测持久树在"什么都没脏"时
   重算耗时 **0.001ms**(整棵树一次根缓存命中),改一个叶子文本 **1.9ms compute**;
   稳态 30 帧连测,增量帧 compute 1.94ms + walk 2.51ms = **4.45ms**,
   对照同一棵树的现状全量帧 **72.4ms**(已含 CAP 修复)/ **311ms**(修复前)。
   低层 trait 版能省掉的只有 build 的 17ms,而 build 在持久化之后本来就只做一次;
   代价是把我们绑到 taffy 最不稳定的一层(0.12.2 自己的 `traits.rs` 模块文档
   都还停在旧签名,见 §4.3)。

**一句话:先做"便宜的一半"(§7 步骤 1–2,约 1.2 人周,拿 4–5 倍),
再做持久树(步骤 3–4,约 2 人周,拿再 15 倍)。低层 trait 版本列入"永不"清单,
除非影子树内存成为问题。**

---

## 1. 触发线复核:自己跑的数

机器 12th Gen Intel Core i5-12400(12 线程)/ Windows 11 / release / CPU 后端 /
1920×1080 @1.0。**这台机器上常有并行 cargo 编译,同一条命令前后能差 2–2.5 倍**
(实测同一条 membench 命令三连跑:233 / 244 / 117ms)。下面凡是绝对值都取
**多次中的最好一次**(= 最接近"机器安静"的读数),凡是结论都写成比值。

### 1.1 membench(仓库内基准台,`--scene rows` = 全量树)

```
READY backend=cpu scene=rows mutate=false virtual=false nodes=30001 signals=12001 \
      build_ms=14 warmup_ms=798 frame_avg_ms=117.44 p99_ms=144.67 low1_fps=3 fps=8 frames=20
READY backend=cpu scene=rows mutate=true  virtual=false nodes=30001 signals=12001 \
      build_ms=17 warmup_ms=792 frame_avg_ms=471.50 p99_ms=761.09 low1_fps=1 fps=1 frames=20
READY backend=cpu scene=virtual mutate=true virtual=true nodes=152 signals=63 \
      build_ms=0 warmup_ms=12 frame_avg_ms=6.40 p99_ms=9.18 low1_fps=109 fps=156 frames=60
```

规模扫描(同一批次连跑,机器相对安静那一轮):

| controls | nodes | 静止帧 avg | 变更帧 avg | 差值 ≈ 布局 |
|---|---|---|---|---|
| 1 000 | 1 001 | 6.76ms | 8.85ms | 2.1ms |
| 3 000 | 3 001 | 13.84ms | 19.31ms | 5.5ms |
| 10 000 | 10 001 | 39.89ms | 62.71ms | 22.8ms |
| 30 000 | 30 001 | 117–127ms | 419–472ms | **~300–350ms** |

**读法**:静止帧走布局缓存(`render.rs:458` `layout_full_cached`,键 = Doc 身份 +
版本 + 宽高),所以静止帧 ≈ 纯绘制;变更帧减静止帧 ≈ 一次全量布局。
节点数 ×3(10k→30k),布局成本 ×14 —— **已经不是线性**,这与 §2 的缓存抖动一致。

### 1.2 仓库内探针测试

`cargo test --release -p sv-shell -- layout_30k --nocapture`
(`crates/sv-shell/src/lib.rs:2461`):

```
[probe] 30k 全量 build+layout:冷 238.63ms / 热 256.97ms(2ms 触发线已越,增量升级列档 B)
```

安静时重跑同一形态的树,分段拆解后是 **88–96ms**(见 §2 表 A 行)。

> **⚠ 这个探针本身有问题,它把成本藏起来了。** 它建 6000 行 × 5 个文本,
> 但文本只有 `"标签"` / `"value"` **两种字符串**(lib.rs:2469)——
> `text::measure` 的缓存必然全命中,parley 一次都不跑。真实界面每行文本都不一样。
> 换成"每行唯一串"的同规模树,同一台机器同一时刻测得 **311–315ms**,
> **差 3.3 倍**。DESIGN.md 里记的"实测 ~130–160ms:taffy 裸 ~45ms + 叶子
> measure ~70ms"是在这个不具代表性的树上得到的,**分解也不成立**(见 §2)。
> 步骤 0 就是修这个探针。

---

## 2. 瓶颈拆解(问题 1)

方法:把仓库拷到 `%TEMP%` 下的临时目录,在 `layout_tree_full` 的三段之间插
`Instant`,并给 `text::measure` 加调用/未命中计数(计数器只做 `Cell` 自增,
**计时默认关**;第一版插桩每次调用都读 `std::env::var` 与算一次额外哈希,
把 measure 的单价虚高了 3–7 倍,发现后重做——记在这里因为这类插桩失真很容易
当成真结论)。仓库本身未改一行。复现方法见 §9。

### 2.1 分段计时(每帧全量重建,取 5 帧均值)

| 树形 | 配置 | build | compute | walk | 合计 | measure 调用/未命中 |
|---|---|---|---|---|---|---|
| A 探针树 36001 节点(2 种串) | 现状 CAP=4096 | 26–27ms | 58–65ms | 3.7–4.0ms | **88–96ms** | 300000 / **0** |
| B membench 同构 30001 节点(6000 种串) | 现状 CAP=4096 | 16–19ms | 292–304ms | 2.9–3.2ms | **311–315ms** | 180000 / **40378** |
| B 同上 | CAP=32768 | 16–18ms | 48–53ms | 2.8–3.3ms | **69–74ms** | 180000 / **0** |
| B 同上 | CAP=32768 + 叶内 memo | 16–19ms | 41–44ms | 3.1ms | **60–63ms** | **42000** / 0 |

**逐条结论:**

- **不是"重建 TaffyTree 的分配"。** build(递归造节点 + 每叶 `to_taffy` 造一份
  240B 的 `taffy::Style` + 每叶 `n.text.clone()` 一次堆分配,`render.rs:265–306`)
  只占 16–27ms,是全量帧的 5%(现状)/ 25%(缓存修好之后)。
- **不是"walk 的 Vec 分配"。** walk 稳定在 2.9–4.0ms / 30k 节点 ≈ 0.1µs/节点,
  占比 1%。顺手试了"用 `child_ids()` 迭代器替掉 `tree.children()` 的
  `Vec` clone"(`taffy_tree.rs:828` 每个容器节点一次堆分配),**没有可测收益**
  —— walk 的成本在 `HashMap<u64,ViewId>` 反查与递归本身,不在那次 clone。
  这条否定结论省下了一次没必要的优化。
- **是 measure 调用次数 × 单价,而且两个因子都爆了。**
  - **次数**:taffy 对每个叶子每帧问 **10 次**(180000/18000)。这是 taffy 自己的
    多趟协议(MaxContent 问固有宽 → MinContent → Definite 问折行后高),
    它的 9 槽缓存(`cache.rs:11 CACHE_SIZE = 9`)刚好装不下 10 个不同的
    `(known, available)` 组合。这 10 次落到我们的 `measure_leaf` 只对应
    **约 2.3 个不同的 `wrap_w`**,所以叶内 memo 能把 180000 压到 42000。
  - **单价**:18000 个测量叶产生 **18004 个不同缓存键**(每串约 3 个 `wrap_w`:
    `None` / `Some(0.0)` / `Some(定宽)`,而定宽逐行不同),而 `CAP = 4096`。
    两代淘汰在这种规模下退化成抖动:每帧 **40378 次未命中**,每次真跑一遍 parley。
    对照实验:4000 个键时第二轮起 0 新未命中;6000 个键时第二轮只有 96 次新未命中
    —— **两代缓存在 1.5 倍超配下还撑得住,4.4 倍超配就塌了。**

- **一个次生结论**:全局串表在大规模下连命中都不便宜。A 行 300000 次全命中
  (表里只有 2 条)与 B 行 180000 次全命中(表里 18004 条)相比,后者单次
  明显更贵——18004 条 × 约 40B 已经超出 L2,每次查表是一次内存往返 + 一次
  SipHash。**测量结果本该属于节点,不属于全局串表**;叶内 memo 是正解,
  全局表退居"跨节点共享同一串"的二线(如 6000 个 `"静态标签"` 的首帧)。

### 2.2 变更分级实测(这条决定了步骤 2 的价值)

| 场景 | 现状 | 持久树下 |
|---|---|---|
| 什么都没脏,只是重算一次 | 全量 311ms(版本一变缓存就废) | **0.001ms**(根节点一次缓存命中) |
| 改 50 个节点的前景色 | 全量 311ms | compute **0.0001ms** + walk 4.2ms |
| 改 1 个叶子文本 | 全量 311ms | compute **1.9ms** + walk 2.5ms |
| 滚动 1 像素 | 全量 311ms(实测见 membench `--scene scroll`) | compute 0ms + walk 2.5ms |

> membench 的 `scroll` 场景(并行落地的 `examples/membench/README.md`)已经独立
> 印证了这一点:**"滚动帧和全量突变帧同价"**(3k 档 21.75 vs 19.18ms)。
> 滚动偏移根本不进 taffy —— `walk_taffy` 把它当作子节点原点的平移
> (`render.rs:432–449`),但它 bump 了版本,于是整棵树陪着重算。

---

## 3. taffy 0.12.2 到底支持什么增量能力(问题 2)

版本:`cargo tree -p sv-shell -i taffy` → **taffy v0.12.2**,
源码在 `~/.cargo/registry/src/rsproxy.cn-*/taffy-0.12.2/`。

### 3.1 `TaffyTree` 能不能复用 —— 能,而且这就是官方的增量机制

- **每节点缓存**:`Cache { final_layout_entry, measure_entries: [_; 9], is_empty }`
  (`tree/cache.rs:138`)。所有布局入口(容器与叶子一视同仁)都走
  `compute_cached_layout`(`compute/mod.rs:174`),命中就直接 return,
  **不再递归下去**。
- **`mark_dirty(node)`**(`tree/taffy_tree.rs:873`):清本节点缓存,再沿 `parents`
  向上清;**遇到已经脏的祖先就停**(`ClearState::AlreadyEmpty` 分支)。
  于是"脏一个叶子"= 清 O(depth) 个缓存,兄弟子树的缓存原样保留。
- **为什么干净子树不需要重写坐标**:taffy 的 `Layout.location` 是**父相对**的,
  父在 `compute_flexbox_layout` 里对每个子调用 `set_unrounded_layout`
  (`compute/flexbox.rs:1998`),**无论那个子是不是缓存命中**。所以"上面的兄弟
  变高了,下面整块往下挪"这种情形,被挪动的子树内部一行都不用重算。
  这是整个方案成立的地基。
- **实测**:30001 节点持久树,连算两次,第二次 **0.001ms**。改一个叶子后
  0.5–2.5ms(取决于扇出,见 3.2)。

### 3.2 扇出决定增量的地板(实测,同为约 30k 节点)

| 树形 | 改 1 个叶子的 compute |
|---|---|
| root 直挂 6000 行 | 2.04ms |
| root → 20 组 × 300 行 | 2.51ms |
| root → 100 组 × 60 行 | **0.59ms** |
| root → 600 组 × 10 行 | **0.20ms** |

**读法**:脏叶子的每一层祖先都要重跑自己的 flex 算法,而 flex 是 O(直接子节点数)
—— 即使每个子节点都缓存命中,遍历本身跑不掉。**一个有 6000 个直接子节点的
容器,单叶变更的地板就是 2ms**,增量布局对它无能为力(这是 CSS flex 的语义,
不是 taffy 的实现问题)。扇出 ≤100 时地板降到 0.6ms 以下。
写进文档给用户:**大列表要么虚拟化,要么分组,别把一万个孩子挂在一个 flex 容器下。**

> 注:早期一轮里 "20 组 × 300 行" 测出 9–10ms 的异常高值,查明是 measure 缓存
> 抖动的叠加(CAP=4096),把 CAP 提上去后回落到 2.5ms 并恢复单调。这条记在
> 这里是因为它演示了一个陷阱:**缓存抖动会污染任何与它同帧的测量**。

### 3.3 低层 trait 是什么形状,自己实现意味着多少工作量

`tree/traits.rs`,0.12.2 的真实签名(**注意:该文件 1–128 行的模块文档已经过期**,
里面写的 `fn get_style(&self) -> &Style` / `get_cache_mut` 是旧版形状;
真身在同文件 148–255 行。照文档写编译不过——这本身就是"这一层在动"的证据):

| trait | 必须实现 | 说明 |
|---|---|---|
| `TraversePartialTree` | `type ChildIter<'a>`、`child_ids` / `child_count` / `get_child_id` | 直接读 `ViewNode.children` 即可 |
| `LayoutPartialTree` | `type CoreContainerStyle<'a>`、`type CustomIdent`、`get_core_container_style`、`set_unrounded_layout`、`compute_child_layout` | 分发到 flex/leaf 由我们自己写 |
| `LayoutFlexboxContainer` | `type FlexboxContainerStyle<'a>`、`type FlexboxItemStyle<'a>`、两个 getter | |
| `CacheTree` | `cache_get` / `cache_store` / `cache_clear` | **每节点要有 `taffy::Cache`(368B)存哪儿** |
| `CoreStyle`(样式视图) | 16 个方法,**全部带默认实现** | 只需覆盖我们用到的 10 个左右 |
| `FlexboxContainerStyle` / `FlexboxItemStyle` | 6 + 4 个方法,带默认实现 | |
| `RoundTree` | — | **我们不需要**:`disable_rounding()` 后 `round_layout` 不跑 |

工作量的真实构成不是这 30 多个方法(多数有默认实现,签名也直白),而是三件事:

1. **`taffy::Cache` 与 `unrounded_layout` 存哪儿。** 放进 `sv_ui::ViewNode` =
   sv-ui 依赖 taffy,违反"sv-ui 是编译目标、保持零依赖"的既有纪律
   (DESIGN.md R3 里为词边界规则专门重申过这条)。正解是 sv-shell 侧的
   `SecondaryMap<ViewId, (Cache, Layout)>` 旁表 —— 可行,但那已经是"半棵影子树"了,
   只是省掉 `Style`(240B)与 children/parents。
2. **脏标记还得自己写。** 低层 trait 不送 `mark_dirty`;`cache_clear` 是我们实现的,
   沿父链清缓存的循环也得我们写。**"该脏没脏"这个 bug 类在方案 (a) 和 (b) 里
   一模一样**,(b) 并不便宜。
3. **升级税。** 高层 `TaffyTree` 的 API(`new_leaf_with_context` / `set_style` /
   `mark_dirty` / `insert_child_at_index` / `remove_child`)是 taffy 最稳的一层;
   低层 trait 在 0.12 一个 minor 里就换过形状(见上文文档过期)。这与
   DESIGN.md 对 parley 定的"门面即防波堤"是同一条纪律,方向相反。

### 3.4 三个必须知道的 API 陷阱(源码核实)

- **`TaffyTree::remove(node)` 不标脏父节点。** `taffy_tree.rs:618–638` 把 node 从
  父的 children 里 `retain` 掉,却**没有** `mark_dirty(parent)`。直接 `remove`
  会留下一个缓存陈旧的父 —— 表现是"删了节点但布局没变"。
  必须先 `remove_child(parent, node)`(它在 `:752` 标脏)再 `remove`。
- **`get_node_context_mut` 不标脏。** `:663–667`。改文本的正确姿势是
  "改 context + 显式 `mark_dirty(node)`"。而 `set_node_context`(`:642`)会标脏,
  但它要求交出所有权,对"只改一个 String 字段"是浪费。
- **`set_style` / `add_child` / `insert_child_at_index` / `set_children` /
  `remove_child` / `remove_children_range` / `replace_child_at_index` 都自带
  `mark_dirty(parent)`** —— 这一半是免费的,危险的是上面两个例外。

---

## 4. 方案对比(问题 3)

### 4.1 三条路线

**(a) 持久化 `TaffyTree`** —— `ViewId ↔ NodeId` 映射常驻 sv-shell;结构变更增删节点,
样式变更 `set_style`,文本变更改 `MeasureCtx` + `mark_dirty`;每帧只在真脏的
子树重算。**输出 `Vec<Placed>` 契约不动。**

**(b) 自己实现低层 trait** —— 把 `DocumentInner` + 一张 `SecondaryMap<ViewId,
(Cache, Layout)>` 旁表作为 taffy 的"树",省掉影子树这一整层。

**(c) 不动 taffy,只做 measure 缓存 + 变更分级** —— 缓存扩容 + 叶内 memo;
Doc 记录"这一帧脏了什么类别",A 类(纯绘制)直接复用上帧 `Layout`,
B 类(只挪位置)只重走 walk,C 类才走现状的全量重建。

### 4.2 取舍矩阵

| | (a) 持久 TaffyTree | (b) 低层 trait | (c) 缓存 + 分级 |
|---|---|---|---|
| **预期收益**(30k,基于 §2 实测) | 变更帧布局 311 → **4.5ms**(compute 1.9 + walk 2.5);静止/纯绘制帧 → **~0** | 同 (a),额外省掉一次性 build 的 17ms 与影子树 ~24MB | 变更帧 311 → **60–70ms**;纯绘制帧/滚动帧/打字帧 → **0 或 3ms** |
| **对 1k–5k 中等树** | 3k 变更帧布局 5.5 → 约 0.5ms(按扇出外推,**未直接实测**) | 同 (a) | 3k 变更帧布局 5.5 → 约 1.5ms(**未直接实测**) |
| **工作量** | 2.0 人周(步骤 3+4) | 3.5–5 人周(**未核实**,无先例可比;含样式视图、缓存旁表、自写脏传播、一次上游升级演练) | 1.2 人周(步骤 1+2) |
| **风险** | 中:两棵树失同步、漏标脏 | 中高:同 (a) 的漏标脏,外加绑死 taffy 最不稳的一层 | 低:局部常数 + 一层旁路 |
| **可逆性** | 好:引擎是 sv-shell 内的一个结构体,退回 = 每帧 `LayoutEngine::rebuild()`(就是今天的行为),一个开关 | 差:样式映射与算法分发全部重写,退不回去 | 极好:两个 commit 各自可 revert |
| **对 sv-ui 的侵入** | 加一个脏日志 API(与 (c) 共用) | 同左 + 旁表要按 ViewId 世代作废 | 加一个脏日志 API |
| **内存** | 影子树常驻 ≈ **24MB @30k**(`taffy::Style` 240B + `Cache` 368B + 两份 `Layout` 168B + children/parents,实测 `size_of` 见 §9) | ≈ **13.6MB @30k**(只留 Cache+Layout) | 0(影子树仍是每帧临时) |

### 4.3 裁决

**做 (c),再做 (a);(b) 进"永不"清单(附触发条件)。**

- (c) 与 (a) 不是二选一,是同一条路的前后两段:(c) 的"变更分级"正是 (a) 需要的
  脏信息来源,(c) 的"叶内 memo"在 (a) 的持久 `MeasureCtx` 下**跨帧生效**
  (今天 `MeasureCtx` 每帧新建,memo 只在帧内有效)。先做 (c) 能在 1.2 人周内
  拿到 4–5 倍并把接口定下来,(a) 接上去是纯增量工作。
- (b) 相对 (a) 的净收益 = 一次性 build 17ms + 10MB 内存 + "两棵树不可能失同步"
  这个不变量;代价 = 多 1.5–3 人周 + 每次 taffy minor 的移植 + 不可逆。
  **(b) 的触发条件**:①影子树内存成为约束(例如 100k 全量树,影子树 ≈80MB);
  ②要支持 grid/block 且 `to_taffy` 造整份 `taffy::Style` 成为热点;
  ③taffy 的高层 API 出现我们必须绕过的语义。三条现在一条都不成立。

---

## 5. 正确性怎么保证(问题 4)

### 5.1 对拍策略 —— 已经跑通,不是纸上方案

在 `%TEMP%` 的临时副本里实现了持久树 + 手工脏标记,与 `layout_tree_full` 逐字段
对拍(30001 节点,`f32` **精确相等**,不设 epsilon):

```
[初始]        逐字段全等 OK (30001 条)
[改文本#0]    逐字段全等 OK (30001 条)     // 换成更短的串
[改文本#1]    逐字段全等 OK (30001 条)     // 换成撑宽整行的长串
[改样式]      逐字段全等 OK (30001 条)     // padding 4 → 12
[插入叶子]    逐字段全等 OK (30002 条)
[删除叶子]    逐字段全等 OK (30001 条)
[故意漏标脏]  MISMATCH 4 条,最大偏差 165.304688   ← 反例卫兵生效
```

**两个可以直接拿去用的结论:**

1. **精确相等做得到。** 不需要 epsilon,不需要"允许 1px 误差"。原因是 taffy 的
   缓存命中路径返回的是**上次算出来的同一批浮点数**,不是重算的近似值。
   一旦有人写出"差不多相等"的断言,这条测试就废了 —— 增量布局的典型 bug
   (漏标脏)产生的是**几十上百像素**的偏差,不是 ulp。
2. **反例卫兵是必需品。** 上表最后一行是故意只改场景树、不动 taffy。
   没有这一行,对拍测试有可能在某次重构后退化成"两边都调用同一份缓存"
   而永远通过。

### 5.2 三道防线(按"能不能被人忘掉"排序)

1. **编译期闸(最强)**:`Style` 的布局相关性判定写成**不带 `..` 的穷尽解构**:

   ```rust
   fn layout_relevant(a: &Style, b: &Style) -> bool {
       // 新增字段时这里会 "missing field" 编译失败 —— 这是故意的
       let Style { direction, gap, padding, /* …逐个列全… */ text_align } = a;
       let _ = (text_align,);              // 明确标注"这个不影响布局"
       *direction != b.direction || *gap != b.gap || /* … */
   }
   ```
   往 `Style` 加字段而不给它分类,**编译不过**。靠纪律记住"改了 Style 要同步
   改分类表"必然会漏 —— ADR-7 里 each 行的 signal 被 effect 悄悄销毁、
   R1 里 `:focus` 与 `onfocus` 互相顶掉回调槽,都是同一类"人记不住"的事故。

2. **对拍测试(CI 常驻)**:
   - `layout_incremental_matches_full` —— 一棵覆盖面完整的树(嵌套 / 滚动容器 /
     裁剪 / 弹层三层 / modal / textarea / flex wrap / 继承字号),把 `Doc` 的
     **每一个写方法各来一遍**,每次写后同时跑增量与全量,`Placed` 的
     `id / rect 四值 / clip / clip_depth` 全部精确相等,`ScrollArea` 与
     `OverlayRegion` 同样比。
   - `layout_incremental_fuzz` —— 固定种子的随机操作序列(建/删/改文本/改样式/
     滚动/开关弹层,1000 步),每 20 步对拍一次;失败时打印种子与操作序列。
     种子写死在测试里(不要用时间做种,否则 CI 变成随机红)。
   - `missed_dirty_is_caught` —— 反例卫兵,断言"跳过一次标脏 ⇒ 对拍必失败"。
3. **运行期开关**:`SV_LAYOUT_VERIFY=1` 时每帧算两遍并断言相等
   (debug 构建默认开)。CI 里用它跑一遍所有 examples 的离屏渲染
   (`cargo run -p counter -- --png`、showcase、settings-sfc、overlay-demo)。

### 5.3 现有测试的零回归面

`crates/sv-shell/src/lib.rs` 42 个测试里,直接吃布局产物的至少这些必须零改动通过:
`scroll_offset_shifts_children_and_clamps`、`clipped_child_not_hit`、
`wheel_routes_and_chains_to_ancestor_at_edge`、`scroll_clip_golden_and_cpu_pixels`、
`scrollbar_thumb_geometry`、`scrollbar_thumb_drag`、`virtual_list_driven_by_wheel`、
`text_wraps_at_container_width`、`cjk_wraps_without_spaces_and_respects_punct`、
`long_token_force_breaks`、`wrapped_measure_two_pass`、`flex_grow_and_justify_and_align`、
`gap_cross_axis_semantics_pinned`、`overflow_axis_split`、
`overlay_paints_after_base_and_hit_prefers_it`、`anchor_below_flips_when_clipped`、
`modal_blocks_base_and_traps_focus`、`tooltip_delay_and_never_hit`、
`a11y_roles_names_bounds_golden`(bounds 来自 `Placed.rect`)、
`recording_painter_golden` / `input_paint_golden` / `scroll_clip_golden_and_cpu_pixels`
(命令流/像素金样)。
**判据:这批测试一行不改就绿。** 任何需要改断言的地方都是回归,不是"预期变化"。

---

## 6. 变更分级表(步骤 2 的规格,也是漏标脏的清单)

按 `Doc` 的写方法逐个定级。**A 类整帧复用上一份 `Layout`;B 类只重走 walk;
C 类才动 taffy。**

| 类 | 写入口(`crates/sv-ui/src/lib.rs`) | 依据 |
|---|---|---|
| **A 纯绘制** | `set_checked`(:621) | `measure_leaf` 的 Checkbox 分支只看字号(`render.rs:344`) |
| A | `focus` / `blur`(:865) | 焦点环是渲染壳合成绘制,不进树(`render.rs:930` 附近) |
| A | `set_accessible_label` / `set_focusable` / `set_accepts_text` / 各 handler 注册 | 不进 `to_taffy`,不进 `measure_leaf` |
| A | `Style` 的 `bg` / `fg` / `corner_radius` / `opacity` / `cursor` / `border` 的**颜色** / `text_align` | `to_taffy`(:171)只读 `border.width`;`measure` 恒用 `TextAlign::Left`(`text.rs:178`) |
| A | **TextInput 的 value / 光标 / 选区 / 预编辑 / placeholder** | `measure_leaf` 的 TextInput 分支恒返回 `200 × 行高×rows`(`render.rs:353`),与内容无关。**这是打字帧的大红利**;哪天做 auto-size input 要升级到 C 类,分类函数里必须留注释 |
| A | Checkbox 的 `set_text`(label) | 同上,Checkbox 不测文本 |
| **B 只挪位置** | `set_scroll`(:1093)、平滑滚动逐帧推进 | 滚动只是 `walk_taffy` 里子原点的平移(`render.rs:445`),不进 taffy |
| B | `set_content_override`(:1141) | 只影响 walk 里的 `ScrollArea.content/max` 与钳制(`render.rs:404`) |
| **C 真布局脏** | `set_text`(:607)—— Text / Button | 进 `MeasureCtx.text` |
| C | `set_multiline`(rows) | 进 `MeasureCtx.rows` |
| C | `set_style` / `update_style`(:635/:650)且 `layout_relevant` 为真 | direction/gap/padding/margin/border.width/width/height/min·max_*/justify/align_*/flex_*/overflow*/text_wrap/font_size |
| C | `append`(:540)/ `remove`(:554)/ `clear_children`(:594) | 结构 |
| C | 窗口尺寸变化 | root `set_style` |
| C·特例 | **`font_size` 变化要沿继承链下传** | `MeasureCtx.px` 是 build 期用 `resolve_font_size`(`render.rs:116`)解析好的,taffy 不知道继承。改一个 View 的 `font_size`,其子树里所有"自己没设 font_size"的叶子都要刷 `px` 并标脏 —— **这是本方案最容易漏的一条**,fuzz 里必须有"改中间层字号"这个动作 |
| C·特例 | 字体注册表变化(系统字体安装/fallback 结果变) | 全局失效。今天没有触发点,但要在引擎上留一个 `invalidate_all()` 并注释清楚 |

`update_style`(:650)今天**无条件 bump**(没有相等剪枝,不像 `set_style`
在 :641 比了 `PartialEq`)。步骤 2 顺手补上:先 clone 旧值、跑完闭包再比,
不等才 bump 并定级。这一改本身就能消掉一批假脏帧。

**API 形状(sv-ui 侧,不引入 taffy 依赖)**:

```rust
pub enum DirtyKind { Layout, Structure, Position }
/// 一帧的变更日志。渲染壳每帧 take 走。
pub struct DirtyLog { pub items: Vec<(ViewId, DirtyKind)>, pub overflowed: bool }
impl Doc { pub fn take_dirty(&self) -> DirtyLog { .. } }
```

- **为什么是日志不是"每节点脏位"**:脏位要遍历全树才能收集,量纲是 O(n);
  日志是 O(变更数),而变更数正是我们要压的那个量。
- **`overflowed` 安全阀**:`{#each}` 整表重建一次能推进来上万条,记日志本身
  比重算还贵。超过阈值(建议 1024)就丢日志、置位,渲染壳看到它就
  `LayoutEngine::rebuild()`(= 今天的行为)。**这条同时是"漏标脏"的兜底**:
  最坏情况退化成现状,不会画错。
- 版本号 `version`(:374/:438)保留不动 —— 它还担着重绘触发、a11y 节拍
  (`incremental_tree_update`)、静止帧短路(`lib.rs:230` 的 `frame_key`)三份职责,
  与脏日志正交。

---

## 7. 分步落地(问题 5)

每步独立可合、独立验收。人周按单人全职估,置信度是对**工作量估计**的信心。

### 步骤 0 · 探针纠偏(0.2 人周,置信度 高)

- 改 `layout_30k_full_tree_budget_probe`(`lib.rs:2461`):文本改成每行唯一串;
  保留旧形态为第二个探针 `layout_30k_shared_text_probe`,两个都打印。
- membench 侧不动(`--scene text` 已经把 pool 内/外拆开了,是同一条洞见的另一面)。
- **验收**:两个探针的输出差 ≥3 倍(实测 88–96 vs 311–315ms);
  `cargo test --release -- layout_30k --nocapture` 的输出进 PR 描述。
- **为什么排第一**:后面每一步的收益都要拿它来量;量尺本身偏了 3 倍,
  后面所有数字都不可信。这一步只改测试,零风险。

### 步骤 1 · measure 成本(0.4 人周,高)

- `text.rs:160` 的 `CAP` 从常数改成**按上一帧 distinct 键数自适应**:
  `cap = next_pow2(last_frame_keys × 2).clamp(4096, 65536)`。
  65536 条 × 24B 载荷 ≈ 每代 0.9MB,两代 1.9MB —— 对照 ADR-9 的 28MB 基线可接受;
  上限必须有,否则一个每帧生成新串的界面会把内存吃光。
- `MeasureCtx`(`render.rs:159`)加 4 槽内联 memo,键 = `wrap_w` 的位型
  (`px` 与 `text` 在同一个叶子里是常量)。实测把 180000 次调用压到 42000。
- **验收**:
  - `layout_30k_distinct_text_probe` 从 ~311ms 降到 ~60–70ms(允许 ±30% 机器差);
  - `membench --scene text --controls 2000 --mutate` 的 **miss 档与 pool 档的差值
    不变**(说明只改了缓存命中率,没把 parley 单价也一起动了);
  - 全部现有测试零改动通过。
- **可逆性**:两个独立 commit,各自 revert。

### 步骤 2 · 变更分级 + 脏日志(0.8 人周,中高)

- sv-ui:`DirtyKind` / `DirtyLog` / `take_dirty`;`layout_relevant` 穷尽解构;
  `update_style` 补相等剪枝。
- sv-shell:`layout_full_cached` 升级成小状态机 —— A 类复用上一份 `Layout`
  (连 clone 都不用,改成 `Rc<Layout>`);B 类只重跑 `walk_taffy`;C 类走全量。
- 顺手修掉两个既有小问题:①`lib.rs:239/282` 在 `render_frame` 内部已经算过
  布局之后又调一次 `layout_full_cached`,拿到的是缓存命中但仍 clone 一份
  1.4MB(30k 档)——改 `Rc` 后自然消失;②布局缓存是**单槽** thread_local
  (`render.rs:462`),两个窗口交替会互相顶掉,今天表现为"每帧都全量"。
- **验收**:
  - 新测 `paint_only_change_reuses_layout`(改 `fg` 后拿到的是同一个
    `Rc<Layout>`,`Rc::ptr_eq` 断言);
  - 新测 `scroll_change_skips_taffy`(滚动后 taffy 未被调用,用计数器);
  - 新测 `typing_in_input_is_paint_only`;
  - `membench --scene scroll --controls 3000 --mutate` 从 ~21.75ms 掉到接近
    静止档(该场景的 README 已经把"有人给滚动加快路 → 这里应该掉下来"
    写成了验收口径,直接引用);
  - 现有测试零改动。
- **这一步之后就能对外说"滚动/打字/换色不再触发全量布局"。**

### 步骤 3 · 持久 TaffyTree(1.2 人周,中)

- sv-shell 新增 `LayoutEngine`:
  ```rust
  struct LayoutEngine {
      doc_id: usize, size: (f32, f32),
      tree: taffy::TaffyTree<MeasureCtx>,
      v2n: slotmap::SecondaryMap<ViewId, taffy::NodeId>,
      n2v: HashMap<u64, ViewId>,
      root: taffy::NodeId,
      overlays: Vec<(ViewId, taffy::TaffyTree<MeasureCtx>, ..)>,
  }
  ```
  - `v2n` 用 `SecondaryMap<ViewId, _>` 而不是 `HashMap` —— **ViewId 是世代键**,
    `Doc::remove`(:554)从 slotmap 删节点后世代 +1,`SecondaryMap` 天然让老键失效;
    用 `HashMap<ViewId,_>` 则会在 slot 复用时静默取到旧 `NodeId`。
    a11y 侧已经踩过同一个坑并定了同样的结论(DESIGN.md R3 P4:"NodeId=ViewId
    世代键")。
  - 删除路径必须 **`remove_child` 再 `remove`**(§3.4 第一条);
    改文本必须 **改 context 再显式 `mark_dirty`**(第二条)。这两条各配一个
    单测,测试名里写上"上游不标脏"。
  - `doc_id` / `size` 任一不匹配 → 整体 `rebuild()`。窗口 resize 走 root `set_style`。
  - 弹层:每个 overlay 一棵独立持久树(现状每帧新建,`render.rs:516–557`),
    按注册表增删;`OverlayRegion` 的区间语义不变。
- **验收**:
  - `layout_incremental_matches_full` + `layout_incremental_fuzz` +
    `missed_dirty_is_caught`(§5.1 已验证可行);
  - `layout_30k_single_text_change_budget`:≤ **5ms**(实测 4.45ms);
  - `layout_30k_idle_recompute_budget`:≤ **0.1ms**(实测 0.001ms);
  - 现有测试零改动;
  - membench 加一档内存采样对照,常驻影子树的增量 ≤ 30MB @30k。
- **可逆性**:留 `SV_LAYOUT_ENGINE=rebuild` 环境开关,一行退回今天的行为
  —— 这也是线上出问题时的止血阀。

### 步骤 4 · 增量 walk(0.8 人周,中低)

- 前提:**本帧结构未变**(有结构变更就整份重走,别在这里省)。
- 机制:保存上一帧每个 `ViewId` 的 `(location, size)` 与它在 `Vec<Placed>` 里的
  区间;walk 时若某节点的 taffy `Layout` 与上帧**逐位相等**且其子树内无脏节点 →
  整段 `Placed` 原样保留、不递归。
- 收益上界:单叶改文本且行高不变(最常见的形态)时,只有 1 条 `Placed` 变,
  walk 从 2.5ms 降到微秒级 → 变更帧布局进 **2ms 以内**。
- 顺手把 walk 从递归改成显式栈:membench README 记录了 `--depth ≥ 400` 时
  `build_taffy`/`walk_taffy` 在 Windows 1MB 主线程栈上**爆栈**;持久化让 build
  只做一次,walk 改显式栈后这个天花板一并抬掉。
- **验收**:`layout_30k_single_text_change_budget` 收紧到 ≤ **2ms**;
  `incremental_walk_matches_full`(对拍再加一层:增量 walk vs 全量 walk);
  `deep_tree_1000_layers_no_stack_overflow`。

### 步骤 5 · 收尾与 CI(0.5 人周,高)

- 多窗口/多 Doc(引擎按 Doc 身份存);DPI 变更;`invalidate_all()` 的调用点。
- 对拍两条测试进 CI 常规 `cargo test`;`SV_LAYOUT_VERIFY=1` 跑一遍所有 examples
  的离屏渲染。
- CI bench job 加一条 `--scene rows --controls 30000 --mutate` 的预算闸
  (**门槛照既有做法定得宽**:本地 ~120ms,CI 定 600ms,拦的是数量级回归)。
- 改 DESIGN.md R2 的触发线记录(见 §7.1)。

**合计 ≈ 3.9 人周**(原估 2–3 人周)。差额主要在两处:原估没算变更分级
(步骤 2,它是漏标脏防线的地基),也没算对拍与 fuzz 的成本。
**如果只批 1.2 人周,做步骤 0+1+2**,收益 4–5 倍且几乎无风险;
步骤 3–4 可以推到"绘制端也开始做增量"的同一批次里。

### 7.1 什么时候可以把档 B 的触发线标绿

DESIGN.md R2 现在写的是"**30k 全量档 2ms 触发线已越**(实测 ~130–160ms:
taffy 裸 ~45ms + 叶子 measure ~70ms)→ 按预案将'低层 trait 增量布局'列入档 B"。
建议改成三句:

1. **分解纠偏**:实测 30k 变更帧布局 ~311ms(build 17 / compute 292 / walk 3),
   compute 里主因是 `text::measure` 两代缓存容量不足导致每帧约 4 万次 parley 重排;
   原记录的"taffy 裸 45ms + measure 70ms"是在"全树只有两种字符串"的探针上测的,
   不代表真实界面。
2. **门槛改写**:2ms 这条线**对全量档没有工程意义** —— 同一棵 30k 树的
   纯绘制帧就要 117ms(CPU 后端)。把触发线改成
   **"30k 全量档的变更帧布局 ≤ 5ms、静止/纯绘制帧 ≤ 0.1ms"**,
   并注明"再往下压之前,先动绘制端"。
3. **翻绿判据**:步骤 3 合入且
   `layout_30k_single_text_change_budget`(≤5ms)与
   `layout_30k_idle_recompute_budget`(≤0.1ms)在 CI 绿 → 档 B 该项翻绿。
   步骤 4 是加分项(把 5ms 收到 2ms),不作为翻绿前置。

---

## 8. 什么情况下这件事不该做

### 8.1 瓶颈确实在 measure,不在树重建 —— 方案已按此改写

任务书里的假设是"如果实测发现瓶颈在 measure 而不是树重建,方案要怎么改"。
**实测就是这样**(§2.1:build 占 5%,walk 占 1%,measure 占 60%+)。改写后的形状是:

- 原来的单一大件"低层 trait 增量布局"拆成三段:
  **①测量成本(缓存容量 + 叶内 memo)→ ②不该重算的帧不重算(变更分级)→
  ③该重算的帧只重算脏子树(持久树)**;
- 顺序不能倒:先做 ③ 而不做 ①,那些**真需要重算**的帧仍然会被 measure 抖动拖死
  (§3.2 里 "20 组×300 行" 那个 9ms 异常就是活例);
- 低层 trait 从"方案本体"降级为"③ 的一个实现选项",并被否掉。

### 8.2 绘制是更高的墙 —— 30k 全量树在 CPU 后端上怎么都不可交付

实测 30k 静止帧(布局缓存全命中)仍要 **117ms**。把布局从 311ms 压到 2ms,
整帧从 ~430ms 变成 ~120ms —— **还是 8fps**。所以:

- **不要**用"30k 全量树能跑 144fps"来给本方案定目标,那个目标属于 ADR-9 的虚拟化;
- membench README 已经指出绘制端的头号机会是 **`text::shape` 没有缓存**
  (静止帧与"每帧全量改文本"帧几乎同价,2000 个 Text 档 52.14 vs 46.86ms)。
  **如果只能排一件事,那件事是给 `shape` 加缓存,不是增量布局** ——
  它同时降低静止帧与变更帧,而增量布局只降变更帧;
- 反过来说,`shape` 缓存做完之后,布局就会变成新的头号项 —— 两件事最终都要做,
  只是顺序上绘制在前。本方案的步骤 0–2 因为便宜且正交,可以插在任何位置。

### 8.3 虚拟化已经把"长列表"压住了 —— 真实收益场景是这四类

ADR-9 的虚拟化实测 100k 逻辑行只物化 152 个节点、p99 6.4ms(§1.1 READY 行),
**长列表这个形态不需要增量布局**。增量布局的真实收益在:

1. **中等规模的全量树**(1k–5k:设置面板、表单、属性面板、IDE 侧栏)。
   3k 档变更帧现在 19.3ms、其中布局约 5.5ms;压到 <1ms 后 3k 档整体进 60fps 预算。
   这是最可能被用户真正遇到的规模。
2. **滚动**(最高频交互)。今天滚 1 像素与改一行数据同价(membench `scroll` 场景
   实测 21.75 vs 19.18ms)。步骤 2 一步把它降为"只重走 walk"。
   **这一条与树的规模无关,小界面也受益。**
3. **打字**。`TextInput` 的 measure 与内容无关(`render.rs:353`),
   今天却每敲一个键触发一次全量重算。步骤 2 把它降为 A 类(零布局)。
4. **虚拟化盖不住的形态**:节点图、大表格的横向列、时间轴、代码编辑器的行内 span
   —— "一屏之内本来就有几千个控件",没有可虚拟化的一维长度。

如果一个产品只做"长列表 + 中小界面",且已经在用 `virtual_list`,
那么**步骤 3–4 的收益接近于零,只做步骤 0–2 就该收工**。

### 8.4 别在这些前提没定之前动手

- **API 冻结顺序**:步骤 2 会给 sv-ui 加公开 API(`take_dirty`)。DESIGN.md R4 把
  "双前端内核合并"列为 API 冻结的最后前置。脏日志这个 API 面很小、且是
  渲染壳专用(不进模板前端的词汇表),但仍应在冻结清单里过一遍,别在冻结之后加。
- **扇出建议要先写进文档**:§3.2 表明"一个容器挂 6000 个孩子"的单叶变更地板是
  2ms,增量布局救不了。这条要先写进 CSS/布局指南(`docs/zh-CN` + `docs/en` 两边),
  否则用户会拿着一个天生 2ms 的树来质疑增量布局没生效。
- **内存口径**:影子树从"每帧临时"变成"常驻 ~24MB @30k"。调研 16/17 的
  内存基线(0.5KB/控件)会被推高约 0.8KB/控件。这个数要先跟 ADR-9 的
  "WS 28MB @1M 虚拟化"口径对齐(虚拟化档只有 152 个节点,影子树可以忽略),
  别让内存基准 CI 在毫无预告的情况下变红。

---

## 9. 附录:实验怎么复现

仓库一行未改。全部实验在 `%TEMP%` 下的仓库副本里做:

```sh
# 1. 拷一份(只要 crates/ examples/ Cargo.toml Cargo.lock)
# 2. 打三处插桩:
#    a) crates/sv-shell/src/text.rs::measure —— 调用/未命中计数器(Cell 自增),
#       CAP 从常数改成 OnceCell 读 SV_MEASURE_CAP;
#       ⚠ 计时用 Instant 要放在开关后面,插桩本身别进热路径
#    b) crates/sv-shell/src/render.rs —— layout_probe_phases():
#       把 layout_tree_full 的 build / compute_layout / walk 三段分别计时
#    c) 同文件 PersistentProbe:持久 TaffyTree + touch_text/touch_style/
#       insert_leaf/remove_node/walk_layout,用来测增量与对拍
# 3. 用独立 target 目录跑,别污染仓库的 C:/cargo-target:
CARGO_TARGET_DIR=%TEMP%/ptarget SV_MEASURE_CAP=32768 \
  cargo test --release -p sv-shell -- probe_experiments --nocapture --test-threads=1
```

仓库内可直接跑的验证:

```sh
cargo build -q --release -p membench
./target/release/membench --scene rows --controls 30000 --frames 20 --hold-secs 0
./target/release/membench --scene rows --controls 30000 --frames 20 --hold-secs 0 --mutate
./target/release/membench --scene scroll --controls 3000 --frames 20 --hold-secs 0 --mutate
cargo test --release -p sv-shell -- layout_30k --nocapture
```

实测到的结构尺寸(`size_of`,用于内存估算):
`taffy::Style` 240B / `taffy::Cache` 368B / `taffy::Layout` 84B /
`MeasureCtx` 88B / `Placed` 48B / `sv_ui::Style` 136B。

**未核实项**(本文中凡是这几条,都已在正文标注):
- 1k–5k 中等树上各方案的收益数字(只在 30k 上直测,中等档是按扇出外推);
- 方案 (b) 的 3.5–5 人周估计(无先例可比,是按 trait 面 + 缓存旁表 + 一次升级
  演练粗估);
- vello 后端下的布局/绘制配比(本轮全部在 CPU 后端测;离屏 vello 含纹理回读,
  与 CPU 档不可直接比,要单独立基线);
- 影子树常驻内存的**实测**值(24MB 是按 `size_of` 相加估算,没有采样进程 WS)。
