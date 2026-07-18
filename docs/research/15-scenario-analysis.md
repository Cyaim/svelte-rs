# 15 · 三类场景下的现状分析:轻量内存 / 复杂界面 / 复杂界面+3D

> 2026-07-18。基于本仓库实测(release 构建、Windows 11、msyh.ttc 系统字体)与
> 既有 14 份调研的交叉引用。结论先行:**当前架构的"天赋点"押在轻量与中小界面上,
> 且押对了;复杂界面的短板全部有既定路线(M1/M2 清单覆盖);3D 复合是唯一
> 尚无设计的空白,需要新增一条 ADR 级决策(外部纹理节点),且以 wgpu 后端为前置。**

## 0. 实测基线(release)

| 指标 | 实测值 | 评注 |
|---|---|---|
| 二进制体积 | counter 1.29MB / showcase 1.43MB | 无 GPU 栈、无巨依赖;对托盘工具/小组件级场景非常好 |
| 核心结构体 | `ViewNode` 232B · `Style` 88B · `Edges` 16B | 万节点 ≈ 2.3MB,场景树本体不构成内存问题(已加护栏测试 `core_struct_sizes_within_budget`) |
| 运行内存 | **~188MB WorkingSet**(counter 与 showcase 几乎相同) | **异常项,已归因**:fontdue 对 18.8MB CJK TTC 的急切全量解析(≈10× 膨胀,已知行为);UI 本体与像素缓冲仅个位数 MB |
| 响应式节点 | slotmap arena + `Box<dyn Any>` 值 | 每信号 ≈ 数十字节 + 值本身;无全局订阅表,作用域销毁即回收(`debug_node_count` 测试保证) |

**fontdue 内存问题是横跨三场景的第一优先级工程项**(叠加其 2025-02 起停更,
调研 13 已入册):swash/Parley 走懒解析 + zero-copy 字体数据,迁移后预计
运行内存回到 ~20MB 量级。短期止血选项:非 CJK 字体优先探测(西文场景 ~30MB)、
字体子集化、或 Glifo 光栅路径。

## 1. 场景一:轻量内存(托盘工具、小组件、低端机、常驻进程)

**现状优势(天赋点)**:
- CPU 渲染栈零 GPU 依赖:无驱动兼容性、无 wgpu 初始化延迟与显存占用,
  远程桌面/虚拟机友好——这正是 ADR-3b 保留 CPU 兜底档的理由;
- 编译期样式表 + 零运行时选择器/解析器:样式系统运行时开销为零;
- 细粒度更新:静止 UI 零 CPU(无轮询、无每帧 diff),写入才醒;
- 二进制 1.3MB、节点 232B、编译产物是普通 Rust 函数(无解释器/字节码)。

**现状短板**:
1. **fontdue CJK 内存**(见上,首要);
2. **无脏区(damage tracking)**:任何变更全窗重绘重光栅——小窗口可接受
   (480×400@2x ≈ 每帧 3MB 像素写),但常驻进程的功耗画像不佳;
   `RecordingPainter` 命令流正是未来帧间 diff 的载体(调研 14 预留);
3. 字形无缓存:每帧对可见文本重新光栅(CPU 时间而非内存;调研 14 已列
   coverage LRU 到 TinySkiaPainter 的清单项)。

**判决**:架构契合度最高的场景。补齐"字体懒解析 + 字形 LRU + 脏区"三件后,
可以按"~20MB 内存、1.5MB 二进制、静止零功耗"给出承诺性指标。

## 2. 场景二:复杂界面(IDE 式多面板、长列表、富文本、高频更新)

**现状能扛住的**:细粒度更新模型本身(签名级精准更新不随界面规模退化——
这正是相对 immediate-mode(egui)与 VDOM 系的结构优势);keyed each 的行作用域
复用;组件模型与 scoped 样式的工程化组织。

**会破的点(全部有既定路线,无设计空白)**:
1. **全量布局 + 全量重绘**:每帧对整树 O(n) measure/place + 全窗光栅。
   万节点 CPU 光栅在 4K 下不可行 → **vello classic(M2)是硬前置**
   (GPU 光栅 + 后续脏区);
2. **无滚动与虚拟列表**:长列表(万行日志/文件树)需要 overflow 滚动容器
   (M1 路线图)+ 视窗化 each(只实例化可见行——keyed each 的行作用域
   恰好是虚拟化的正确底座,新增 `each_block_virtual` 原语即可);
3. **文本栈**:fontdue 无 shaping 缓存、无换行/富文本/省略号;Parley 迁移
   (M2)同时解决能力与内存;
4. **布局能力**:行列堆叠不够表达 IDE 布局 → taffy(M1);
5. **帧调度缺位**(ADR-6):高频写入现在是写后同步 flush,复杂界面需要
   事件→batch→帧前统一 flush 的管线,否则一次拖拽触发多次全量重算。

**判决**:响应式内核与编译产物形态已为该场景设计好;瓶颈全部集中在
渲染/布局/文本三条基建线,即 M1/M2 清单本身。风险是顺序而非方向:
**vello + taffy + Parley + 滚动 + 虚拟列表** 五件套齐之前,不应宣称支持该场景。

## 3. 场景三:复杂界面 + 3D(CAD/数字孪生/编辑器:3D 视口嵌入 UI)

**现状:唯一真正的设计空白**——不仅未实现,连 ADR 都没有。硬障碍:

1. **合成通道不存在**:当前 CPU pixmap 与用户 3D(必然 GPU)无法廉价合成
   ——GPU→CPU readback 每帧数十 MB,不可行。**结论:3D 场景以 wgpu 后端
   (vello classic)为硬前置**,共享 `wgpu::Device`,3D 内容渲到
   `wgpu::Texture`,UI 侧作为图像节点合成(vello 的 image brush 天然支持);
2. **场景树缺"外部表面"节点**:需要新增 `<surface3d>` 元素类型:持有用户
   回调(device, texture, size, dt)、resize 协商、输入事件路由(指针进入
   视口后转发原始事件而非命中测试);
3. **帧调度耦合**(ADR-6 升级为硬前置):3D 连续渲染(每帧)与 UI 按需渲染
   (变更才画)要在同一 surface 上协调——vsync 驱动、UI 层可缓存
   (`RecordingPainter` 命令流不变则跳过 UI 重编码);
4. **先例可抄**:egui 的 `PaintCallback`(wgpu 回调嵌入)、iced 的
   `shader` widget、Bevy 的 UI-over-3D——共同形态都是"UI 后端暴露 device +
   用户填纹理 + 合成器当图像用"。

**判决**:可行但排在 M2(vello)之后作为 M2.5/M3 议题;应新增 ADR
(`<surface3d>` 外部纹理节点 + 输入路由 + 帧调度协同),避免 M2 的 Painter/
合成设计无意间堵死这条路——**本次分析建议在 Painter trait 预留
`external_texture` 能力位(caps),vello 后端实现,CPU 后端声明不支持。**

## 4. 场景 × 后端矩阵(与 ADR-3b 对齐)

| | 轻量内存 | 复杂界面 | 复杂界面+3D |
|---|---|---|---|
| 渲染后端 | vello_cpu(接替 tiny-skia)+ softbuffer | vello classic(wgpu) | vello classic + 共享 device 外部纹理 |
| 文本 | Parley(懒解析,解决 188MB 问题) | Parley(shaping/换行/富文本) | 同左 |
| 布局 | 现行行列 or taffy | taffy 必需 | taffy 必需 |
| 关键缺件 | 字形 LRU、脏区 | 滚动、虚拟列表、帧调度 | `<surface3d>` ADR、输入路由 |
| 现状可用度 | ✅ 今天即可(内存修复后) | 🚧 M2 五件套后 | ❌ 需新 ADR,M2.5+ |

## 5. 行动清单(按杠杆排序)

1. **Parley/swash 迁移提级**(同时解决:188MB 内存、fontdue 停更风险、
   复杂界面文本能力)——原 M2 项,建议 M1.5;
2. Painter trait 增补 caps 协商位,预留 `external_texture`(为 3D 不堵路,一行接口的代价);
3. 新增 `<surface3d>` 设计 ADR(M2 启动前定稿);
4. 虚拟列表原语 `each_block_virtual` 挂在滚动容器设计里(M1);
5. 内存/功耗预算进 CI:结构体护栏已加,补"运行内存 < X MB"的冒烟基准(字体迁移后启用)。
