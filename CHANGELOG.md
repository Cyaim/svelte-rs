# 更新日志 / Changelog

格式沿用 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循下述 **0.x 语义化政策**。

## 版本政策(0.x 期)

尚未 1.0,API 会变。本仓库对 0.x 采用 Cargo 的 0.x 兼容规则并额外承诺:

- **minor 号(0.X.0)= 破坏性变更**;**patch 号(0.0.X)= 向后兼容**。
- 破坏性变更必须在本文件 `Changed` / `Removed` 段落写明**迁移方式**。
- 每次发版前跑 `cargo semver-checks`,漏报的破坏性变更按 bug 处理。
- 三类**已排期**的破坏性变更——双前端内核合并、`on:` 事件语法收敛、
  帧调度语义(ADR-6)——**已于 2026-07-22/23 全部落地**(迁移方式见下方
  [未发布] 的变更/移除段);谈 1.0 还差 crates.io 首发与稳定期。

首发 crate(同版本号、依赖序推送):sv-reactive → sv-ui → sv-compiler →
sv-macro → **sv-lottie / sv-pag**(sv-shell 的可选依赖,即便默认关也须在
registry 就位——crates.io 要求 optional 依赖同样已发布)→ sv-shell →
伞 crate svelte-rs,共 **8 crate**。**不随首发的 crate 均已标 `publish = false`
作为误发护栏**(都不在这 8 个的依赖闭包里):`sv-lsp`(LSP MVP)、`sv-vap`
(VAP,需应用侧 H.264 解码)、以及组件库轨道的 `sv-arco` / `sv-arco-tokens`
(随 D 单独排期)。`examples/` 不发布。

## [未发布]

### 新增

- **sv-arco-tokens:Arco Design 令牌层(调研 26 A0 波次)**:色板算法自
  `@arco-design/color` 0.4.0 逐式移植(HSL 色相路由/构造钳制/JS 舍入/暗色板
  hex 量化四个数值细节全对齐),金样 = 上游仓库自带 jest 断言原文机械提取
  (13 色板 × 亮暗 × 10 档逐字符一致);`global.less`(web-react 2.66.16,
  commit 已 pin)经入库生成器转译为 Rust 常量 + `:root` CSS 亮暗双出口,
  `tests/sync.rs` 防漂移。合规:LICENSE-ARCO + 非官方声明。
- **sv-arco:Arco 风格组件库 A1 静态件七件(调研 26 A1 波次)+
  examples/arco-gallery**:**Button** 全矩阵(4 变体 × 4 状态 × 4 尺寸 ×
  disabled)、**Tag**(14 色板 × 4 档,arco 预设缺的 yellow 按通式补齐)、
  **Badge**(standalone:count 胶囊/99+/dot,红绿蓝灰接线含 gray-4 怪癖)、
  **Divider**(纯线/线-字-线/纵向)、**Alert**(info/success/warning/error ×
  有/无标题,1px 透明边框并入 padding 保几何)、**Typography** 子集(字号
  阶梯 + 色档;secondary 按 token 原文取 text-2)、**Link**(四状态 + 禁用,
  warning 禁用色照抄 arco 的 light-2 怪癖)。取值全部就近抄 vendored
  `*-token.less`(7 份入库),行为测试 36 项(含 Button/Link 的 hover/active
  离屏状态迁移);build.rs 把令牌 `:root` 块注入各组件 `<style>` 后走
  sv-compiler(`:root` 作用域是每文件独立的)。A1 降级口径(无图标/无阴影/
  单字重/无过渡,disabled 走条件类)见 crate README。**对外是 Rust 函数
  API**(组件注册表单构建目录,跨 crate 无 `<Button>` 标签)。两 crate 均
  `publish = false`,不入首发清单。经 4 视角对抗评审(18 确认/1 驳回),
  顺带登记编译器层缺口(open-issues):if 块包装节点挡拉伸、plain 变量多
  同级闭包 move 冲突、arco 1px 透明边框未补偿几何(亚感知级)。
- **scroll-blit + 脏矩形(CPU 呈现路径)**:滚动帧把上一帧像素按位移复制、
  只重画新露出的条与滚动条列;打字/勾选/换色/焦点/光标闪烁只重画对应矩形
  (`DirtyItem::Paint` 带上了 `id`)。损伤重画走同尺寸 scratch(白底完整
  DFS + shaping 前节点剔除,坐标不平移),按矩形拷回——与全量渲染**逐字节
  相同**由差分测试守着(两种 DPI、混合操作流)。blit 入场券是整数物理位移:
  平滑滚动/滚轮/拖动的偏移全部吸附到物理像素网格(`anim::set_scroll_quantum`,
  离屏与测试不受影响)。守卫齐全:弹层/矢量动画/结构变更/小数位移/视口内
  外来绘制(隔离扫描)一律降级为多画。实测(release)3000 控件滚动场景
  离屏 12.9 → **2.2ms/帧**(5.9×,p99 3.4ms),开窗 55 → ~100fps;
  `SV_DAMAGE=0` 一键关闭。membench 新增 `--blit` 模式与 `blit_frames=`
  等尾部字段。

- **描边与路径的 Mask 裁剪(修可见渲染 bug)**:`stroke_rounded_rect` 从
  "允许出血"改为懒建矩形 Mask 遮罩——被滚动视口裁一半的输入框不再把边框
  画到滚动区外;`fill_path`/`stroke_path` 的"路径不吃裁剪"已知缺口一并补上
  (无裁剪场景零成本)。

- **增量布局(变更分级 + 持久布局树)**:每次 `bump` 分 Paint / Position / 重建
  三级(分级是 `bump` 的必传参数);渲染壳侧保留一棵**只读**布局树,滚动/打字/
  换色只走 Position 或 Paint,不再触发全树重布局(约 6000 节点滚动列表:全量
  29ms → 滚动帧 0.66ms)。C 类(结构)变更仍整棵重建。差分 fuzz 对拍全量重算。

- **`sv check`**:`cargo check --message-format=json` → 源码映射 → rustc 风格输出,
  把 `.svelte` 编译错误的行列指回 `.svelte` 源(而非生成的 `.rs`);附 `.vscode/tasks.json`
  的 problemMatcher。映射覆盖率实测 80.5%(胶水/runes 改写产物退到行级近似)。

- **三种动画格式(解析 + 像素,独立 crate)**:
  - `sv-vap`(腾讯 VAP):MP4 里抠 `vapc` 配置 + alpha/RGB 并排合成,
    `examples/vap-gift` 端到端;与 Python 参考在真实素材上逐字节对拍。
  - `sv-pag`(PAG):零依赖纯 Rust 解析位图序列帧容器档(WebP 解码与真实素材
    验证未做,见 `docs/plans/open-issues.md`)。
  - `sv-lottie`(Lottie):基于 velato(`default-features=false`,依赖树无
    vello/wgpu),自发路径命令走 tiny-skia 像素;**`AnimSource::Vector` 已接入
    场景树**——壳侧 `register_vector` 注册 + 绘制路径 `render_vector` 每帧现算
    路径直发 `Painter`(不落位图,缩放无损),裁剪栈成对平衡。
- **`Painter::draw_image`**:CPU(tiny-skia)/ vello / Recording 三后端统一图像绘制,
  是三种动画格式的共同地基;`ElementKind::Animation` 单一 kind 装所有格式,
  `set_anim_frame` 定级 Paint(一秒 60 次换帧零布局)。

- **`.svelte` 的 `<animation>` 叶子标签**:`<animation src="..." loop autoplay
  label="..." />`,建 `ElementKind::Animation` 节点(标签名描述用途、不绑格式)。
  素材经壳侧 `register_vector`/`register_frames` 接入。`view!` 宏按 ADR-2 冻结策略不加。

- **`sv-lsp`——`.svelte` 语言服务器(LSP MVP)**:打开/改动 `.svelte` 即把编译前端诊断
  (未知标签、非法属性、runes 改写失败、样式语法)实时变成编辑器波浪线
  (`textDocument/publishDiagnostics`,Full 同步)。零外部依赖(手写 `Content-Length`
  分帧 + JSON-RPC)。与 `sv check` 分工:LSP 管编辑期高频的前端错,`sv check` 管
  rustc 类型错。仍未做:补全/跳转/hover。

- **PAG 差分帧重放 + WebP 解码**:`sv_pag::replay_frame` 把位图序列(关键帧 + 脏矩形
  差分)从最近关键帧逐帧覆盖还原成整帧 RGBA,**sv-pag 仍零依赖**(解码器由调用方注入);
  `sv_shell::register_pag_webp` 用内置 `image-webp`(纯 Rust)解码后进帧注册表 → 场景树。
  仍缺:真实 `.pag` 素材验证(仓库无真文件)。

- **增量 Measure(布局)**:一帧里只有 `Measure` 变更(结构没动)时复用布局树,
  只让 taffy 重算脏子树,不整棵重建(计划步骤 3 的安全子集,不碰结构性 taffy 操作)。

- **平滑滚动**(R2 档 B S6):鼠标滚轮走 140ms ease-out 逼近目标;
  触摸板 PixelDelta 保持直通。

- **无障碍滚动与弹层语义**:可滚容器报 `ScrollView` + 偏移与 `ScrollUp/Down/
  SetScrollOffset` 动作;裁剪容器报 `clips_children`;多行输入报
  `MultilineTextInput`;弹层根按层与 modal 位报 `Dialog`/`Menu`/`Tooltip`。

- **`overflow-x` / `overflow-y` 按轴拆分**(R2 档 B):`overflow` 简写写两轴;
  分轴支持"横向裁掉、纵向滚"。

- **keyup 与捕获段**(R1 档 B):`KeyEvent.phase`、`onkeyup` / `on_key_up`、
  `Doc::set_on_key_capture`(root→焦点,先于冒泡)。抬起不触发默认段。

- **`:focus` 伪类**(`.btn:focus` / `&:focus`):接焦点链,与 `onfocus`/`onblur`
  合成一次设入;写了 `:focus` 的元素自动可获焦。
- **滚动条 thumb 拖拽**(调研 22 S4):命中带容差、记住抓点、按比例反算偏移。

- **多行 `<textarea rows="N">`**:与 `<input>` 共用编辑内核与全部属性;
  Enter 换行、粘贴保留换行、按内容宽折行、↑/↓ 按视觉行移动。

- **`#[derive(Store)]`(sv-macro)**:结构体 → 字段级信号 store
  (ADR-1 里 Proxy 深层响应的替代品);改一个字段不再叫醒只读别的字段的 effect。

- **无障碍增量推送(调研 24 P6)**:语义树只推内容变动的节点,不再每次全量。

- **R3 弹层体系**:离散层(Base→Popup→Tooltip)+ `overlay_block` 原语 +
  `.svelte` 的 `<overlay>` 内建元素;锚定四侧 + 越界翻转、关闭策略三值 + Esc LIFO、
  模态区间阻断与焦点陷阱、tooltip 悬停延时、Popup 内方向键导航。
- **R3 无障碍**:AccessKit 接入(`build_tree_update` 纯函数树映射 +
  accesskit_winit 懒激活适配器 + 动作回派)。
- **R3 文本栈**:Parley 0.11 接管 shaping/折行/光标几何,fontique 系统字体
  发现与 script fallback(CJK/Latin 混排不再出方框)。
- **文本输入编辑手势**:词跳与删词(Ctrl/⌥+←/→、Ctrl+Backspace/Delete)、
  拖拽选择、双击选词、三击全选、撤销重做(Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z)。
- **性能基准 CI 化**:membench 两场景(3k 全量 / 100k 虚拟化)p99 帧预算护栏。
- **发布工程**:`ShellError` 类型化错误、cargo-deny 依赖审计、MSRV 1.88 构建道、
  clippy `-D warnings` 阻塞门禁、发布演练(依赖序 `cargo package`)。

### 修复

- **条件类上的 `:active`/`:focus` 被 codegen 静默丢弃**(sv-arco A1 对抗评审
  查获):`class:x={cond}` 形态的伪类变体里,此前只有 `:hover` 被收集,
  `:active`/`:focus` 整块无声丢——用条件类承载全部变体态的组件(如 arco
  Button/Link)按压/聚焦态从不生效,且无编译错误无警告。修法:codegen 加
  `active_conds`/`focus_conds` 与 `hover_conds` 对称收集,在 active/focus
  block 里加条件门控臂。静态类路径本就正确不受影响;`.svelte` golden 逐字节
  不变(无 fixture 组合条件类与这两个伪类)。契约测试
  `conditional_class_active_and_focus_variants_emit`(产物字符串 + 变异探针)
  守着。
- **弹层不进语义树**:弹层是游离子树(不挂任何父),此前对屏幕阅读器**整个
  不存在** —— 对话框/菜单读不出来。现在接到 root 的 children 名下。

### 变更

- **`Doc::bump` 现在必传变更分级参数(API breaking)**:签名从 `bump()` 改为
  `bump(item: dirty::DirtyItem)`,漏定级是编译错误。迁移:调用点按语义选
  `Paint`(仅重绘)/ `Position`(重走坐标)/ `Structure` 等分级;绑定原语的
  改写产物已随之更新。这是"没有分级就没有增量布局"的地基,故列为破坏性变更。

- **keyed `{#each}` 行内容原地更新(ADR-7)**:行改持 `Signal<T>`,同 key 换内容
  不再显示旧数据(顺带修掉"列表一变就把所有行作用域悄悄销毁"的 bug);顺序未变
  时零树改动。迁移:项类型需 `Clone + PartialEq`;keyed 绑定名须为单个标识符
  (解构改用 `{@const}`);`sv_ui::each_block_keyed` 的 row 回调签名改为
  `Fn(&Doc, ViewId, Signal<T>)`。
- 新增 `sv_reactive::with_owner`:在指定作用域下建节点(effect 内建"活过重跑"的
  子作用域,同时保住 context 沿 owner 链可达)。

- **双前端共享 codegen 内核(ADR-2 无悔三步 ①)**:新增 `sv_compiler::emit`
  作为两个前端对 sv-ui 的唯一发射口;`sv-macro` 现依赖 `sv-compiler`。
  对用户无行为变化(两条路线生成的代码形状不变)。

- **双前端内核合并完成(ADR-2 ①,2026-07-23)**:公共模板 IR 转正为
  `sv_compiler::template`(表达式载荷双态:`.svelte` 源码串+字节偏移 /
  `view!` 带真 span 的 token 直通),codegen 只剩一份(宏侧入口
  `sv_compiler::generate_template`),两个前端只剩各自 parser。
  **两条路线的表面语法与生成代码语义都不变**(`.svelte` 产物逐字节不变、
  `view!` 行为测试零改动);对库使用者的可见变化是 `sv_compiler` 新增
  pub 模块 `template` 与函数 `generate_template`。
  唯一的宏表面收紧:`placeholder`/`bind_value`/`on_input`/`on_submit`
  用在 `<input>` 以外的标签上,从"编译通过、运行时静默无效"改为**解析期
  编译错误**(指到属性名;对齐 `.svelte` 前端的标签守卫)。迁移:删掉
  这些本来就无效的属性即可。

- **标识符改名(ADR-10 收尾,API breaking)**:`sv_compiler::compile_sv` /
  `compile_sv_with` / `compile_sv_mapped` → **`compile` / `compile_with` /
  `compile_mapped`**(`.sv` 后缀已废,名字随之;伞 crate 路径
  `svelte_rs::compiler::*` 同步变化)。迁移:改函数名即可,签名不变。
  二进制 `sv-check` → **`cargo-sv` + `check` 子命令**(即 `cargo sv check`;
  避开与 Linux runit `/usr/bin/sv` 的撞名):
  `cargo run -q -p sv-compiler --bin cargo-sv -- check [cargo check 参数]`;
  诊断尾注 `[sv-check: …]` → `[sv check: …]`,LSP/problemMatcher 的
  diagnostic source 同步为 `sv check`。`.svmap` 磁盘格式与 `__sv_*`
  内部占位符不变。

- **帧调度(ADR-6,语义 breaking)**:开窗应用的 signal 写入不再当场跑 effect,
  改为攒到帧边界由渲染壳统一冲刷(一次事件连写 N 次 = 一帧一轮)。迁移:
  需要立刻看到结果的地方调 `sv_reactive::tick()`。离屏渲染与测试路径不受影响。

- `sv_shell::caret_x` / `caret_index_at` 迁至 `text` 模块并改签名:不再收
  `&FontRef` 参数(几何由 Parley 提供)。迁移:去掉第一个参数即可。
- `sv_shell::run_app` 的错误路径改为返回 [`ShellError`],窗口/呈现层不再 panic;
  帧内 present/resize 失败降级为丢帧。
- 长文本溢出时输入框的点击定位改为与绘制同源(此前忽略横向滚移,点击会偏)。

- **动画依赖 feature 化(sv-shell,API breaking + 首发前置,2026-07-24)**:
  `sv-lottie`(Lottie 矢量)与 `sv-pag`+`image-webp`(PAG 位图序列)从硬依赖
  改为**可选**,分别置于 feature `lottie`、`pag` 之后,**默认关**。动机:velato
  系依赖重且有导入期 panic 面,多数应用不播矢量/PAG 动画,不该被强塞进默认
  依赖树;顺带解开"六 crate 首发清单与 sv-shell 硬依赖矛盾"的死结
  (optional 依赖仍须发布,故首发清单据实改为 8 crate,但默认构建不拉它们)。
  受影响的 API:`register_vector`/`unregister_vector`/`Lottie`/`LottieError`
  归 `lottie`;`register_pag`/`register_pag_webp`/`BitmapSequence`/`DecodedImage`/
  `PagFile` 归 `pag`。迁移:用到矢量/PAG 动画的应用在依赖里加
  `features = ["lottie"]` / `["pag"]`。位图帧路径(`register_frames`/`frame` /
  `AnimSource::Frames`)与 VAP 不受影响(vap-gift 示例走 `register_frames`,无需 feature)。

### 移除

- **遗留 `on:click` 事件指令(语法 breaking,2026-07-23)**:`.svelte` 事件
  只保 Svelte 5 属性形态。迁移:`on:click={h}` → `onclick={h}`(机械替换);
  写任何 `on:*` 都会得到指路属性形态的编译错误。`view!` 宏的
  `on_click(闭包)` 方法形态不受影响(本就不是 `on:` 指令)。

- `sv_shell::ui_font_handle` 与内置字体探测(`font.rs`):字体一律经 fontique
  发现。`FontHandle` 现由 `sv_shell::text` 导出,保留键 0 的语义随之消失。

<!-- 首个公开版本发布后,这里开始出现 [0.1.0] - YYYY-MM-DD 等条目 -->

---

**EN** — This project keeps a Chinese-first changelog; entries follow
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). During 0.x, **a minor
bump means breaking changes** and a patch bump is backwards compatible; every
breaking change documents its migration path above.
