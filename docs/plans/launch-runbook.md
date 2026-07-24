# 首发 runbook（crates.io 0.1.0 · form B）

> 目的：让第一次 crates.io 发布**没有意外**。这里把发布链、依赖序、以及本机环境的
> 几个真实陷阱钉成一张可执行清单。依据 = 2026-07-24 发布就绪度审计（调研 28 §首发、
> `docs/plans/open-issues.md` R4）。**执行前从上到下过一遍。**

## 0. 前置状态（已就位）

- 版本 = **0.1.0**（根 `Cargo.toml` workspace.package + 11 条 dep 约束 lockstep；ADR-10
  要求以真实 0.1.0 占名，crates.io 反对 0.0.x 空壳）。改版本只改根 `Cargo.toml` 一处，
  子 crate 全走 `version.workspace = true`。
- 元数据齐全：8 个可发布 crate 的 description/license/repository/homepage/readme/
  keywords（5，上限）/categories（`gui`/`rendering`，合法 slug）/rust-version（1.88）
  均在（多数经 workspace 继承）。readme 文件真实存在。
- 误发护栏：`sv-lsp` / `sv-vap` / `sv-arco` / `sv-arco-tokens` 均 `publish = false`
  （不在首发依赖闭包）。
- 伞 crate 依赖期票已订正（`svelte-rs/src/lib.rs` + README）：**不是"一条依赖拿全套"**
  —— `view!`/`.svelte` 生成代码发绝对路径 `::sv_ui`/`::sv_reactive`，消费者须直依
  `sv-ui` + `sv-reactive`（`.svelte` 另需 `sv-compiler` 作 build-dep）。

## 1. 🔴 本机发布陷阱（必读，否则误发/失败）

1. **默认 registry 是 rsproxy 镜像，不是 crates.io**。用户全局 `~/.cargo/config` 设了
   `[source.crates-io] replace-with = 'rsproxy-sparse'` 且 `[registry] default = rsproxy`。
   **本机手动 `cargo publish` 不显式指定会误发到只读镜像**。发布必须：
   ```sh
   cargo publish -p <crate> --registry crates-io
   ```
   （或临时在干净 shell 里 `unset` 那两项 / 用 CI 的 GitHub runner 干净配置发布。）
2. **网络间歇**：本机 github.com:443 时通时断（api.github.com 较稳）；crates.io 上传前
   确认 `cargo login` 令牌就位、网络窗口内速战。

## 2. 依赖序（拓扑，逐个发、等索引可见再发下一个）

```
sv-reactive → sv-ui → sv-compiler → sv-macro → sv-lottie → sv-pag → sv-shell → svelte-rs
```
共 **8 crate**。sv-lottie/sv-pag 是 sv-shell 的 **optional** 依赖，但 crates.io 要求
optional 依赖也已发布，故必须在 sv-shell 之前发（已在序内）。每发一个，等 crates.io
索引可见（通常几十秒）再发下一个，否则下一个解析不到。

## 3. 发布前全链验证（唯一可靠办法 = 本地 registry）

`cargo package --no-verify` 逐个**不可行**（实测证伪）：只有 3 个叶子（sv-reactive/
sv-compiler/sv-pag）能单独打包，其余 5 个报 `no matching package named '…' found`
——`cargo package` 即便 `--no-verify` 也要解析依赖图，未发布的内部 dep 直接挡住。

**可靠办法**——本地 registry 走一遍完整发布代码路径：
```sh
cargo install cargo-local-registry            # rsproxy 可达
mkdir -p /tmp/svreg
# 按依赖序逐个打包灌入本地 registry，--sync 把外部依赖(winit/syn/vello…)也同步进去
cargo local-registry --sync Cargo.lock /tmp/svreg
# 在仓库级 .cargo/config.toml 用 replace-with 指向本地 dir（临时压过 rsproxy），
# 然后按序 cargo publish --registry local（或去掉 --no-verify 的 cargo package），
# 让每个 stripped 包真 verify-build —— 能同时抓"打包漏文件""path-only 漏 version"类问题。
```
最高保真 = 支持 publish 的本地 registry + 按序 `cargo publish --registry local`，走与
crates.io 完全一致的 verify build。

**2026-07-24 实测状态**：
- **本机装不了 `cargo-local-registry`**：`cargo install` 走 rsproxy **git** 索引
  （`https://rsproxy.cn/crates.io-index`）返回 404（本机已知问题）。全链 verify-build
  在本机受阻，须在**干净环境**（全局无 rsproxy 替换）或 **CI runner** 上跑。实际发布前
  **必须**在那种环境跑一次并把结果记在此节。
- **能在本机做的 registry-free 检查（已跑，全过）**：① 8 个可发布 crate **均无
  `include`/`exclude`**（默认打包 = src/ + Cargo.toml + README，低漏文件风险）；
  ② 全部 `src/lib.rs` 就位；③ 元数据齐全（§0）；④ 依赖全带 version、无 path-only 漏；
  ⑤ 叶子（sv-reactive/sv-compiler/sv-pag）`cargo package` 真打包过；⑥ `cargo build
  --workspace` 通过。
- **残余风险 = 低**：唯一未验的是非叶子 verify-build（deps 从 registry 取而非 path），
  但 path 版与 registry 版**源码相同**、构建结果应一致；publish 专有失败面（元数据/漏
  version/漏文件）已逐条排除。CI 只对叶子 sv-reactive 做真 verify-build，非全链——
  补全链是发布前剩下唯一的信心缺口。

## 4. 已知非阻塞项（可随首发或紧接补）

- **tarball 不含 LICENSE 文本**：SPDX `license` 字段不会把根 `LICENSE-MIT`/`LICENSE-APACHE`
  打进 `.crate`（实测 sv-reactive 打包只含 6 文件、无许可证文本）。crates.io 元数据层已声明
  双许可、能上传，但下游 vendoring/审计会缺。补法：各 crate 目录放 LICENSE-*（软链/复制根文件）。
- **社区文件缺**：`.github/` 只有 workflows。缺 CONTRIBUTING / SECURITY.md /
  CODE_OF_CONDUCT / issue+PR 模板 / CODEOWNERS / dependabot。公开首发门面项。
- **伞 crate 无 `[features]`**：无法经 `svelte-rs` 转发 `lottie`/`pag`/`backend-vello`
  到 sv-shell。想要动画/GPU 后端须直依 sv-shell 开 feature（叙事应写成"默认 CPU 全套 +
  动画/GPU 直依 sv-shell"）。
- **商标**：ADR-10 记为维护者知情承担（"Svelte"名，商标粗查未做），非工程门。

## 5. 发布后

- 接 `cargo-semver-checks` 基线（需已发布版本才有意义）。
- 首发公告如实标注：$state 表面语法未冻结（最大悬置 breaking）、0.x minor=breaking 政策。
