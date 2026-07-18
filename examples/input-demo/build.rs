fn main() {
    // 扫描 src/ 下所有 .sv,编译成 $OUT_DIR/<组件名>.rs
    sv_compiler::build("src");
}
