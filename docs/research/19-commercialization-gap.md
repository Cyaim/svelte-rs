# 19 · 距离可商用状态还有多远:四路审计与分档判决

> 2026-07-18。方法:四路并行审计——①真实应用功能硬缺口(以"设置面板+列表+表单+
> 对话框"的工具 App 为基准逐项对代码核验)②平台与生产就绪度 ③工程成熟度
> ④业界可商用基线对标(Slint/iced+COSMIC/egui/Dioxus,联网核实)。
> 全部缺口均给代码证据;文档已有人周估算的一律引用。

## 0. 一句话判决

**现处 M0,相当于 iced 2019 / egui 2020 的起点**:业界"可商用最低配置"九项交集
中约七项完全缺失(键盘事件通道都尚未接入)。但 M1/M2 路线图与该交集**逐项重合**
——认知无盲区,缺的是执行时间。按先例校准:距"内部工具可用"约 **1–2 个季度**
全职;距"单桌面平台可商用"约 **2–3 年**全职(单人~小团队;egui 2 年下限、
Slint 3 年 50 贡献者、iced 6 年);四平台承诺(含鸿蒙)在此之上再加 M3。

## 1. 业界基线:被商用的 Rust GUI 九项交集

对四个已被真实商用的框架取交集(出处见文末):

| # | 交集项 | sv 现状 |
|---|---|---|
| 1 | 完整文本输入组件(光标/选区/编辑) | ❌ 无 TextInput 元素 |
| 2 | IME 组合文本(可粗糙但必须有) | ❌ 零(未 set_ime_allowed) |
| 3 | 剪贴板 | ❌ 零(路线图亦未列) |
| 4 | 文本换行 + 基本 shaping | ❌ 单行线性排版 |
| 5 | 滚动容器 + 滚轮 | ❌ 零(Painter 连 clip 都没有) |
| 6 | 焦点链 / Tab 导航 / 快捷键 | ❌ 零键盘事件 |
| 7 | flex 级布局引擎 | ❌ 仅行列顺排(taffy 在 M1) |
| 8 | HiDPI / 窗口生命周期 | ✅ 基本正确 |
| 9 | crates.io 稳定发版节奏 | ❌ 未发布(六个名字均未被占,好事) |

**不在交集内**(先例证明可后置):API 语义稳定(iced/egui 长期 0.x 照样商用)、
无障碍(egui 商用后才接 AccessKit;但政企/消费级会升格为硬门槛)、菜单/托盘/
模态/富文本(Slint v1.13/2025-10 才系统性补)。

先例时间线:Slint 2020 立项(ex-Qt 全职团队)→ 2023 v1.0("3 年、50 贡献者"),
商用集中在嵌入式;iced 2019 → COSMIC 1.0 随 Pop!_OS 24.04(2025-12),约 6 年,
IME 0.14/2025 才官方合入;egui 2020 单人 → ~2 年被 Rerun 商用(最快先例);
Dioxus 桌面靠 webview 把文本/IME/无障碍整体外包——反向印证自绘文本栈成本之高。

## 2. 分档距离

**档 A · 内部工具可用**(自己人用的 TodoMVC 级小工具):
需 M1 主体 = 键盘通道+焦点链 → 文本输入组件+剪贴板 → 滚动(滚轮+clip)→
文本换行 → taffy。已有估算仅 CSS C2 的 taffy 部分(+6–10 人周);文本输入与
滚动无估算。**粗估 1–2 个季度全职**。

**档 B · 单桌面平台可商用**(第三方敢采标交付):
档 A 之上 + M2 全量(Parley:IME/字体 fallback/emoji;AccessKit;深色模式/
多窗口)+ 弹层体系(对话框/菜单/下拉,依赖绝对定位+焦点)+ 发布工程
(crates.io、semver 承诺、CHANGELOG、打包/签名故事——目前是路线图盲区)+
API 冻结(前提:M1 双前端合并 + ADR-6 帧调度落地,二者都是语义级 breaking)。
**按先例 2–3 年全职**;最大不确定性 = 自绘文本栈(输入+IME+换行),所有先例中
最贵或被直接外包的一段。

**档 C · 四平台承诺**(鸿蒙一等公民):
档 B + M3:探底 1–2 周 + IME 再 1–2 周(调研 03 估算,仅探底非完成度)、
accesskit-ohos 自研桥 2–4 人周(调研 05)、hvigorw CI+签名。鸿蒙目前零代码,
且 ADR-4 的窄窗口 trait 尚未抽出(sv-shell 直接绑 winit 类型)——接入点还不存在。

## 3. P0 缺口清单(逐项带代码证据)

| 缺口 | 证据 | 排期 |
|---|---|---|
| 文本输入+IME 整链路 | ElementKind 仅 View/Text/Button/Checkbox(sv-ui lib.rs);事件循环无 KeyboardInput/Ime 分支(sv-shell lib.rs) | M1 隐含 / M2 Parley;无人周估算 |
| 键盘/焦点链/Tab/快捷键 | window_event 仅鼠标左键+悬停 | M1;无估算 |
| 滚动体系(容器+滚轮+clip) | 全仓 MouseWheel 零命中;Painter 无 clip 动词 | M1;无估算 |
| 文本换行 | render.rs 自认"简化线性排版";measure_text 恒单行 | M1 换行 / M2 Parley |
| flex 布局 | measure/place 仅 direction/gap/padding/margin;对齐/伸缩/百分比全无 | M1 taffy(C2 +6–10 人周) |
| API 冻结前提未到 | 全 crate 0.0.1;已排期三类 breaking(M1 合并、on: 移除、ADR-6 时序) | M1 后 |
| .sv IDE 支持为零 | 风险清单第 1 条自认最大悬置 | 及格线 9–16 人周 |

P1 摘要:弹层体系(无 z-index/绝对定位/遮罩)、图片元素(Painter 无 draw_image)、
指针事件窄(无右键/双击/拖拽/坐标回调,click 在 press 触发不合桌面惯例)、
无障碍零集成、字体无运行时 fallback 链/emoji、交付面空白(打包/签名/崩溃报告/
日志框架全无,且不在路线图)、帧调度未做(vello 窗口 vsync 钉 60;窗口路径
storage-buffer 128MB 上限未修)、运行时库 panic 点(sv-shell 8 expect、
sv-reactive 10 expect+4 panic 部分为有意设计)、测试结构单一(无 fuzz——
编译器/解析器类商用前通常需要)、热重载缺失(M4)。

## 4. 已达标项(商用评估的加分面)

- **性能与内存故事过硬**:基线 27MB/首帧 11ms(swash,调研 18);100 万控件
  p99=5.28ms、1% low=174fps(ADR-9);静止帧零功耗短路。
- **渲染架构有工程质感**:Painter 可切换、双后端 parity 1.017、金样测试、
  探测回退;HiDPI/CJK/离屏验证管线齐全。
- **响应式+编译器面领先同期**:runes 内核完整、Svelte 语法面 43/77、双前端闭环
  ——这是四个对标框架都没有的差异化,且此层已达可用质量。
- **工程纪律超原型平均**:114 测试全绿、3-OS CI(真字体+软件 Vulkan 渲染覆盖)、
  零 unsafe、双许可文件、9 ADR+19 调研+两张矩阵的决策留痕、风险清单诚实自评。
- **维护面收敛策略正确**:渲染/文本/布局/无障碍全押 Linebender,自研面收敛到
  编译器+响应式+组件运行时——与 egui 成功先例同构,是小团队走通的前提。

勘误:审计中"代码未纳入版本控制"表述不准确——远端 github.com/Cyaim/svelte-rs
已有全部代码与 32 个合并 PR(经 Git Data API 发布);真实问题是**本地 git 历史
与远端脱节**、无本地提交纪律,属流程债而非代码失踪。

## 5. 建议的最短商用路径(按依赖序)

1. **键盘事件通道 + 焦点链**(一切输入交互的地基,M1);
2. **文本输入组件 + 剪贴板 + IME**(九项交集的 3 项,最大不确定段,尽早探底);
3. **滚动体系**(滚轮+clip+滚动条,让 virtual_list 接上真实输入);
4. **换行 + taffy**(文本与布局同批,C2 的 6–10 人周含后者);
   —— 至此档 A ——
5. **Parley 迁移**(fallback/emoji/IME 组合文本渲染)+ **AccessKit**;
6. **弹层体系**(对话框/菜单,依赖 4 的绝对定位与 1 的焦点);
7. **发布工程**(crates.io 首发前一次做完:改名裁决、semver、CHANGELOG、
   社区文件 1–2 天、去 panic 审计 1–2 周、fuzz 1 周、clippy 转阻塞 1–2 天);
8. **API 冻结**(M1 合并 + ADR-6 之后)→ 档 B;鸿蒙 M3 → 档 C。

## 出处(业界对标)

- https://slint.dev/blog/announcing-slint-1.0 · https://slint.dev/blog/making-slint-desktop-ready · https://slint.dev/showcase
- https://en.wikipedia.org/wiki/COSMIC_desktop · iced PR #2777(IME)
- egui PR #2294(AccessKit)· Rerun(egui 商用)
- https://dioxuslabs.com(0.7 发布说明)
