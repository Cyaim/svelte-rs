//! source map(生成 `.rs` → `.svelte` 位置回映)的验收。
//!
//! 分三层,越往下越贵、也越接近用户真正看到的东西:
//! 1. **纯函数**:`SourceMap::lookup` / `check::relocate` 的查表与降级
//!    (在 `src/sourcemap.rs` 与 `src/check.rs` 的 `mod tests` 里);
//! 2. **建图**:对真实 `.svelte` 建图后逐段自校验 + 完整性断言(本文件主体);
//! 3. **端到端**:临时 crate 里真跑 `cargo check`,断言诊断落在 `.svelte` 的正确行
//!    (本文件末尾,`#[ignore]`——它要现编 sv-ui/sv-reactive,分钟级)。
//!
//! 为什么第 2 层的"逐段自校验"不够、必须有完整性断言:逐段校验只检查
//! **已记录的段是否正确**,对"该记录却没记录"完全无感——provenance 被
//! `format_ident!` 丢掉的那 98 处会让它保持全绿。所以真正的验收闸是
//! `map_covers_all_reactive_reads`。

use std::path::{Path, PathBuf};

use sv_compiler::PropsRegistry;
use sv_compiler::sourcemap::{MapKind, SourceMap, byte_to_line_col};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn compile(src: &str) -> sv_compiler::Compiled {
    sv_compiler::compile_sv_mapped(src, "probe", &PropsRegistry::new(), "probe.svelte")
        .expect("fixture 应能编译")
}

const COUNTER: &str = r##"<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<view style="padding:24; gap:12">
  <text>Count: {count + "x"} · 双倍 = {double}</text>
  <button on:click={|| count += 1}>+1</button>
  <button on:click={|| count = 0}>归零</button>
</view>
"##;

/// 所有能拿到的真实 `.svelte`:金样 fixture + 仓库里的示例组件
fn all_sv_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in ["wide.svelte", "child.svelte", "parent.svelte"] {
        let p = fixtures_dir().join(name);
        out.push((
            name.to_string(),
            std::fs::read_to_string(&p).expect("读 fixture 失败"),
        ));
    }
    // examples/ 下的 .svelte 是"真实用法"的最好样本;不存在就跳过(不让本测试
    // 依赖别人名下的目录结构)
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut stack = vec![examples];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "sv")
                && let Ok(s) = std::fs::read_to_string(&p)
            {
                out.push((p.display().to_string(), s));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 建图的不变量
// ---------------------------------------------------------------------------

/// 开映射与不开映射的生成代码必须**逐字节相同**。
///
/// 这是"golden 不该变"的直接证据,也是整套机制的根基:provenance 靠给
/// `parse_str` 垫虚拟换行拿到,而 prettyplease 从 AST 打印、根本不看 span。
/// 这条一旦红,说明垫行影响了产物形状,机制要重新设计。
#[test]
fn mapped_output_is_byte_identical() {
    for (name, src) in all_sv_sources() {
        // 组件调用需要 props 注册表,这里只比对能独立编译的那些
        let Ok(plain) = sv_compiler::compile_sv(&src, "probe") else {
            continue;
        };
        let mapped = compile(&src);
        assert_eq!(plain, mapped.code, "{name}:开映射后生成代码变了");
    }
}

/// 每一段映射的两侧文本必须逐字相等 —— 这就是"锚点"的定义。
/// (soundness 断言:只保证记下来的没错,不保证该记的都记了。)
#[test]
fn map_segments_are_verbatim() {
    for (name, src) in all_sv_sources() {
        let Ok(_) = sv_compiler::compile_sv(&src, "probe") else {
            continue;
        };
        let c = compile(&src);
        assert!(!c.map.segs.is_empty(), "{name}:一段映射都没建出来");
        for s in &c.map.segs {
            let a = src.get(s.sv_start..s.sv_end).unwrap_or_else(|| {
                panic!(
                    "{name}:.svelte 区间 {}..{} 不在字符边界上",
                    s.sv_start, s.sv_end
                )
            });
            let b = c
                .code
                .get(s.gen_start..s.gen_end)
                .unwrap_or_else(|| panic!("{name}:生成区间 {}..{} 越界", s.gen_start, s.gen_end));
            assert_eq!(a, b, "{name}:映射段两侧文本不相等");
        }
    }
}

/// 查表靠二分:段必须按生成侧起点升序且互不重叠
#[test]
fn map_is_sorted_and_disjoint() {
    for (name, src) in all_sv_sources() {
        let Ok(_) = sv_compiler::compile_sv(&src, "probe") else {
            continue;
        };
        let c = compile(&src);
        for w in c.map.segs.windows(2) {
            assert!(
                w[0].gen_end <= w[1].gen_start,
                "{name}:映射段重叠 {:?} / {:?}",
                w[0],
                w[1]
            );
        }
    }
}

/// **完整性闸**:`.svelte` 里每一处反应式变量的引用都必须有映射。
///
/// 这些引用在生成代码里是被 `Rewriter` 重造出来的
/// (`x` → `x.get()`、`x = v` → `x.set(v)`、`x += v` → `x.update(…)`)。
/// 一旦重造时用了 `format_ident!`,span 就是 `Span::call_site()`
/// (fallback 下 line=1),整批 provenance 会被无声丢弃而**上面的
/// soundness 断言依旧全绿**。这条测试就是防它的。
#[test]
fn map_covers_all_reactive_reads() {
    let c = compile(COUNTER);
    let starts: Vec<usize> = c.map.segs.iter().map(|s| s.sv_start).collect();
    for name in ["count", "double"] {
        let mut found = 0usize;
        let mut at = 0usize;
        while let Some(rel) = COUNTER[at..].find(name) {
            let pos = at + rel;
            at = pos + name.len();
            // 整词才算(避免匹配到别的标识符的一部分)
            let before = COUNTER[..pos].chars().next_back();
            let after = COUNTER[at..].chars().next();
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if word(before) || word(after) {
                continue;
            }
            assert!(
                starts.contains(&pos),
                "`{name}` 在 .svelte {:?} 处没有映射段 —— provenance 被丢了",
                byte_to_line_col(COUNTER, pos)
            );
            found += 1;
        }
        assert!(found >= 2, "`{name}` 的引用数不对,测试本身写坏了");
    }
}

/// `{#each x as pat}` 的模式与 `bind:` 的目标走的是 `self.expr` 之外的
/// 旁路 parse 入口,单独守一道:它们的 token 若按胶水处理,诊断会静默降级
#[test]
fn map_covers_each_pattern_and_bind() {
    let src = std::fs::read_to_string(fixtures_dir().join("wide.svelte")).unwrap();
    let c = compile(&src);
    let mapped_at = |pos: usize| c.map.segs.iter().any(|s| s.sv_start == pos);
    // `{#each items as it (it.0)}`:模式 `it` 与 key 表达式里的 `it`
    assert!(
        mapped_at(src.find("as it (").unwrap() + 3),
        "{{#each}} 的模式绑定应有映射"
    );
    assert!(
        mapped_at(src.find("(it.0)").unwrap() + 1),
        "{{#each}} 的 key 表达式应有映射"
    );
    // `bind:checked={agree}`
    assert!(
        mapped_at(src.find("{agree}").unwrap() + 1),
        "bind:checked 的目标应有映射"
    );
}

/// prettyplease 会把长行折断(实测:一行 28 个汉字的 `create_text(...)` 折成 4 行)。
/// 映射不预测行号——输出位置是从**输出文本自己**重解析出来的,所以折行无关。
#[test]
fn map_survives_prettyplease_reflow() {
    let src = r##"<script>
let count = $state(0i32);
</script>

<view>
  <text>{format!("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十{}", count)}</text>
</view>
"##;
    let c = compile(src);
    // 触发折行的证据:生成侧那一段确实跨了多行
    let head = c.code.find("一二三").expect("生成代码里应有那串汉字");
    let tail = c
        .code
        .find("count.get()")
        .expect("生成代码里应有 count.get()");
    assert!(
        c.code[head..tail].contains('\n'),
        "这个用例没有触发 prettyplease 折行,测试失去意义"
    );

    let pos = src.rfind("count").unwrap();
    let seg = c
        .map
        .segs
        .iter()
        .find(|s| s.sv_start == pos)
        .expect("折行之后 count 仍应有精确映射");
    assert_eq!(&c.code[seg.gen_start..seg.gen_end], "count");
    assert_eq!(byte_to_line_col(src, seg.sv_start), (6, 54));
}

/// **覆盖率地板**:`.svelte` 里用户写的 Rust token 有多大比例拿到了精确映射。
///
/// 阈值不是拍脑袋定的:2026-07-22 在仓库全部 10 个可独立编译的 `.svelte` 上实测
/// **281/349 = 80.5%**(wide 84% / InputDemo 86% / Counter 76% / Card 57%)。
/// 地板取 70%,留出加新语法时的余量;掉到地板以下说明某个 parse 入口的
/// provenance 断了,而 `map_segments_are_verbatim` 那种 soundness 断言
/// **抓不到这种"该记却没记"**。
///
/// 剩下的 ~20% 缺口是已知的:`{#snippet}` 参数类型没有 `.svelte` 偏移可用、
/// 样式值编译期折叠成字面量、`{@render}` / 块头部的部分 token。
#[test]
fn map_coverage_floor() {
    let (mut total, mut covered) = (0usize, 0usize);
    let mut worst: Vec<String> = Vec::new();
    for (name, src) in all_sv_sources() {
        if sv_compiler::compile_sv(&src, "probe").is_err() {
            continue;
        }
        let c = compile(&src);
        let starts: std::collections::HashSet<usize> =
            c.map.segs.iter().map(|s| s.sv_start).collect();
        let (mut n, mut k) = (0usize, 0usize);
        for (base, text) in user_rust_regions(&src) {
            let Ok(ts) = <proc_macro2::TokenStream as std::str::FromStr>::from_str(&text) else {
                continue;
            };
            for off in anchor_offsets(ts) {
                n += 1;
                k += usize::from(starts.contains(&(base + off)));
            }
        }
        total += n;
        covered += k;
        worst.push(format!("{name} {k}/{n}"));
    }
    let pct = 100.0 * covered as f64 / total as f64;
    assert!(
        pct >= 70.0,
        "映射覆盖率跌到 {pct:.1}%(基线 80.5%),明细: {}",
        worst.join(" | ")
    );
}

/// 粗略切出 `.svelte` 里的"用户 Rust 文本":script 块 + 模板里的 `{...}`。
/// 刻意粗糙——它只服务覆盖率这一个统计量,不参与任何正确性判断。
fn user_rust_regions(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    if let Some(a) = src.find("<script>") {
        let s = a + "<script>".len();
        if let Some(b) = src[s..].find("</script>") {
            // rune 的 `$` 不是合法 Rust token,换成 `_` 保持字节长度
            out.push((s, src[s..s + b].replace('$', "_")));
        }
    }
    let tpl_start = src.find("</script>").map_or(0, |i| i + "</script>".len());
    let tpl_end = src.find("<style>").unwrap_or(src.len());
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let (off, c) = chars[i];
        if off >= tpl_start && off < tpl_end && c == '{' {
            let mut depth = 1i32;
            let mut j = i + 1;
            while j < chars.len() && depth > 0 {
                match chars[j].1 {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            if depth == 0 {
                let (s, e) = (chars[i].0 + 1, chars[j - 1].0);
                // 跳过 {#if}/{/each}/{:else}/{@const} 这类块标记
                if !src[s..e].starts_with(['#', '/', ':', '@']) {
                    out.push((s, src[s..e].to_string()));
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn anchor_offsets(ts: proc_macro2::TokenStream) -> Vec<usize> {
    let mut out = Vec::new();
    fn go(ts: proc_macro2::TokenStream, out: &mut Vec<usize>) {
        for t in ts {
            match t {
                proc_macro2::TokenTree::Group(g) => go(g.stream(), out),
                proc_macro2::TokenTree::Punct(_) => {}
                other => out.push(other.span().byte_range().start),
            }
        }
    }
    go(ts, &mut out);
    out
}

// ---------------------------------------------------------------------------
// CRLF 与非 ASCII
// ---------------------------------------------------------------------------

/// 仓库在 Windows 上,`.svelte` 很可能被 git 按 CRLF 检出。
/// 行号靠数 `\n`、列靠往回数到 `\n` 为止的**字符**数,`\r` 落在上一行末尾,
/// 两者都不受影响 —— 这条测试把这句话钉住。
#[test]
fn map_handles_crlf_source() {
    let lf = COUNTER;
    let crlf = COUNTER.replace('\n', "\r\n");
    let a = compile(lf);
    let b = compile(&crlf);
    assert_eq!(a.code, b.code, "CRLF 不该改变生成代码");

    let pick = |c: &sv_compiler::Compiled, src: &str, needle: &str| {
        let pos = src.find(needle).unwrap();
        let s = c
            .map
            .segs
            .iter()
            .find(|s| s.sv_start == pos)
            .unwrap_or_else(|| panic!("{needle} 没有映射"));
        byte_to_line_col(src, s.sv_start)
    };
    // `{count + "x"}` 里的 count:LF 与 CRLF 下必须是同一个行列
    assert_eq!(pick(&a, lf, "count + "), (7, 17));
    assert_eq!(pick(&b, &crlf, "count + "), (7, 17));
}

/// 列的口径必须是 **1-based 字符列**(与 rustc 一致),不是字节列、不是 UTF-16。
/// 中文行上字符列 == UTF-16 列,碰巧看不出差别;所以这里必须**同时**放
/// 一行 emoji(非 BMP,占 2 个 UTF-16 code unit),否则测不出这一类。
#[test]
fn map_columns_are_char_based_with_cjk_and_emoji() {
    let src = "<script>\nlet count = $state(0i32);\n</script>\n\n\
               <view>\n  <text>中文中文中文中文 {count} 尾</text>\n  \
               <text>🎉🎉🎉 {count} 尾</text>\n</view>\n";
    let c = compile(src);
    let mut cols = Vec::new();
    for s in &c.map.segs {
        if &src[s.sv_start..s.sv_end] == "count" {
            cols.push(byte_to_line_col(src, s.sv_start));
        }
    }
    // 声明处(2:5)+ 两处模板引用
    assert!(cols.contains(&(2, 5)), "cols = {cols:?}");
    // `  <text>中文中文中文中文 {count}`:2 空格 + 6 + 8 汉字 + 空格 + `{` = 18,count 从 19 起。
    // 按字节算会是 2+6+24+1+1+1 = 35 —— 差 16,一眼能看出跑偏
    assert!(cols.contains(&(6, 19)), "CJK 行的列不对: {cols:?}");
    // `  <text>🎉🎉🎉 {count}`:2 + 6 + 3 emoji + 空格 + `{` = 13,count 从 14 起。
    // 按 UTF-16 code unit 算会是 17(每个 emoji 占 2)—— 这条专抓那种写法
    assert!(cols.contains(&(7, 14)), "emoji 行的列不对: {cols:?}");
}

// ---------------------------------------------------------------------------
// 落盘 + `sv check` 的消费路径(含降级)
// ---------------------------------------------------------------------------

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sv-map-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("建临时目录失败");
    d
}

/// 造一份"生成文件 + .svmap + .svelte"三件套,模拟 OUT_DIR 的现场
fn stage(dir: &Path, sv_src: &str) -> (PathBuf, sv_compiler::Compiled) {
    let sv_path = dir.join("Probe.svelte");
    std::fs::write(&sv_path, sv_src).unwrap();
    let c = sv_compiler::compile_sv_mapped(
        sv_src,
        "probe",
        &PropsRegistry::new(),
        &sv_path.display().to_string(),
    )
    .expect("应能编译");
    let gen_file = dir.join("probe.rs");
    std::fs::write(&gen_file, &c.code).unwrap();
    std::fs::write(dir.join("probe.rs.svmap"), c.map.to_text()).unwrap();
    (gen_file, c)
}

fn diag_json(file: &Path, line: usize, col: usize, col_end: usize) -> String {
    format!(
        r#"{{"level":"error","code":{{"code":"E0425"}},"message":"cannot find value `nope` in this scope","spans":[{{"file_name":"{}","line_start":{line},"column_start":{col},"column_end":{col_end},"is_primary":true}}],"children":[]}}"#,
        file.display().to_string().replace('\\', "\\\\")
    )
}

/// 落盘 → 读回 → 查表:`sv check` 真正走的那条路
#[test]
fn check_remaps_type_error_to_sv() {
    let dir = tmp_dir("remap");
    let (gen_file, c) = stage(&dir, COUNTER);

    // 在生成代码里找 `count.get() + "x"` 的 `count`,按它的行列造一条诊断
    let at = c.code.find("count.get() + \"x\"").unwrap();
    let (gl, gc) = byte_to_line_col(&c.code, at);
    let v = sv_compiler::check::json::parse(&diag_json(&gen_file, gl, gc, gc + 5)).unwrap();
    let r = sv_compiler::check::render(&v, &mut sv_compiler::check::Maps::default());

    let want = format!("{}:7:17: error[E0425]", dir.join("Probe.svelte").display());
    assert!(
        r.headline.starts_with(&want),
        "诊断没被搬回 .svelte:\n  实得 {}\n  期望前缀 {want}",
        r.headline
    );

    // 主 span 落在标点上(`+`)时也要能报到那个 `+`,而不是退到整个元素
    let plus = at + "count.get() ".len();
    let (pl, pc) = byte_to_line_col(&c.code, plus);
    let v = sv_compiler::check::json::parse(&diag_json(&gen_file, pl, pc, pc + 1)).unwrap();
    let r = sv_compiler::check::render(&v, &mut sv_compiler::check::Maps::default());
    let want = format!("{}:7:23:", dir.join("Probe.svelte").display());
    assert!(
        r.headline.starts_with(&want),
        "落在 `+` 上的诊断没落到 .svelte 的 `+`:\n  实得 {}\n  期望前缀 {want}",
        r.headline
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 映射不到时**必须**原样透出生成文件的位置 + 一句说明。
/// 吞诊断比不映射糟得多——用户会以为编译通过了。
#[test]
fn check_degrades_on_glue_without_dropping() {
    let dir = tmp_dir("glue");
    let (gen_file, c) = stage(&dir, COUNTER);
    // `__el1` 是纯胶水(codegen 的 fresh() 造出来的),不该有任何 .svelte provenance
    let at = c.code.find("__el1").expect("生成代码里应有 __el1");
    let (gl, gc) = byte_to_line_col(&c.code, at);
    let v = sv_compiler::check::json::parse(&diag_json(&gen_file, gl, gc, gc + 5)).unwrap();
    let r = sv_compiler::check::render(&v, &mut sv_compiler::check::Maps::default());
    assert!(
        r.headline.contains("cannot find value `nope`"),
        "降级路径把诊断内容弄丢了: {}",
        r.headline
    );
    assert!(
        r.headline.contains("sv-check:"),
        "降级必须附一句说明,不能默默给个假位置: {}",
        r.headline
    );
    assert!(r.headline.contains("probe.rs"), "降级时位置应留在生成文件");
    let _ = std::fs::remove_dir_all(&dir);
}

/// map 与 `.svelte` 对不上(改了 `.svelte` 但 build.rs 没重跑)时宁可不映射:
/// 给一个"像样但错误"的行号比不给更糟
#[test]
fn check_rejects_stale_map() {
    let dir = tmp_dir("stale");
    let (gen_file, c) = stage(&dir, COUNTER);
    std::fs::write(dir.join("Probe.svelte"), format!("\n\n{COUNTER}")).unwrap();
    let at = c.code.find("count.get()").unwrap();
    let (gl, gc) = byte_to_line_col(&c.code, at);
    let v = sv_compiler::check::json::parse(&diag_json(&gen_file, gl, gc, gc + 5)).unwrap();
    let r = sv_compiler::check::render(&v, &mut sv_compiler::check::Maps::default());
    assert!(
        r.headline.contains("span map 与 .svelte 内容对不上"),
        "过期 map 应触发降级: {}",
        r.headline
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 输入 N 条诊断,输出必须 N 条 —— 混着能映射的、映射不了的、根本不是我们的。
///
/// **走 `Session`(= bin 真正走的那条路),不走 `render`。**
/// `render` 的签名是 `&Value -> Rendered`,拿 `map().collect()` 的长度去断言
/// "条数守恒"是**恒真**的(`Iterator::map` 由类型系统保证等长),那种写法
/// 对真正会丢诊断的地方——"这行 JSON 解析不了就 `continue`"——完全无感。
#[test]
fn check_never_drops_diagnostic() {
    let dir = tmp_dir("count");
    let (gen_file, c) = stage(&dir, COUNTER);
    let at = c.code.find("count.get()").unwrap();
    let (gl, gc) = byte_to_line_col(&c.code, at);
    let glue = c.code.find("__el1").unwrap();
    let (el, ec) = byte_to_line_col(&c.code, glue);

    let wrap = |d: String| format!(r#"{{"reason":"compiler-message","message":{d}}}"#);
    let inputs = [
        wrap(diag_json(&gen_file, gl, gc, gc + 5)), // 能精确映射
        wrap(diag_json(&gen_file, el, ec, ec + 5)), // 落在胶水上
        wrap(diag_json(Path::new("src/main.rs"), 3, 1, 4)), // 普通 .rs
        wrap(r#"{"level":"warning","message":"unused variable","spans":[]}"#.to_string()), // 无 span
        r#"{"reason":"build-finished","success":false}"#.to_string(), // 不是诊断
        r#"{"reason":"compiler-message","message":{"level":"error","messa"#.to_string(), // 截断的 JSON
    ];
    let mut s = sv_compiler::check::Session::new();
    let (mut diags, mut skipped, mut unparsed) = (Vec::new(), 0usize, 0usize);
    for line in &inputs {
        match s.feed_stdout(line) {
            sv_compiler::check::Line::Diag(r) => diags.push(r),
            sv_compiler::check::Line::Skip => skipped += 1,
            sv_compiler::check::Line::Unparsed => unparsed += 1,
        }
    }
    assert_eq!(diags.len(), 4, "诊断条数不守恒");
    for (i, r) in diags.iter().enumerate() {
        assert!(!r.headline.is_empty(), "第 {i} 条诊断被吞了");
    }
    assert_eq!(
        diags.iter().filter(|r| r.is_error).count(),
        3,
        "error/warning 的分级不该在搬运中丢失"
    );
    assert_eq!((skipped, unparsed), (1, 1));
    assert!(
        s.summary().contains("解析不了"),
        "解析失败必须进汇总,否则那一行就是被静默丢了: {}",
        s.summary()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 没有 `.svmap` 的普通 `.rs` 诊断不该被我们碰
#[test]
fn check_leaves_plain_rust_diagnostics_alone() {
    let v = sv_compiler::check::json::parse(&diag_json(Path::new("src/lib.rs"), 12, 5, 9)).unwrap();
    let r = sv_compiler::check::render(&v, &mut sv_compiler::check::Maps::default());
    assert_eq!(
        r.headline,
        "src/lib.rs:12:5: error[E0425]: cannot find value `nope` in this scope"
    );
}

/// 建图降级(锚点失配)时 map 仍然写得出来,只是没有精确段 —— 查表全部返回
/// `None`,诊断走"原样透出"。这条模拟那个场景。
#[test]
fn empty_map_degrades_instead_of_panicking() {
    let m =
        SourceMap::parse_text("svmap 1\nsvlen 0\nsvhash 0000000000000000\ngenlen 0\nsv x.svelte\n")
            .expect("空表也应能解析");
    assert!(m.segs.is_empty());
    assert_eq!(m.lookup(0, 1), None);
}

/// 空表**不等于**"这里全是胶水":保险丝烧了要说自己烧了。
///
/// 这两种情形 `lookup` 都返回 `None`,但给用户的解释完全不同——
/// 说成"落在 runes 改写的胶水上"会把人支去查自己的 `.svelte`,而真实原因是
/// 映射机制整表作废、这个文件的**每一条**诊断都回不去。
#[test]
fn blown_fuse_is_not_reported_as_glue() {
    let dir = tmp_dir("blown");
    let (gen_file, c) = stage(&dir, COUNTER);
    let at = c.code.find("count.get()").unwrap();
    let (gl, gc) = byte_to_line_col(&c.code, at);

    let render_with = |svmap: String| {
        std::fs::write(dir.join("probe.rs.svmap"), svmap).unwrap();
        let v = sv_compiler::check::json::parse(&diag_json(&gen_file, gl, gc, gc + 5)).unwrap();
        sv_compiler::check::render(&v, &mut sv_compiler::check::Maps::default()).headline
    };

    // 对照组:表是好的,只是删光了段 → 确实是"落在胶水上"
    let mut empty = c.map.clone();
    empty.segs.clear();
    let glue = render_with(empty.to_text());
    assert!(glue.contains("胶水"), "{glue}");

    // 熔断:必须换一套说法,而且把原因带出来
    empty.blown = Some("锚点数格式化前后不等(120 vs 119)".into());
    let blown = render_with(empty.to_text());
    assert!(
        !blown.contains("胶水"),
        "熔断被说成了胶水,用户会去查错方向: {blown}"
    );
    assert!(blown.contains("整表作废"), "{blown}");
    assert!(blown.contains("120 vs 119"), "熔断原因必须带出来: {blown}");
    assert!(
        blown.contains("cannot find value `nope`"),
        "诊断丢了: {blown}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **锚点并行走必须是全的**(`lsp-spike.md` P0 验收项 `map_anchor_walk_is_total`)。
///
/// 仓库里全部真实 `.svelte` 上,保险丝一次都不许烧:烧了就意味着 prettyplease 换了
/// 打印策略 / 某个 parse 入口的 provenance 断了,那个文件的诊断会整批回不去。
/// `map_segments_are_verbatim` 的 `!segs.is_empty()` 是弱替代——它对"熔断后
/// 恰好还剩几段"这种情形无感,而现在熔断是**显式**状态,可以直接断言。
#[test]
fn map_anchor_walk_is_total() {
    for (name, src) in all_sv_sources() {
        if sv_compiler::compile_sv(&src, "probe").is_err() {
            continue;
        }
        let c = compile(&src);
        assert_eq!(
            c.map.blown, None,
            "{name}:锚点并行走熔断了(整张表作废,该文件所有诊断都回不到 .svelte)"
        );
        assert!(!c.map.segs.is_empty(), "{name}:一段映射都没建出来");
    }
}

/// **相邻锚点插值不许跨行。**
///
/// region 的粒度如果是"整个 script 块一个包络"(计划 §6 批准的第一版降级),
/// 插值会跨语句:实测 `let double = …` 那条语句的 `let`(runes 改写后已是胶水)
/// 被插到上一条语句末尾的 `);` 上,报出 `.svelte` 第 2 行——而它在第 3 行。
/// 这正是本 crate 自己写的"宁可不映射,也不给像样但错误的位置"要挡的东西,
/// 所以 region 取到**行**。这条测试在全部真实 `.svelte` 上钉住不变量:
/// `Between` 命中的那段 `.svelte` 间隙里**不含换行**。
#[test]
fn between_interpolation_never_crosses_a_line() {
    for (name, src) in all_sv_sources() {
        if sv_compiler::compile_sv(&src, "probe").is_err() {
            continue;
        }
        let c = compile(&src);
        let mut seen = 0usize;
        for (i, _) in c.code.char_indices() {
            let Some(h) = c.map.lookup(i, i + 1) else {
                continue;
            };
            if h.kind != MapKind::Between {
                continue;
            }
            seen += 1;
            let gap = &src[h.sv_start..h.sv_end];
            assert!(
                !gap.contains('\n'),
                "{name}:生成侧 {:?} 的插值跨了 .svelte 的行 —— 间隙 {gap:?} @ {:?}",
                byte_to_line_col(&c.code, i),
                byte_to_line_col(&src, h.sv_start)
            );
        }
        assert!(seen > 0, "{name}:一次插值都没发生,测试失去意义");
    }
}

/// 招牌用例的反面:`let double` 这条语句的 `let` 是 runes 改写的产物,
/// 它**不该**被插值到上一条语句上,而该老实降级
#[test]
fn glue_let_in_script_degrades_instead_of_pointing_at_previous_statement() {
    let c = compile(COUNTER);
    let at = c
        .code
        .find("let double")
        .expect("生成代码里应有 let double");
    let hit = c.map.lookup(at, at + 3);
    if let Some(h) = hit {
        let (l, _) = byte_to_line_col(COUNTER, h.sv_start);
        assert_ne!(
            l, 2,
            "被插值到了上一条语句(.svelte 第 2 行),`let double` 在第 3 行"
        );
    }
}

/// 三档降级都要能被区分出来(输出里的措辞不同)
#[test]
fn lookup_kinds_are_distinguishable() {
    let c = compile(COUNTER);
    let exact = c.code.find("count.get() + \"x\"").unwrap();
    assert_eq!(
        c.map.lookup(exact, exact + 5).map(|h| h.kind),
        Some(MapKind::Exact)
    );
    let plus = exact + "count.get() ".len();
    assert_eq!(
        c.map.lookup(plus, plus + 1).map(|h| h.kind),
        Some(MapKind::Between)
    );
}

// ---------------------------------------------------------------------------
// 端到端:临时 crate 里真跑 cargo check
// ---------------------------------------------------------------------------

/// 造一个**故意写错**的 `.svelte`(引用了不存在的变量),真跑 `cargo check`,
/// 断言 `sv check` 把诊断报在 `.svelte` 的正确行列上。
///
/// `#[ignore]`:它要在独立 target 目录里现编 sv-ui / sv-reactive / syn 全家桶,
/// 分钟级。跑法:
/// ```sh
/// cargo test -p sv-compiler --test sourcemap -- --ignored --nocapture
/// ```
#[test]
#[ignore = "要现编依赖,分钟级;CI 单独 job 跑"]
fn e2e_bad_sv_reports_on_sv_line() {
    let root = tmp_dir("e2e");
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .unwrap();
    let crates = crates.display().to_string();
    let crates = crates
        .strip_prefix(r"\\?\")
        .unwrap_or(&crates)
        .replace('\\', "/");

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            "[workspace]\n\
             [package]\nname = \"sv-e2e\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\n\
             sv-ui = {{ path = \"{crates}/sv-ui\" }}\n\
             sv-reactive = {{ path = \"{crates}/sv-reactive\" }}\n\n\
             [build-dependencies]\n\
             sv-compiler = {{ path = \"{crates}/sv-compiler\" }}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("build.rs"),
        "fn main() { sv_compiler::build(\"src\"); }\n",
    )
    .unwrap();
    // 第 7 行第 20 列引用了不存在的 `nope`(前面刻意放中文,顺带验列的口径)
    let bad = "<script>\n\
               let count = $state(0i32);\n\
               </script>\n\
               \n\
               <view>\n\
               \x20 <text>计数中文:{count}</text>\n\
               \x20 <text>坏的中文:{nope}</text>\n\
               </view>\n";
    std::fs::write(root.join("src/Bad.svelte"), bad).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "include!(concat!(env!(\"OUT_DIR\"), \"/bad.rs\"));\nfn main() {}\n",
    )
    .unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_sv-check"))
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env_remove("CARGO")
        .output()
        .expect("跑 sv-check 失败");
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("--- sv-check stdout ---\n{stdout}");
    eprintln!("--- stderr ---\n{}", String::from_utf8_lossy(&out.stderr));

    // `  <text>坏的中文:{nope}</text>`:2 + 6 + 5 个中文/全角字符 + `{` = 14,nope 从 15 起
    let pos = bad.find("nope").unwrap();
    assert_eq!(byte_to_line_col(bad, pos), (7, 15), "测试自身的期望算错了");
    // map 里的路径是 canonicalize 过的,分隔符与 `join("src/Bad.svelte")` 不同,
    // 比较前先统一(Windows 上 rustc 自己吐的路径也是混用的)
    let norm = |s: &str| s.replace('\\', "/");
    let want = format!(
        "{}:7:15: error[E0425]",
        norm(&root.join("src/Bad.svelte").display().to_string())
    );
    assert!(
        stdout.lines().any(|l| norm(l).starts_with(&want)),
        "诊断没落在 .svelte 的正确行列上,期望前缀:\n  {want}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
