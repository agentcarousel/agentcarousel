fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_experimental)");
    let exp = std::path::Path::new("src/cli/experimental/mod.rs");
    if exp.exists() {
        println!("cargo:rustc-cfg=has_experimental");
    }
    if std::env::var("CARGO_FEATURE_EXPERIMENTAL").is_ok() {
        println!("cargo:rustc-env=AGENTCAROUSEL_VERSION_SUFFIX= (experimental)");
    } else {
        println!("cargo:rustc-env=AGENTCAROUSEL_VERSION_SUFFIX=");
    }
    println!("cargo:rerun-if-changed=src/cli/experimental/mod.rs");
}
