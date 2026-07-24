# sv-arco-tokens

Arco Design 设计令牌层:色板算法 + 全局 token,`sv-arco` 组件库的地基
(调研 26 §4 的 A0 波次)。纯 Rust,零依赖。

## 内容

- **色板算法**(`palette::light` / `palette::dark`):任一基准色 → 10 档梯度,
  亮/暗双模式。移植自 [`@arco-design/color`] 0.4.0(60 行 HSV 数学),数值
  行为与上游**逐字符一致**——金样就是上游仓库自带的 jest 断言原文
  (vendored 于 `assets/upstream-color-*.js`,`tools/extract-golden.mjs`
  机械提取,13 组 × 亮/暗 × 10 档 + gray 字面值)。
- **全局令牌**(`src/generated.rs`):圆角/字号/尺寸/间距/阴影/亮暗语义色 +
  14 组预置色板,转译自 [`@arco-design/web-react`] 2.66.16 的
  `global.less` / `colors.less`(vendored 于 `assets/`)。
- **双出口**:Rust 常量(`view!` 宏与运行时逻辑用)+ `CSS_ROOT_LIGHT` /
  `CSS_ROOT_DARK`(`:root { --x }` 文本,`.svelte` 的 `<style>` 用,走
  sv-compiler 编译期 `var()` 代入)。

## 转译而非手抄

```sh
# arco 升版:更新 assets/*.less → 重新生成 → diff 审查 → 测试
cargo run -p sv-arco-tokens --bin gen_tokens
cargo test -p sv-arco-tokens
```

`tests/sync.rs` 强制 `src/generated.rs` 与生成器一致;`tests/tokens.rs` 用
从 less 原文人工读出的独立样本抽查解析正确性;`tests/golden.rs` 对拍色板。
唯二手工维护的是"功能色→色板"(primary→arcoblue 等)与"语义色→灰阶"两张
接线表(less 里是 `rgb(var())` 间接层,无法机械解析),出处行号见
`src/generator.rs` 注释,升版时人工复核。

## 已知边界

- `border-radius-circle`(50%)未编:百分比圆角在本渲染栈无对应概念。
- 阴影令牌目前只是**数据**:box-shadow 渲染动词尚未落地(CSS-SUPPORT ⏳)。
- `font-weight` 令牌未编:文本栈单字重,等 fontique 字重选择接线后补。
- `line-height` 阶梯不在 global.less(组件级 less 各自定义),不在本 crate
  范围;组件落地时就近取值。
- 暗色语义色的 `fade(#fff, N%)` 编成 hex-alpha(如 `#FFFFFFE6`),alpha
  字节按 N%×255 四舍五入。

## 许可与署名

视觉规范与 design token 派生自 ByteDance **Arco Design**(MIT License,
原文见 [`LICENSE-ARCO`](./LICENSE-ARCO)):

- [`arco-design/arco-design`](https://github.com/arco-design/arco-design)
  2.66.16(commit fbf2ec0a8cc2)
- [`arco-design/color`](https://github.com/arco-design/color)
  0.4.0(commit d882db3e3e25)

本 crate 为**非官方**实现,与 ByteDance 无关联、未获其背书
(unofficial, not affiliated with or endorsed by ByteDance)。
"Arco" 名称归其权利人所有;MIT 许可不授予商标权。

[`@arco-design/color`]: https://github.com/arco-design/color
[`@arco-design/web-react`]: https://www.npmjs.com/package/@arco-design/web-react
