# sv-lsp

`.svelte` 单文件组件的语言服务器(LSP),**最小可用版**。

编辑器每次打开 / 改动 `.svelte`,把全文交给 `sv_compiler::compile_sv` 编译一遍,
编译前端报的错(未知标签、非法属性、runes 改写失败、样式语法……)实时变成
波浪线(`textDocument/publishDiagnostics`)。

## 分工:与 `sv check` 的区别

- **sv-lsp**:编译**前端**能立刻算出的错,快、不落盘、每次击键都能重算 ——
  编辑期最高频的一档。textDocumentSync = Full。
- **`sv check`**(sv-compiler 的二进制):跑 `cargo check` 拿 **rustc** 的类型错,
  按 source map 搬回 `.svelte`。给 `tasks.json` 的 problemMatcher 用,一次一遍。

两者互补:LSP 管"语法/结构立即错",`sv check` 管"类型/借用等 rustc 才知道的错"。

## 现状(MVP)

已做:诊断(publishDiagnostics)、Full 文档同步、initialize/shutdown/exit 生命周期。
**未做**:补全 / 跳转 / hover(要真嵌套包络与符号表,见 `docs/plans/lsp-spike.md` §3)。

## 零外部依赖

LSP = `Content-Length` 分帧 + JSON-RPC,都手写:协议解析复用
`sv_compiler::check::json`,序列化在 `lib.rs`(~40 行)。不引 tower-lsp/tokio,
与全仓"依赖面尽量干净"一致。协议处理是纯函数 `Server::handle`,有单元测试;
传输层(stdio 分帧)在 `main.rs`。

## 接入(VS Code 等)

把 `sv-lsp` 二进制配成 `.svelte` 的 language server,走 stdio 通道即可。
