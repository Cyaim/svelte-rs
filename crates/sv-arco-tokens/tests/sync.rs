//! 转译不漂移:`src/generated.rs` 必须与生成器输出逐字符一致(CRLF 归一)。
//! 改了 `assets/*.less` 或生成器却忘了重跑 `gen_tokens`,这里会红。

#[test]
fn generated_rs_is_in_sync_with_generator() {
    let expected = sv_arco_tokens::generator::generate();
    let actual = include_str!("../src/generated.rs");
    assert_eq!(
        actual.replace("\r\n", "\n"),
        expected.replace("\r\n", "\n"),
        "src/generated.rs 过期:请跑 cargo run -p sv-arco-tokens --bin gen_tokens"
    );
}
