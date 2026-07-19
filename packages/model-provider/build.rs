fn main() {
    let target_vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default();
    let local_ml_enabled = std::env::var_os("CARGO_FEATURE_LOCAL_ML").is_some();

    if target_vendor == "apple" && local_ml_enabled {
        // The official static ORT framework uses CoreML and Accelerate internally. Emitting
        // these here keeps standalone model-provider/catalog consumers linkable too.
        println!("cargo:rustc-link-lib=framework=Accelerate");
        println!("cargo:rustc-link-lib=framework=CoreML");
    }
}
