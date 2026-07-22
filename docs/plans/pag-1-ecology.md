# PAG(Portable Animated Graphics)生态核实与可行性

> 核实日期:**2026-07-22**。所有版本号/日期/体积均来自当日实查(GitHub REST API、
> jsdelivr 取仓库原文、官网文档页),证据链见 §10。查不到的一律标"未核实",
> **本文不出现任何未经实查的版本号、API 名或性能数字**。
>
> 判决对象:能否把腾讯 PAG 接进 svelte-rs 的渲染栈(ADR-3/ADR-3b 的 `Painter`
> 抽象、ADR-5 的鸿蒙一等公民路线)。

---

## 0. 一句话判决

**PAG 是一套优秀的、鸿蒙官方在维护的 C++ 动效运行时,但它作为"运行期依赖"
与本仓库的 ADR-3 是正面冲突的 —— 冲突程度不低于当初被排除的 skia-safe,
在构建可离线性这一项上甚至更差。**

分档结论(详见 §8):

| 档 | 内容 |
|---|---|
| **(a) 能接** | 只有**离线烘焙**这一形态现在就能接:构建期用 PAG 官方工具把 `.pag` 转成我们能画的资产(帧序列位图 / SVG),运行期零 C++ 依赖。代价是丢掉 PAG 的全部运行期可编辑性,本质上是"用 PAG 的导出工具链",不是"接 PAG"。 |
| **(b) 先补** | 想要真正的运行期集成,前置件是:`Painter` 补 `draw_image` + 变换 + 渐变 + 混合模式 + 图层组;`<surface3d>`/`external_texture` ADR(调研 15 已挂账)定稿;鸿蒙 native 交叉编译 spike;`PAG_USE_C=ON` 的自建产物 + bindgen 封装。四件事没一件是小活。 |
| **(c) 不该做** | **不该把 libpag 放进 svelte-rs 的默认依赖树。** 理由不是"C++ 恐惧症",是四条硬伤:①构建期要 Node.js + `depsync` 联网 git clone 二十来个第三方库,cargo 生态无法 vendor/离线/发 crates.io;②tgfx 无软件光栅路径,直接废掉本仓库"三档共享 imaging model"里的 CPU 兜底档;③tgfx 自持 GPU 上下文与我们的 wgpu 抢设备,且 PAG 自带 animator 与异步渲染线程,与 ADR-6 帧调度形成双时钟;④Rust 绑定生态为零(唯一一个 2 星、2023 年停更),C API 覆盖不到矢量模型。 |

**推荐路线**:动效主线走 **Lottie + velato(纯 Rust,vello 0.9 版本已对齐)**;
PAG 降级为"**资产互通格式**"(离线转换)+ "**可选外部 surface 合成**"
(真有客户硬需求时,做成独立 feature crate,不进默认依赖树)。对照见 §7。

---

## 1. PAG 是什么

### 1.1 定位与格式

- **官方定义**(仓库描述,实查):"The official rendering library for PAG
  (Portable Animated Graphics) files that renders After Effects animations
  natively across multiple platforms."
- **二进制,不是 JSON**。这是它相对 Lottie 的第一性差异。官方 README 自述:
  同等动画下 PAG 文件比 JSON **小约 50%**、**解码快 10 倍**
  (原文 "about 50% smaller in file size" / "decode 10 times faster than JSON
  files")。这是**官方自述数字,未做第三方复现**。
- 二进制的实际收益不止体积:**单文件可内嵌图片/音频/视频资源**,交付时不用管
  一堆散图 —— Lottie 的 `.lottie`(zip 容器)是后来才补上的同类能力。

### 1.2 三种导出形态(这是理解 PAG 的钥匙)

官方文档把导出分三档(`pag.io/docs/zh-CN/ae-bmp-guide.html` 等):

1. **矢量导出** —— 文件极小、性能好、内容运行期可编辑,但**只支持 AE 特性的一个
   子集**。
2. **BMP 预合成导出** —— 支持**全部** AE 特性(含粒子、第三方插件),
   代价是文件大、**运行期不可编辑**。
   **关键工程后果:BMP 预合成内部是 H.264 视频**,所以运行期需要一个视频解码器
   (移动端走硬解;桌面/兜底走 `ffavc`(FFmpeg 壳)或 `libavc`)。
   对一个桌面 UI 库来说,这等于为了播个图标动画拖进一个 H.264 解码器。
3. **矢量 + BMP 混合导出** —— 需要运行期编辑的文本/图片层走矢量,其余走 BMP。

> **踩坑提示**:很多"PAG 支持所有 AE 特性"的说法,省略了"那是 BMP 档"这个前提。
> 矢量档的能力清单和 Lottie 是同一个量级的问题域,并没有魔法。

### 1.3 矢量档到底支持什么(官方特性表 `pag.io/zh-CN/feature`)

社区版矢量档实查覆盖:
- 图层类型:空对象 / 纯色 / 文本 / 形状 / 预合成 / 图片
- 混合模式:16 种(Normal/Multiply/Screen/Overlay/…)
- 形状层:组、矩形、椭圆、多边星形、路径、填充、描边、合并路径、中继器、
  修剪路径、圆角、渐变填充/描边
- 2D 变换:锚点、位置(含 X/Y 分离)、缩放、旋转
- 蒙版:路径蒙版 + 扩展 + 四种模式(相加/相减/相交/差值)+ 不透明度 + 羽化
- 轨道遮罩:Alpha / Alpha 反转 / Luma(v4.1+)/ Luma 反转
- 文本属性:字体族/样式、字号、颜色、描边、行距、字符间距、对齐

**企业版(付费)才有的**(实查同页):
- **3D 变换与摄像机图层**(v4.2+)
- **动感模糊、高斯模糊、置换映射、边角定位**等效果
- **文本动画器**(范围选择器 / 摆动选择器,v3.2+)

> 这一条对判决很重要:**"PAG 能力比 Lottie 强"这个印象,有相当一部分建立在
> 企业版功能上。** 开源社区版的矢量能力清单,和 velato 能画的东西不是碾压关系。

### 1.4 表达式:**不支持**

官方 FAQ 明说不原生支持 AE 表达式,理由是"要内嵌 JS 虚拟机、包体会显著变大"。
绕行方案两条:① 在 AE 里右键把表达式**转成关键帧**再导出;② 该层标记为
**BMP 预合成**导出。

> 对我们是好消息 —— 表达式是 Lottie 生态里最难啃的兼容性泥潭,PAG 直接不趟。
> 但也说明"PAG 表达力更强"不成立:它是把复杂度推给了导出期。

### 1.5 工具链

- **PAGExporter**:AE 导出插件(Win/macOS,官网提供安装与手动安装文档)。
- **PAGViewer**:桌面预览工具,支持**替换文本、填充占位图**,不上线就能看效果。
- **PAG File Format Spec**:官网 `pag.io/docs/en/pag-spec.html` 提供 **PDF 下载**。
  → 格式是**公开有规范文档**的,不是黑盒。
  (**未核实**:PDF 正文我没有逐条读,只核实了下载入口存在。)
- 仓库内还有 `cli/npm`(node 打包的命令行工具,含 `html-snapshot`)、`exporter/`、
  `viewer/`。

### 1.6 PAGX:2026 年新出的 XML 姊妹格式(重要变量)

仓库 `spec/` 目录(实查:**最早提交 2026-02-13,最新 2026-07-22,共 25 次提交**)
定义了一个全新的 **PAGX(Portable Animated Graphics XML)**:

- 规范原文:"PAGX (Portable Animated Graphics XML) is an XML-based markup
  language for describing animated vector graphics."
- 与 PAG **双向可转**:"convert to PAG for optimized loading performance during
  publishing; use PAGX format for reading and editing during development and
  review."
- 覆盖面自述:"Fully covers vector graphics, raster images, rich text, filter
  effects, blending modes, masking, and related capabilities"。
- 规范里引用了 **CSS 简写**与 **CSS Flexbox** 语义描述容器布局;仓库还有
  `spec/html_subset.md` 与近期提交 "Add HTML import pipeline with html-snapshot
  tool for browser-rendered capture (#3444)"、"Improve HTML-to-PAGX fidelity (#3596)"。
- 附带 `spec/pagx.xsd`(XML Schema)。
- 近期提交 "Remove version attribute from pagx schema (#3445)" —— **规范仍在变形期,
  连版本号字段都在增删**。CMake 里 `PAG_BUILD_PAGX` **默认 OFF**。

> **裁决相关**:PAGX 是文本 + 公开 XSD,理论上 Rust 侧写解析器是可行的
> (不像二进制 PAG 要移植 Codec)。但它今年 2 月才出生、五个月改了 25 次、
> 默认不编译、无版本号 —— **现在押它等于押一个移动靶**。
> 另外值得警惕的信号:PAG 正在从"动效格式"往"带 Flexbox 的声明式 UI 描述格式"
> 长,这意味着它未来可能与我们的 `.sv` 前端**在定位上重叠**,而不是互补。

### 1.7 许可证

- **libpag:Apache-2.0**(README 原文 "libpag 基于 Apache-2.0 协议开源";
  LICENSE.txt 里有"原署名 THL A29 Limited 现应理解为腾讯持有"的说明。
  GitHub API 的 `license` 字段报 `"Other"`,是因为 LICENSE.txt 加了这段附注
  导致自动识别失败,**不是非标准协议**)。
- **tgfx:BSD-3-Clause**(tgfx README 原文)。
- **企业版:付费授权**(官方 FAQ:社区版 Apache-2.0 可免费商用;企业版为
  "付费授权使用",在社区版核心之上叠加视频模板、素材加密、3D 图层、Movie 模块等)。
  **未核实**:企业版具体价格/授权条款,官网未公开报价。

### 1.8 活跃度(实查 GitHub API,2026-07-22)

| 指标 | Tencent/libpag | Tencent/tgfx |
|---|---|---|
| 创建 | 2020-09-22 | 2023-10-19 |
| 最近 push | **2026-07-22**(当日) | **2026-07-22**(当日) |
| star / fork | 5737 / 533 | 1567 / — |
| open issues | 217 | 3 |
| 主语言 | C++ | C++ |
| archived | false | false |

- 最新 release:**v4.5.81,发布于 2026-07-22T08:11:52Z**(当日)。
- 最新 issue/PR 编号 **#3614**(2026-07-22 创建,"Update tgfx and accept
  rendering baseline changes")。
- CI(`.github/workflows/build.yml`)覆盖 job:`ios` / `android` / `web` /
  `win` / `linux` / `qt` / **`ohos`**。

**结论:维护强度极高,是活跃的一线工业项目,不存在"停更风险"。**
这一点上它明显强于 velato(Linebender 志愿者项目,152 star)。

---

## 2. 实现形态:C++ 与依赖账单 —— 与 ADR-3 的正面对撞

### 2.1 是 C++,而且是"带自研 GPU 引擎"的 C++

- GitHub 报 `"language": "C++"`;顶层 `CMakeLists.txt` 要求
  **CMake ≥ 3.13、C++17**。
- **它不依赖 Skia。** PAG 4.0(2022-07-05 发布)把 Skia 换成了自研的
  **TGFX**。腾讯云开发者社区当期文章给的数字:Android 单架构压缩后
  **2.36MB → 0.89MB(-62.3%)**,iOS -64.8%,Web -76.0%(降到 0.72MB),
  矢量与文本渲染性能 **+60%**。
  (二手数字:另有博客称 TGFX 包体约 400KB、部分场景比 Skia 快 10 倍以上 ——
  **未在官方渠道核实,不采信**。)

### 2.2 TGFX 是什么(这决定了能不能接进 Painter)

实查 tgfx README:

- 定位:"A lightweight 2D graphics library for modern GPUs" ——
  **for modern GPUs**,这四个字是重点。
- 后端:OpenGL 3.2+ / OpenGL ES 3.0+ / WebGL 2.0+ / Vulkan 1.1+ / WebGPU 1.0 /
  Metal(README 标注 in progress)。
- **没有 CPU 软件光栅路径。** README 未列任何软件后端;Linux 上的 demo 靠
  **SwiftShader**(Google 的 CPU 版 Vulkan)顶。libpag 顶层 CMake 也确实带着
  `PAG_USE_SWIFTSHADER`(默认 OFF)与 `PAG_USE_ANGLE`(默认 OFF)两个开关。
- 头文件目录:`include/tgfx/{core,gpu,layers,pdf,platform,svg}` ——
  **没有 C API 目录**,纯 C++ 接口。
- 顺带一提:`include/tgfx/svg/` 里有 `SVGExporter.h` 与 `SVGDOM.h`,
  即 TGFX **能导出 SVG**(§8 的离线烘焙路线用得上)。

### 2.3 依赖账单(实查 `DEPS` + `vendor.json`)

**libpag 直接拉取(`DEPS`,均为固定 commit 的 git 仓库):**

| 依赖 | 用途 |
|---|---|
| `libpag/vendor_tools` | 构建自动化 |
| `libpag/tgfx` | 渲染引擎(它自己还有 13 个依赖,见下) |
| `libpag/libavc` | H.264 软解 |
| `rttrorg/rttr` | **C++ 运行期反射**(PAGX 用) |
| `harfbuzz/harfbuzz` | 整形 |
| `lz4/lz4` | 压缩 |
| `libexpat/libexpat` | XML |
| `Tehreer/SheenBidi` | 双向文本 |
| `GNOME/libxml2` | XML |
| `google/woff2` | 字体 |

**tgfx 自己再拉(`tgfx/vendor.json`):**
skcms、pathkit、zlib、libwebp、libjpeg-turbo、libpng、freetype、harfbuzz、
googletest、expat、highway、**shaderc**、**SPIRV-Cross**。

合计 **20+ 个第三方 C/C++ 库**(harfbuzz/expat 两边各一份)。

### 2.4 构建方式:这才是致命伤

- 依赖不是 git submodule、不是 vendored 源码,而是由一个 **Node.js 写的
  `depsync` 工具在构建时联网 git clone**。README:macOS 跑 `./sync_deps.sh`,
  其他平台需要 **Node.js** 然后 `npm install -g depsync && depsync`。
- 工具链要求(README 实查):CMake 3.13+、**Ninja 1.9+**、NDK 28+、
  Emscripten 3.1.58+、**Node.js 14.14+**、VS2019+ / Xcode 11+ / GCC 9+。
- Linux 额外坑(`linux/README.md` 实查):要装 **libX11-devel**,理由原文是
  "swiftshader depends on some header files";Ninja 还要"从 git 编译"、
  先装 `re2c`。

> **对 cargo 生态意味着什么**:
> - `cargo build` 不能离线跑 —— build.rs 里 shell out 到 node + git clone。
> - `cargo vendor` / `cargo package` 失效 —— 依赖不在 crate 里。
> - **不可能发到 crates.io** —— crates.io 禁止构建期下载源码的做法在实践上
>   等同于不可分发。
> - CI 每次冷构建都在拉二十个 C++ 仓库并编译。
>
> **这比 skia-safe 更糟。** skia-safe 至少是个 crates.io 上的正经 crate,
> 还提供预编译二进制下载。ADR-3 排除 skia-safe 的理由是"C++ 构建重、
> 拖累鸿蒙交叉编译"——**libpag 在这条理由上每一项都更重**。

### 2.5 产物体积(实查 v4.5.81 release assets)

| 产物 | 大小 |
|---|---|
| `libpag_4.5.81_ohos_arm64-v8a.har` | **1,654,079 B(≈1.58 MB)** |
| `libpag_4.5.81_macOS_arm64_x86_64.zip` | 8,467,963 B |
| `libpag_4.5.81_ios_arm64_x86_64_static.zip` | 9,802,462 B |
| `libpag_4.5.81_android_..._arm64v8a.aar`(3 ABI) | 7,320,789 B |
| 同上 `_noffavc`(去掉软解) | 5,539,962 B |
| `libpag_4.5.81_web.zip`(wasm) | 2,765,971 B |
| `libpag_4.5.81_include.zip` | 57,761 B |

体积本身**不算离谱**(鸿蒙 1.58MB 的 har 很克制)。
**但注意:34 个 release 产物里,没有一个是 Windows 或 Linux 的预编译库。**
桌面 Windows/Linux 只能从源码构建 —— 而 README 自己写着
"日常主要都在 macOS 平台上进行开发,**Windows 平台偶尔可能会出现编译不通过的情况**"。

> 本仓库的一等公民里 Windows 排第一。这句 README 自述值一整段风险。

---

## 3. Rust 绑定:实质为零

### 3.1 crates.io

实查 `crates.io/api/v1/crates?q=pag`:**没有任何 PAG/libpag 相关 crate**。
命中的 `pag-lexer` / `pag-parser` / `pag-compiler` 是某个"parser-lexer fusion
generator"项目,与动效无关;其余是 page/paging/pagination 的字面撞名。

### 3.2 GitHub

实查 `search/repositories?q=libpag+rust`:唯一命中
- **`colorhook/pag-rs`** —— 描述 "libpag rust binding",Rust,
  **2 star,最后 push 2023-08-27**。

三年没动、两个星。**当作不存在。**

### 3.3 自己写 FFI:C API 摸底

好消息是 libpag **有 C API**(不像 tgfx)。实查 `include/pag/c/`:

- 首次提交 **2023-10-17,"Add c api for pag. (#1779)"**;
- 该路径累计 **10 次提交**,最新一次 **2026-01-23,
  "Add pag_surface_make_from_texture C interface for creating PAGSurface from
  external textures. (#3233)"**;
- 头文件 18 个 + `ext/` 子目录:`pag_player.h`、`pag_surface.h`、`pag_file.h`、
  `pag_decoder.h`、`pag_layer.h`、`pag_image.h`、`pag_text_document.h`、
  `pag_backend_texture.h`、`pag_animator.h`、`pag_disk_cache.h` …
- `pag_types.h` 里是干净的不透明指针 + 枚举 + `PAG_EXPORT`
  (MSVC `dllexport` / GCC visibility)。**没有版本宏,也没有任何
  stable/experimental 标注。**

关键 API(逐字实查):

```c
/* pag_player.h */
pag_player* pag_player_create();
void        pag_player_set_composition(pag_player*, pag_composition*);
void        pag_player_set_surface(pag_player*, pag_surface*);
void        pag_player_set_progress(pag_player*, double);
bool        pag_player_flush(pag_player*);
bool        pag_player_flush_and_signal_semaphore(pag_player*, pag_backend_semaphore*);
int64_t     pag_player_get_duration(pag_player*);
int64_t     pag_player_get_graphics_memory(pag_player*);

/* pag_surface.h —— 只有两个! */
pag_surface* pag_surface_make_offscreen(int width, int height);
bool         pag_surface_read_pixels(pag_surface*, pag_color_type, pag_alpha_type,
                                     void* dstPixels, size_t dstRowBytes);

/* pag_c/ext/pag_surface_ext.h —— 外部纹理在这里 */
pag_surface* pag_surface_make_from_texture(pag_backend_texture*, pag_image_origin,
                                           bool forAsyncThread);
pag_surface* pag_surface_make_offscreen_double_buffered(int, int, bool tryHardware,
                                                        void* sharedContext);

/* pag_file.h —— 运行期可编辑性的入口 */
pag_file*         pag_file_load(const void* bytes, size_t length, const char* filePath);
int               pag_file_get_num_texts(pag_file*);
int               pag_file_get_num_images(pag_file*);
pag_text_document* pag_file_get_text_data(pag_file*, int editableTextIndex);
void              pag_file_replace_text(pag_file*, int, pag_text_document*);
void              pag_file_replace_image(pag_file*, int, pag_image*);

/* pag_decoder.h —— 逐帧位图,离线烘焙的抓手 */
pag_decoder* pag_decoder_create(pag_composition*, float maxFrameRate, float scale);
int          pag_decoder_get_num_frames(pag_decoder*);
bool         pag_decoder_check_frame_changed(pag_decoder*, int index);
bool         pag_decoder_read_frame(pag_decoder*, int index, void* pixels, size_t rowBytes,
                                    pag_color_type, pag_alpha_type);
```

### 3.4 自己写 FFI 的工作量与风险

**工作量**(乐观估):
- bindgen 跑 18 个头文件 + 手写安全封装(`pag_release` 统一释放、
  `!Send` 标注、错误码转 `Result`):**1–2 人周**。
- 构建集成(build.rs 驱动 cmake,`-DPAG_USE_C=ON`,四平台 × 交叉编译):
  **3–6 人周**,且是长期维护负担。

**风险**(比工作量更劝退):
1. **`PAG_USE_C` 在 CMake 里默认 `OFF`。** 官方预编译产物**大概率不导出 C 符号**
   (**未核实**:我没有下载 `.har`/`.aar` 逐符号验证)。也就是说,
   **走 C API 就必须自建**,吃满 §2.4 的全部构建代价。
2. **C API 覆盖面是"播放器"级,不是"模型"级。** `include/pag/file.h`(63 KB)
   里把整个 AE 模型公开了 —— `PathData`、`Keyframe<T>`、`Property<T>`、
   `Transform2D/3D`、`MaskData`、各种 `Effect` 子类、`ShapeLayer`/`TextLayer`/
   `PreComposeLayer`/`CameraLayer`、`VectorComposition`/`BitMapComposition`/
   `VideoComposition`、`Codec`、以及一个 `TagCode` 枚举(二进制的 tag 表)。
   **但这一整套只有 C++ 接口,C API 一个字都没暴露。**
   → **想拿矢量数据自己画,C API 这条路是堵死的。**
3. **无稳定性承诺**:10 次提交、无版本宏、无 ABI 政策。它是给"接入方图省事"用的
   薄壳,不是给下游绑定生态用的契约。
4. 生命周期语义靠 `pag_release` 手工引用计数;`forAsyncThread` 会**内部另建 GPU
   上下文 + 信号量同步** —— 这些语义要在 Rust 侧完整建模,是典型的
   "封装看着简单、用错就崩"的 FFI。

---

## 4. 鸿蒙相性:官方一等支持,但**对我们的折价很大**

### 4.1 官方支持,证据充分

| 证据 | 内容 |
|---|---|
| README 平台表 | "HarmonyOS Next 5.0.0(12)+"(与 iOS 9 / Android 5 / macOS 10.15 / Windows 7 并列) |
| release 产物 | `libpag_4.5.81_ohos_arm64-v8a.har`(1.58 MB)+ `.symbol.zip`,**每个 release 都发** |
| 分发渠道 | 可从 release 下 HAR,也可 **OHPM** 装 |
| CI | `.github/workflows/build.yml` 有独立 **`ohos` job**(runner `macos-latest`,跑 `assembleHar`) |
| 构建配置 | `ohos/libpag/build-profile.json5` 的 `externalNativeOptions` 指向 `../../CMakeLists.txt`,参数 `-DOHOS_STL=c++_static` |
| 上游引擎 | tgfx README 平台表含 **"HarmonyOS 5.0+ and OpenHarmony"**;`tgfx/vendor.json` 里 libjpeg-turbo / libpng / shaderc / SPIRV-Cross 均显式列了 `ohos` 目标 |
| 符号导出 | `ohos/libpag/export.def` 为 `{ global: *pag*; local: *; }` |

**这一项 PAG 完胜 Lottie 阵营的任何 Rust 方案。** 没有任何 Rust Lottie 库
声称支持鸿蒙。

### 4.2 但对**本仓库**折价严重

鸿蒙侧的 PAG 支持,形态是 **"给 ArkTS 应用用的 HAR 包"**:
`ohos/libpag/` 是个标准 DevEco 模块(`Index.ets`、`oh-package.json5`、
`hvigorfile.ts`、`obfuscation-rules.txt`),构建靠 **hvigor**。
它的假设是"你在写 ArkUI 应用,PAGView 挂在 XComponent 上"。

而我们(ADR-5)的形态是:**ArkTS 薄壳 → XComponent → OHNativeWindow →
EGL/GLES3,Rust 自绘整个 UI,渲染热路径零 NAPI 调用**。在这个形态里:

- 那个 HAR 里的 ArkTS 封装层**对我们没用**;我们要的是裸 `libpag.so` + C API。
- 于是回到 §3.4 的死结:**需要自建**(`-DPAG_USE_C=ON`),
  用 OHOS SDK 的 native 工具链跑 cmake。
  从 `build-profile.json5` 指向顶层 `CMakeLists.txt` 看,顶层 CMake 本身是懂
  OHOS 的,**理论上可以脱离 hvigor 单独 cmake 交叉编译**
  —— **未核实,这是必须做的第一个 spike**。
- 然后是 GPU 上下文互操作:PAG 在鸿蒙走 GLES;我们走 wgpu(OHOS 目前也只有
  GLES 后端)。要么共享 EGL context 让 PAG 画进我们的纹理
  (`pag_surface_make_from_texture` + 从 wgpu-hal 掏出 GL texture name,
  `unsafe` 且随 wgpu 版本漂移),要么各画各的再合成(§5 路线 A)。

### 4.3 与 Lottie 在鸿蒙的公平对照

**很多人会说"鸿蒙有 Lottie 啊"** —— 有,是
`OpenHarmony-TPC/lottieArkTS`(gitee,包名 `@ohos/lottie`),
纯 ArkTS 实现、**基于 ArkUI 的 Canvas 2D 上下文绘制**。

对自绘引擎来说,**它和 libpag 的 HAR 一样没用**:两者都活在 ArkUI 里,
而我们的 UI 不在 ArkUI 里。

所以鸿蒙这一项的真实对照是:

| | PAG | Lottie(velato) |
|---|---|---|
| 鸿蒙官方 native 库 | ✅ 有(需自建 C API) | ❌ 无 |
| 对**我们的自绘栈**可用 | ⚠️ 要交叉编译 C++ + GL 互操作 spike | ✅ 纯 Rust,`aarch64-unknown-linux-ohos` 是 Tier 2,rustup 直装 |
| 鸿蒙风险落在哪 | libpag 交叉编译 + EGL 共享 + 双时钟 | vello_hybrid 在 OHOS 的成熟度(ADR-3b 已入册的既有风险,**不新增**) |

> **判决:PAG 的鸿蒙优势是真的,但它是"ArkUI 应用生态"的优势。
> 换算到"Rust 自绘引擎"这个坐标系里,它从"决定性优势"缩水成
> "省掉了一个不确定性,换来两个新的"。**
> 任务书里假设的"PAG 鸿蒙官方支持而 Lottie 不是,会显著改变判决" ——
> 核实后不成立,因为两边在鸿蒙上都不能直接用,而 velato 的鸿蒙路径
> 与我们的渲染栈**共命运**(vello 通则通),PAG 的鸿蒙路径则是**另开一条战线**。

---

## 5. 渲染后端:能不能接进 `Painter`

### 5.1 事实:PAG 自带渲染器,不吐矢量

- 渲染器是 **TGFX**(§2.2),GPU-only。
- `include/pag/pag.h` 里 `PAGSurface` 的全部工厂方法(逐字实查):
  `MakeFrom(std::shared_ptr<Drawable>)`、
  `MakeFrom(const BackendRenderTarget&, ImageOrigin)`、
  `MakeFrom(const BackendTexture&, ImageOrigin, bool forAsyncThread)`、
  `MakeOffscreen(int, int)`、`MakeFrom(HardwareBufferRef)`;
  加上 `readPixels(ColorType, AlphaType, void*, size_t)`。
- **没有任何"把这一帧的路径/绘制指令交给宿主"的接口。**
  能拿到的只有:GL/VK/Metal 纹理、hardware buffer、或者 CPU 像素。

对照本仓库 `Painter`(`crates/sv-shell/src/paint.rs`)现有的六个动词
—— `fill_rounded_rect` / `stroke_rounded_rect` / `glyph_run` /
`push_clip` / `pop_clip` / `fill_path` ——
**PAG 与它在语义层级上根本不在一个平面**:
Painter 是"即时模式绘制动词",PAG 是"一个自带 GPU 上下文的完整播放器"。

### 5.2 三条接法与代价

#### 路线 A:外部纹理 / 独立 surface 合成

`pag_surface_make_from_texture(...)` 让 PAG 画进我们提供的 GL 纹理,
我们再把纹理当成一个图层合成进场景。

- **架构上有位置**:调研 15 已经为 `<surface3d>` 挖好了坑 ——
  `PainterCaps::external_texture`(注释原文:"能否合成外部 wgpu 纹理
  (`<surface3d>` 的前置;CPU 后端恒 false)")。PAG 合成就是这条通道的
  第二个消费者。
- **代价**:
  1. **硬绑 wgpu 后端**。CPU 兜底档(vello_cpu/tiny-skia)彻底拿不到 PAG。
     这直接违反 ADR-3b "三档共享同一 imaging model"。
  2. **GL/wgpu 互操作是 unsafe 深水区**。要从 `wgpu::Texture` 掏出底层 GL
     texture name(`wgpu-hal` 的 `as_hal`),桌面上 wgpu 默认走
     Vulkan/D3D12 而**不是** GL —— 也就是说桌面上要么强制 wgpu 用 GL 后端
     (性能与能力都降级),要么走 Vulkan `pag_vk_image_info` 路径
     (C API 里只有 `pag_backend_texture_get_vk_image_info` 这个**读取**函数,
     没有对应的 `create_from_vk_image_info` 构造函数 —— 即
     **C API 只能从 GL texture info 构造 backend texture**)。
     → **实查结论:C API 的外部纹理入口只有 GL 一条。**
  3. **双时钟**:PAG 有自己的 `pag_animator` 与 `forAsyncThread` 独立 GPU
     上下文/信号量;我们有 ADR-6(写入攒到帧边界、渲染壳统一冲刷)。
     两套帧时钟要在同一 surface 上对齐 —— 这正是调研 15 §3 点名的
     "3D 连续渲染 vs UI 按需渲染"难题,PAG 会让它提前引爆。
  4. 裁剪/圆角/层级混合语义要在两套系统间对齐(PAG 画在自己的纹理里,
     我们的 `push_clip` 管不到它)。

#### 路线 B:离屏 readPixels 逐帧

`pag_surface_make_offscreen` + `pag_surface_read_pixels`,拿到 RGBA 后当图片画。

- **代价**:每帧一次 GPU→CPU 回读。调研 15 已经就这个做过判决:
  "GPU→CPU readback 每帧数十 MB,不可行"。
  小尺寸图标动画(比如 64×64)勉强能扛,但仍需 TGFX 起一个 GPU 上下文
  (或 SwiftShader),依赖账单一分不少。
- **且 Painter 没有 `draw_image` 动词** —— 现在连"把一张位图画到屏幕上"
  这个能力都不存在。

#### 路线 C:离线烘焙(构建期跑,运行期零 C++)

用 `pag_decoder_*`(或官方 CLI / PAGViewer / TGFX 的 `SVGExporter`)在**构建期**
把 `.pag` 转成:
- **帧序列位图**(`pag_decoder_read_frame` + `pag_decoder_check_frame_changed`
  天然支持跳过未变帧,可做稀疏帧+差分);或
- **SVG 序列**(TGFX 有 `include/tgfx/svg/SVGExporter.h`,但
  `PAG_BUILD_SVG` 默认 OFF,需自建;**未核实**它能否导出 PAG 播放器的
  逐帧结果,只核实了 TGFX 有 SVG 导出能力)。

- **代价**:丢掉运行期可编辑性(`replace_text`/`replace_image` 全废)、
  丢掉矢量的分辨率无关性(位图路线)、资产体积上升。
- **收益**:**运行期依赖为零**,ADR-3 一寸不破,四平台+鸿蒙全通,
  CPU/GPU 两档一致。只需要给 Painter 补一个 `draw_image`。

### 5.3 `Painter` 的缺口清单(不管走哪条路都躲不掉)

想在**我们自己的栈**里画 PAG 的矢量档(或 Lottie),现有六个动词远远不够:

| 缺口 | PAG 矢量档需要 | Lottie/velato 需要 | 现状 |
|---|---|---|---|
| 位图绘制 `draw_image` | ✅(图片层/BMP 档) | ✅(image layer) | ❌ 无 |
| 仿射变换(push/pop transform) | ✅(每层 Transform2D) | ✅ | ❌ 无(所有动词吃物理像素绝对坐标) |
| 渐变填充/描边 | ✅ | ✅ | ❌ 无(Painter 只收 `Color`) |
| 混合模式(16 种) | ✅ | ✅ | ❌ 无 |
| 图层组 + 组不透明度 | ✅ | ✅ | ❌ 无(`push_clip` 只有裁剪) |
| 轨道遮罩 / Luma matte | ✅ | ✅(部分) | ❌ 无 |
| 路径描边(任意路径) | ✅ | ✅ | ❌ 只有 `stroke_rounded_rect` |
| 模糊 | 企业版 | 部分 | ⚠️ `caps.blur`,vello 侧 true,消费方未落地 |

> 这张表的含义:**"接动效"这件事的真正瓶颈,从来不是选 PAG 还是 Lottie,
> 而是 `Painter` 的动词集离"能画一个通用矢量动画"还差一整个数量级。**
> `fill_path` 只是万里长征第一步(调研 26 §6 已经把"图标管线是隐形深坑"
> 写进风险清单第一位)。

---

## 6. 国内生态权重

- 官方"他们都在使用"页(`pag.io/zh-CN/users.html`)列了 **60+ 产品**:
  微信、手机 QQ、QQ 音乐、QQ 空间、腾讯视频、腾讯新闻、腾讯地图、腾讯会议、
  王者荣耀、和平精英、英雄联盟手游、B 站、豆瓣、知乎、小红书、爱奇艺、
  虎牙、京东金融、微众银行、58、同程旅行 …
- README.zh_CN 自述:"PAG 方案目前已经接入了腾讯系几乎所有主流应用以及外部
  几千个业务"。腾讯新闻 2023-04 的官方稿口径是 **600+ 业务**。
  (**未核实**:"覆盖 40 亿设备"一类数字,官方页面未见。)
- 典型重场景:微信视频号直播的**全部礼物动效**、王者荣耀/和平精英的战报高光、
  广告视频模板批量生成。

**结论:在中国大陆移动端,PAG 的采用面是压倒性的,Lottie 在这个市场是二线。**
但两点必须说清:
1. 这个采用面几乎**全在移动端 App(Android/iOS)与 Web**,
   **桌面(Windows/Linux)几乎没有先例**(release 连预编译都不发)。
2. 采用面是"设计师产能"的护城河,不是"技术接入成本低"的证据。
   我们要付的成本在 §2.4,和微信有多少业务用 PAG 无关。

---

## 7. PAG × Lottie 对照表

> Lottie 侧取**对本仓库真正可用的 Rust 实现**为代表,而不是抽象的"Lottie 格式"。
> 主选 **velato**(Linebender,Lottie→vello),备选 **rlottie**(Samsung,C++)。

| 维度 | **PAG(libpag)** | **Lottie / velato**(纯 Rust) | Lottie / rlottie(C++,参照组) |
|---|---|---|---|
| **格式** | 自研二进制(公开 PDF 规范);另有 2026-02 起的 XML 姊妹格式 PAGX(仍在变形) | JSON 开放标准,Lottie Animation Community 治理(Joint Development Foundation 项目),IANA 已登记 `video/lottie+json` | 同左 |
| **能力(矢量档)** | 形状/蒙版/轨道遮罩/16 混合模式/文本属性;**3D、摄像机、模糊、文本动画器 = 企业版付费**;**不支持表达式** | 覆盖 Lottie 大部分形状/变换/蒙版;**明确缺:文本渲染、图片、位置关键帧缓动(`ti`/`to`)、时间重映射、dash/zigzag、动感模糊、投影、split rotation** | 覆盖面广(含文本/图片),成熟度高 |
| **能力(全量档)** | **BMP 预合成支持全部 AE 特性**(含第三方插件)—— 但代价是内嵌 H.264,运行期要视频解码器,且不可编辑 | 无对应机制 | 无对应机制 |
| **运行期可编辑** | ✅ **强项**:`replace_text` / `replace_image` / 可编辑层索引,官方工具链(PAGViewer)配套 | ⚠️ 需自己改 JSON 模型;velato 无文本渲染 | ⚠️ 有 property override,弱于 PAG |
| **依赖账单** | ❌ C++17 + tgfx + **20+ 第三方 C/C++ 库**;构建期 **Node.js + depsync 联网 git clone**;CMake+Ninja+NDK28+ | ✅ **零 C/C++**:`serde`/`serde_json`/`kurbo`/`peniko`/`vello`(可选) | ⚠️ 单体 C++,CMake,无外部依赖(相对干净) |
| **能否离线/vendor/发 crates.io** | ❌ 全否 | ✅ 全可 | ⚠️ `rlottie` crate 存在(0.5.4,2026-03-07),但要编 C++ |
| **渲染后端** | ❌ 自持 TGFX(GPU-only,**无软件光栅**,无 GPU 靠 SwiftShader/ANGLE);不吐矢量给宿主 | ✅ **输出 vello `Scene`**,由我们的后端画;CPU 档理论上可经 vello_cpu 同源 | ⚠️ 自带 CPU 光栅器,吐 ARGB 位图 |
| **与 `Painter` 抽象的关系** | ❌ 平面不对齐,只能走外部纹理/回读/离线烘焙 | ⚠️ 走 vello 词汇,与 ADR-3b "词汇对齐 vello Scene" **同源**;但 Painter 需补变换/渐变/混合/图层 | ⚠️ 只能当位图源(需 `draw_image`) |
| **版本对齐** | — | ✅ **velato 0.11.0(2026-07-21)依赖 vello 0.9.0 / kurbo 0.13.0 / peniko 0.6.0;本仓库 Cargo.lock 是 vello 0.9.0 / kurbo 0.13.1 / peniko 0.6.1 —— 完全对齐,MSRV 1.88** | — |
| **鸿蒙** | ✅ **官方一等**:每 release 发 `.har`(1.58MB)、OHPM 分发、CI 有 ohos job、tgfx 显式支持 OpenHarmony。⚠️ 但形态是 ArkTS HAR,自绘栈要自建 C API + GL 互操作(**未核实可行性**) | ⚠️ 无鸿蒙专门支持,**但纯 Rust,`aarch64-unknown-linux-ohos` Tier 2 直装;鸿蒙能力 = vello_hybrid 的能力(ADR-3b 既有风险,不新增战线)** | ❌ 要自己交叉编译 C++;ArkTS 侧有 `@ohos/lottie`(Canvas2D,对自绘栈无用) |
| **维护活跃度** | ✅ **极强**:5737★,2026-07-22 当日 push,当日发 v4.5.81,CI 覆盖 7 平台 | ⚠️ 中:152★,2026-07-21 push,Linebender 官方项目但功能缺口自认明确 | ⚠️ 中:1428★,近三次提交(2026-07-22 / 07-03 / 06-05)**全是安全/崩溃修复**,呈维护模式 |
| **许可证** | Apache-2.0(社区版);tgfx BSD-3-Clause;**企业版付费授权** | Apache-2.0 OR MIT(双授权,Rust 生态惯例) | LGPLv2.1 / 见仓库(GitHub 报 "Other") |
| **Rust 绑定** | ❌ crates.io **零**;GitHub 唯一 `colorhook/pag-rs`(2★,2023-08 停更) | ✅ 它**就是** Rust | ⚠️ `rlottie` crate 0.5.4(2026-03-07,近期下载 6999) |
| **国内采用面** | ✅ 压倒性(微信/QQ/王者/B 站/小红书…600+ 业务),但**几乎全在移动端** | ⚠️ 国际主流,国内二线 | ⚠️ Telegram 贴纸等 |
| **接入成本(到本仓库)** | ❌ **高**:自建 C++ 交叉编译 ×4 平台 + FFI + GL 互操作 + 双时钟对齐 ≈ 数月,且永久维护负担 | ✅ **低**:加一个 crate 依赖;真正成本在 Painter 动词补齐(与走哪条路无关) | ⚠️ 中:C++ 构建 + `draw_image` |

---

## 8. 分档判决

### (a) 能接 —— 怎么接

**唯一现在就能接、且不破 ADR-3 的形态:离线烘焙(§5.2 路线 C)。**

形态:
1. 新增一个**构建期工具**(不是运行期 crate),调用官方 PAGViewer / CLI /
   自建的 `PAG_USE_C=ON` 小程序,把 `.pag` 转成中间资产;
2. 中间资产两选一:
   - **帧序列位图**(`pag_decoder_read_frame` + `check_frame_changed` 去重帧),
     运行期只需 Painter 加一个 `draw_image`;
   - **SVG 序列 / 路径序列**,运行期走已有的 `fill_path`
     (代价:要补变换/渐变/混合,见 §5.3;且 TGFX SVG 导出可用性**未核实**)。
3. 工具不进 `Cargo.toml` 依赖树,不进 CI 必经路径,产物入库或由用户本地生成。

**这条路的诚实描述**:它不是"svelte-rs 支持 PAG",是
"svelte-rs 能吃 PAG 导出的资产"。运行期可编辑性(PAG 最大的卖点)全丢。
但它换来的是:零 C++ 运行期依赖、CPU/GPU 两档一致、鸿蒙无新增风险。

**先决条件只有一个**:`Painter::draw_image`(外加位图缓存与 HiDPI 采样策略)。
这个动词无论如何都要加(调研 26 的图标管线、`<img>` 元素都要它),
**不是 PAG 专属成本**。

### (b) 要先补什么(想做真集成的话)

按依赖顺序:

1. **spike-0(必做,1–3 天):鸿蒙裸 cmake 交叉编译**。
   验证能否脱离 hvigor,用 OHOS SDK 的 native 工具链
   (`ohos.toolchain.cmake`)+ `-DPAG_USE_C=ON -DOHOS_STL=c++_static`
   直接产出带 C 符号的 `libpag.so`。
   **这个 spike 失败 = 鸿蒙路线直接死,后面都不用看。**
2. **spike-1(1 周):Windows 源码构建**。README 自承"Windows 偶尔编译不过";
   我们的一等公民第一位是 Windows。在 `%TEMP%` 建临时项目实测
   `depsync` + cmake + MSVC,记录冷构建耗时与失败率。
3. **`Painter` 动词补齐 ADR**(§5.3 那张表):
   `draw_image` → `push_transform/pop_transform` → 渐变 → 混合模式 → 图层组。
   这是 R3/R4 的大件,**独立于 PAG 决策**,应该单独立项。
4. **`<surface3d>` / `external_texture` ADR 定稿**(调研 15 已挂账)。
   PAG 的外部纹理合成是这条通道的消费者之一,不该反过来由 PAG 驱动设计。
5. **帧调度协同**:ADR-6 的"写入攒到帧边界"要怎么和 PAG 的 `pag_animator` /
   `forAsyncThread` 独立 GPU 上下文共存。建议:**禁用 PAG 自带 animator**,
   由我们每帧 `pag_player_set_progress` + `pag_player_flush`,单时钟。
6. **FFI 封装 crate**(`sv-pag`,独立仓/独立 crate,**optional dependency**,
   默认关 —— 与 `backend-vello` 同样的待遇)。

### (c) 不该做,以及为什么

**不该:把 libpag 编进 svelte-rs 的默认依赖树。**

四条硬伤,按杀伤力:

1. **构建可离线性(致命)**。`depsync` 在构建期用 Node.js 联网 clone
   20+ 个 C++ 仓库。这让 `cargo vendor` / 离线构建 / crates.io 发布
   **全部失效**。ADR-3 排除 skia-safe 的理由是"C++ 构建重、拖累鸿蒙交叉编译"
   —— libpag 在这条上**严格更差**:skia-safe 至少是个能 vendor 的 crate。
   > 一句话:**我们排除过一个能发到 crates.io 的 C++ 绑定,没有理由接受一个
   > 连 crate 都不存在、还要在构建期上网的。**
2. **废掉 CPU 兜底档**。TGFX 是 GPU-only。ADR-3b 的三档
   (vello classic / vello_hybrid / vello_cpu)"共享同一 imaging model"
   在 PAG 面前不成立:CPU 档要么没有 PAG,要么拖进 SwiftShader
   —— 后者是又一个 Google 级 C++ 巨物。
3. **两个渲染系统抢 GPU 与抢时钟**。TGFX 自持上下文,与 wgpu 的设备/队列
   互操作只能靠 GL 后端(C API 的外部纹理入口**只有 GL**,实查确认),
   而桌面 wgpu 默认不走 GL。再叠加 PAG 自己的 animator/异步线程 vs ADR-6,
   是"两个引擎在一块 surface 上跳双人舞"。
4. **绑定生态为零 + C API 摸不到矢量**。没有可依赖的上游绑定,
   C API 无稳定性承诺、默认不编译、且**只覆盖播放器不覆盖模型**
   (`file.h` 的 `PathData`/`Keyframe`/`Codec` 全在 C++ 侧)。
   我们会成为唯一的维护者。

**同样不该:为了 PAG 去做纯 Rust 的 `.pag` 解码器。**
虽然 `include/pag/file.h`(63KB,含 `TagCode` 枚举)+ 官网 PDF 规范让"移植
Codec"在**解析层**是可行的(有界工作量),但解析完之后要自己实现
蒙版/轨道遮罩/16 种混合/效果链/BMP 档的 H.264 —— 那是重写 libpag。
**投入产出比远不如把同样的人力投到 velato 的缺口(文本渲染、图片层)上**,
后者还能顺带把整个 vello 生态的 Lottie 支持推上去。

---

## 9. 建议的落子

1. **动效主线定为 Lottie**,实现选 **velato**。理由:纯 Rust、
   版本与本仓库 vello 0.9 / kurbo 0.13 / peniko 0.6 **实测对齐**、
   鸿蒙与渲染栈共命运、Apache-2.0 OR MIT、可 vendor 可离线可发布。
   已知缺口(文本、图片、部分缓动)写进风险册,必要时我们上游贡献。
2. **PAG 定位为"资产互通"**:提供 `.pag → 帧序列/路径序列` 的**构建期**转换
   配方(文档级,不进依赖树),让已有 PAG 素材的团队能用。
3. **真集成留作可选支线**:若出现明确商业需求(客户已有大量 PAG 素材且需要
   运行期换文案/换图),再按 §8(b) 的顺序做,产物是 `sv-pag` 这个
   **默认关闭的 optional crate**,并且必须挂在 `<surface3d>`/
   `external_texture` ADR 之后,不得反向绑架渲染架构。
4. **无论如何都要做的事**(与本判决无关,但被这次调研照出来了):
   `Painter` 的 `draw_image` / 变换 / 渐变 / 混合模式 / 图层组,
   是"任何矢量动画方案"的公共前置。建议单独立一个
   "Painter 动词补齐" ADR,别让它被动效选型绑架。
5. **持续观察 PAGX**。它是文本格式 + 公开 XSD + HTML 导入管线,
   如果两年后稳定下来,Rust 侧写一个 PAGX 解析器 → 我们自己的场景树,
   是**绕开 libpag 全部 C++ 代价**的唯一体面路径。现在太早
   (2026-02 出生、五个月 25 次提交、版本号字段刚被删掉、默认不编译)。

---

## 10. 证据附录(2026-07-22 实查)

**GitHub REST API**
- `api.github.com/repos/Tencent/libpag` → C++,5737★,533 fork,217 open issues,
  created 2020-09-22,pushed **2026-07-22T10:07:13Z**,archived false。
- `api.github.com/repos/Tencent/libpag/releases/latest` → **v4.5.81**,
  published **2026-07-22T08:11:52Z**,34 个 asset(体积见 §2.5;
  **无 Windows / Linux 预编译**)。
- `api.github.com/repos/Tencent/libpag/commits?path=include/pag/c` → 10 commits,
  最早 **2023-10-17 "Add c api for pag. (#1779)"**,
  最新 **2026-01-23 "Add pag_surface_make_from_texture … (#3233)"**。
- `api.github.com/repos/Tencent/libpag/commits?path=spec` → 25 commits,
  **2026-02-13 → 2026-07-22**。
- `api.github.com/repos/Tencent/libpag/contents/{,include/pag,include/pag/c,include/pag/c/ext,ohos,ohos/libpag,linux,spec,cli,.github/workflows}` → 目录清单。
- `api.github.com/repos/Tencent/tgfx` → C++,1567★,created 2023-10-19,
  pushed 2026-07-22T07:24:51Z,open issues 3。
- `api.github.com/repos/Tencent/tgfx/contents/include/tgfx` →
  `core / gpu / layers / pdf / platform / svg`(**无 C API 目录**);
  `…/include/tgfx/svg` → `SVGExporter.h`、`SVGDOM.h` 等。
- `api.github.com/repos/linebender/velato` → Rust,152★,Apache-2.0,
  pushed 2026-07-21T12:28:49Z。
- `api.github.com/repos/Samsung/rlottie` → C++,1428★,pushed 2026-07-22,
  最近三次提交(2026-07-22 / 2026-07-03 / 2026-06-05)全为安全/崩溃修复。
- `api.github.com/search/repositories?q=libpag+rust` → 唯一命中
  `colorhook/pag-rs`(2★,pushed 2023-08-27)。

**仓库原文(经 cdn.jsdelivr.net 取 `@main`)**
- `libpag/README.md` / `README.zh_CN.md` → 平台表、Apache-2.0、depsync、
  "Windows 偶尔编译不过"、"50% smaller / decode 10× faster"、
  "接入了腾讯系几乎所有主流应用以及外部几千个业务"。
- `libpag/CMakeLists.txt` → CMake ≥3.13、C++17;
  `PAG_USE_OPENGL=ON`、`PAG_USE_SWIFTSHADER=OFF`、`PAG_USE_ANGLE=OFF`、
  **`PAG_USE_C=OFF`**、`PAG_USE_RTTR=OFF`、`PAG_BUILD_PAGX=OFF`、
  `PAG_BUILD_SVG=OFF`、`PAG_BUILD_CLI=OFF`、`PAG_USE_FFAVC`(Android 默认 ON)。
- `libpag/DEPS` → vendor_tools / tgfx / libavc / rttr / harfbuzz / lz4 /
  expat / SheenBidi / libxml2 / woff2(均固定 commit)。
- `libpag/vendor.json` → libavc、harfbuzz、rttr、expat、SheenBidi、libxml2。
- `tgfx/vendor.json` → skcms、pathkit、zlib、libwebp、libjpeg-turbo、libpng、
  freetype、harfbuzz、googletest、expat、highway、shaderc、SPIRV-Cross
  (多项显式列 `ohos` 目标)。
- `tgfx/README.md` → "for modern GPUs";平台含 **HarmonyOS 5.0+ 与 OpenHarmony**;
  后端 GL/GLES/WebGL/Vulkan/WebGPU/Metal(in progress);**无软件光栅**;
  BSD-3-Clause。
- `libpag/linux/README.md` → 需 libX11-devel("swiftshader depends on some
  header files")、Node.js、从 git 编译 Ninja。
- `libpag/include/pag/pag.h` → `PAGSurface::MakeFrom(Drawable / BackendRenderTarget
  / BackendTexture / HardwareBufferRef)`、`MakeOffscreen`、`readPixels`;
  **无矢量数据导出接口**。
- `libpag/include/pag/file.h`(63,322 B)→ `TagCode`、`PathData`、`Keyframe<T>`、
  `Property<T>`、`Transform2D/3D`、`MaskData`、各 `Effect` 子类、各 `Layer` 子类、
  `VectorComposition`/`BitMapComposition`/`VideoComposition`、`Codec`
  —— **仅 C++**。
- `libpag/include/pag/c/{pag_types,pag_player,pag_surface,pag_file,pag_decoder,
  pag_backend_texture,ext/pag_surface_ext}.h` → §3.3 逐字签名。
- `libpag/ohos/libpag/build-profile.json5` → `externalNativeOptions.path =
  "../../CMakeLists.txt"`,`arguments = "-DOHOS_STL=c++_static"`。
- `libpag/ohos/libpag/export.def` → `{ global: *pag*; local: *; }`。
- `libpag/.github/workflows/build.yml` → jobs:ios/android/web/win/linux/qt/**ohos**。
- `linebender/velato/README.md` → 未支持项:位置关键帧缓动(`ti`/`to`)、
  时间重映射(`tm`)、**文本渲染**、**图片**、stroke dash/zig-zag、
  动感模糊/投影、color stop、split rotations;MSRV **1.88**;Apache-2.0 OR MIT。
- `linebender/velato/Cargo.toml` → version 0.11.0;**vello 0.9.0 /
  kurbo 0.13.0 / peniko 0.6.0**。

**crates.io API**
- `?q=pag` → **无任何 PAG/libpag 相关 crate**。
- `?q=lottie` → velato 0.11.0(2026-07-21)、rlottie 0.5.4(2026-03-07)、
  rasterlottie 0.2.1(2026-04-24)、lottieconv 0.3.1(2026-02-19)、
  dotlottie-rs 0.1.0-alpha.1(2024-09-18)等。

**官方文档**
- `pag.io/` · `pag.io/docs/zh-CN/home.html` · `pag.io/zh-CN/feature`
  (AE 特性支持表,含企业版标注)· `pag.io/zh-CN/users.html`(用户墙)
  · `pag.io/docs/en/pag-spec.html`(**PAG File Format Spec PDF 下载入口**)
  · `pag.io/docs/install-PAGExporter.html` · `pag.io/docs/pag-edit.html`
  · `pag.io/docs/zh-CN/ae-bmp-guide.html` · `pag.io/docs/zh-CN/faq.html`。
- `cloud.tencent.com/developer/article/2040049`(2022-07-05,PAG 4.0 发布):
  Skia→TGFX;Android 单架构压缩 2.36MB→0.89MB(-62.3%)、iOS -64.8%、
  Web -76.0%(0.72MB);矢量与文本渲染 +60%。
- `lottie.github.io`:Lottie Animation Community,Joint Development Foundation
  治理;IANA 已登记 `.lot` / `video/lottie+json`。
- `gitee.com/openharmony-tpc/lottieArkTS`(包名 `@ohos/lottie`):
  ArkTS + Canvas 2D 实现,API 9+。
- `github.com/libpag/ffavc`:"A video decoder built on ffmpeg which allows
  libpag to use ffmpeg as its software decoder for h264 decoding."
- `github.com/libpag/libavc`:"A H.264 software decoder under apache v2.0 license."

**本仓库交叉引用**
- `crates/sv-shell/src/paint.rs` L102–147:`PainterCaps { external_texture, blur }`
  与六个动词(`fill_rounded_rect` / `stroke_rounded_rect` / `glyph_run` /
  `push_clip` / `pop_clip` / `fill_path`)。
- `Cargo.lock`:vello 0.9.0、wgpu 29.0.4、kurbo 0.13.1、peniko 0.6.1。
- `docs/DESIGN.md` ADR-3 / ADR-3b / ADR-5 / ADR-6;
  `docs/research/15-scenario-analysis.md` §3(`<surface3d>` 外部纹理节点、
  GPU→CPU 回读不可行、3D 与 UI 双时钟);
  `docs/research/26-arco-design-ui-kit.md` §6(图标管线为头号风险)。

## 11. 明确未核实的事项

1. **PAG File Format Spec PDF 的正文内容**(只核实了下载入口存在)。
2. **官方预编译产物(`.har` / `.aar` / macOS zip)是否导出 C API 符号**
   —— `PAG_USE_C` 默认 OFF,我未下载产物做 `nm`/`objdump` 验证。
3. **能否脱离 hvigor,用 OHOS SDK native 工具链单独 cmake 交叉编译 libpag**
   —— 仅由 `build-profile.json5` 指向顶层 CMakeLists 推断,未实测(§8(b) spike-0)。
4. **Windows 源码构建的实际成功率与冷构建耗时** —— 未实测(spike-1)。
5. **TGFX 的 `SVGExporter` 能否导出 PAG 播放器的逐帧结果** ——
   只核实了 TGFX 具备 SVG 导出能力,未验证与 PAG 播放链路的贯通。
6. **企业版价格与授权条款** —— 官网未公开报价。
7. **"TGFX 约 400KB 包体"、"部分场景比 Skia 快 10 倍"** ——
   仅见于第三方博客,未在腾讯官方渠道核实,本文不采信。
8. **PAG 在鸿蒙真机上的性能基线** —— 无第一手数据。
9. **wgpu(OHOS GLES 后端)与 libpag/TGFX 共享 EGL context 的可行性** ——
   纯推理,未实测。
