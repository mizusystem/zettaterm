//! ヘッドレス端末と TTL 実行。コアと同じ Rust なので FFI を通らず直接リンクする。

fn main() {
    println!("mt {} (scaffold)", env!("CARGO_PKG_VERSION"));
}
