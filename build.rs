fn main() {
    if std::env::var("CARGO_FEATURE_LOOM").is_ok() {
        println!("cargo:rustc-cfg=loom");
    }
}
