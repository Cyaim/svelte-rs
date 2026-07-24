// 金样提取:从 vendored 的上游原文机械提取色板断言值,生成
// tests/fixtures/golden_data.rs。手抄 260 个色值必然出错,所以不手抄。
//
//   输入:assets/upstream-color-test.js   (@arco-design/color test/index.js 原文)
//         assets/upstream-color-index.js  (@arco-design/color src/index.js 原文,
//                                          取 colorList 基准色 + gray 字面值)
//   运行:node crates/sv-arco-tokens/tools/extract-golden.mjs
//
// 脚本对提取结果做强断言(13 组 × 2 模式 × 10 档、基准色与 colorList 逐一对上),
// 任何一处对不上就报错退出,不产出半截文件。

import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const testSrc = readFileSync(join(root, 'assets/upstream-color-test.js'), 'utf8');
const indexSrc = readFileSync(join(root, 'assets/upstream-color-index.js'), 'utf8');

const PALETTES = ['red', 'orangered', 'orange', 'gold', 'yellow', 'lime', 'green',
  'cyan', 'blue', 'arcoblue', 'purple', 'pinkpurple', 'magenta'];

// colorList 基准色(交叉校验用)
const colorListMatch = indexSrc.match(/const colorList = \{([\s\S]*?)\};/);
if (!colorListMatch) throw new Error('index.js 里找不到 colorList');
const colorList = {};
for (const m of colorListMatch[1].matchAll(/(\w+):\s*'(#[0-9A-Fa-f]{6})'/g)) {
  colorList[m[1]] = m[2].toUpperCase();
}
if (Object.keys(colorList).length !== 13) throw new Error('colorList 应有 13 色');

// 逐 it 块提取
const sections = testSrc.split(/it\(\s*'/).slice(1);
const rows = [];
for (const sec of sections) {
  const name = sec.slice(0, sec.indexOf("'"));
  if (!PALETTES.includes(name)) continue;
  const base = sec.match(/generate\('(#[0-9A-Fa-f]{6})'/)?.[1]?.toUpperCase();
  if (base !== colorList[name]) {
    throw new Error(`${name}: 基准色 ${base} 与 colorList ${colorList[name]} 不符`);
  }
  const arrays = [...sec.matchAll(/\.toEqual\(\[([\s\S]*?)\]\)/g)]
    .map(m => [...m[1].matchAll(/'(#[0-9A-Fa-f]{6})'/g)].map(x => x[1].toUpperCase()))
    .filter(a => a.length > 0);
  if (arrays.length !== 2 || arrays.some(a => a.length !== 10)) {
    throw new Error(`${name}: 期望亮/暗两组各 10 档,拿到 ${arrays.map(a => a.length)}`);
  }
  rows.push({ name, base, light: arrays[0], dark: arrays[1] });
}
if (rows.length !== 13 || rows.some((r, i) => r.name !== PALETTES[i])) {
  throw new Error(`应按序提出 13 组,实际:${rows.map(r => r.name)}`);
}

// gray 字面值(index.js 硬编码,不走算法)
const grayOf = (mode) => {
  const m = indexSrc.match(new RegExp(`presetColors\\.gray\\.${mode} = \\[([\\s\\S]*?)\\];`));
  if (!m) throw new Error(`index.js 里找不到 gray.${mode}`);
  const hex = [...m[1].matchAll(/'(#[0-9A-Fa-f]{6})'/g)].map(x => x[1].toUpperCase());
  if (hex.length !== 10) throw new Error(`gray.${mode} 应 10 档,拿到 ${hex.length}`);
  return hex;
};
const grayLight = grayOf('light');
const grayDark = grayOf('dark');

const quote = a => a.map(h => `"${h}"`).join(', ');
let out = `// 由 tools/extract-golden.mjs 从 assets/upstream-color-test.js /
// assets/upstream-color-index.js(@arco-design/color commit d882db3e3e25 原文)
// 机械提取,勿手改。重跑:node crates/sv-arco-tokens/tools/extract-golden.mjs

/// 13 组算法色板金样:(名字, 基准色, 亮 10 档, 暗 10 档)。
pub const GOLDEN_CHROMATIC: &[(&str, &str, [&str; 10], [&str; 10])] = &[
`;
for (const r of rows) {
  out += `    (\n        "${r.name}",\n        "${r.base}",\n        [${quote(r.light)}],\n        [${quote(r.dark)}],\n    ),\n`;
}
out += `];

/// gray 亮色 10 档(上游硬编码字面值,不走算法)。
pub const GOLDEN_GRAY_LIGHT: [&str; 10] = [${quote(grayLight)}];

/// gray 暗色 10 档。
pub const GOLDEN_GRAY_DARK: [&str; 10] = [${quote(grayDark)}];
`;

const dest = join(root, 'tests/fixtures/golden_data.rs');
writeFileSync(dest, out);
console.log(`已生成 ${dest}(${rows.length} 组 + gray)`);
