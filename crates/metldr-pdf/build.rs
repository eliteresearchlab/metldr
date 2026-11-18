//! PDFium's wasm build carries libsetjmp.a, whose setjmp helpers Rust's own codegen also
//! emits; the linker keeps the first of the identical definitions.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg=--allow-multiple-definition");
    }
}
