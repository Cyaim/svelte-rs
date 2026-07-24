//! 见 Cargo.toml:这个 lib 的存在意义是**编译**下面 include! 进来的生成代码,
//! 让 rustc 对它做完整借用检查。生成的 `check` 函数不必被调用 —— Rust 对
//! crate 内所有 item(含未使用的)都做借用检查。

// Check.svelte 模板里引用到的两个自由函数(await 的 future、each 的 list 源)。
// 只需类型对得上、能过借用检查即可。
// 带 {:catch} 的 await 走 await_block_result,future 须 resolve 到 Result
async fn load(_s: String) -> Result<i32, String> {
    Ok(0)
}
fn make(s: String) -> Vec<String> {
    vec![s]
}

include!(concat!(env!("OUT_DIR"), "/check.rs"));
