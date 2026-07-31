//! Download + materialization half of the MLX pipeline, without the sidecar.
//!
//! ```sh
//! MLX_E2E_BIT=/path/to/bit.json MLX_E2E_STORE=/path/to/store \
//!   cargo test -p flow-like --test mlx_pack_e2e -- --ignored --nocapture
//! ```

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use flow_like::{
    bit::Bit,
    models::{llm::mlx_pack::materialize_mlx_model, local_utils::ensure_local_weights},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::tokio;
use std::{env, path::PathBuf, sync::Arc, time::Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn mlx_weights_download_and_materialize() {
    let store_dir = PathBuf::from(env::var("MLX_E2E_STORE").expect("MLX_E2E_STORE"));
    let bit_path = env::var("MLX_E2E_BIT").expect("MLX_E2E_BIT");
    let bit: Bit = flow_like_types::json::from_str(
        &std::fs::read_to_string(&bit_path).expect("read bit json"),
    )
    .expect("deserialize bit");

    std::fs::create_dir_all(&store_dir).expect("store dir");
    let mut config = FlowLikeConfig::new();
    let store = Arc::new(LocalObjectStore::new(store_dir.clone()).expect("store"));
    config.register_bits_store(FlowLikeStore::Local(store.clone()));
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));

    println!("cache_key={:?}", bit.mlx_runtime_model_cache_key());

    let pack = bit.pack(state.clone()).await.expect("pack");
    println!("pack bits: {}", pack.bits.len());
    for b in &pack.bits {
        println!(
            "  {} {:?} {} bytes hash={}",
            b.file_name.as_deref().unwrap_or("<root>"),
            b.bit_type,
            b.size.unwrap_or(0),
            b.hash
        );
    }

    let started = Instant::now();
    ensure_local_weights(&pack, &state, bit.id.as_str(), "MLX model")
        .await
        .expect("ensure_local_weights");
    println!("\ndownload finished in {:?}", started.elapsed());

    let started = Instant::now();
    let materialized = tokio::task::spawn_blocking({
        let bit = bit.clone();
        let store = store.clone();
        move || materialize_mlx_model(&bit, &pack, &store)
    })
    .await
    .expect("join")
    .expect("materialize");
    println!("materialized in {:?}", started.elapsed());
    println!("path: {}", materialized.path.display());

    let mut entries: Vec<String> = std::fs::read_dir(&materialized.path)
        .expect("read materialized dir")
        .map(|e| {
            let e = e.expect("entry");
            format!(
                "{} ({} bytes)",
                e.file_name().to_string_lossy(),
                e.metadata().map(|m| m.len()).unwrap_or(0)
            )
        })
        .collect();
    entries.sort();
    println!("\nmaterialized files:");
    for entry in &entries {
        println!("  {entry}");
    }

    assert!(materialized.path.join("config.json").is_file());
    assert!(materialized.path.join("tokenizer.json").is_file());
    assert!(materialized.path.join("model.safetensors").is_file());

    // Second call must hit the cache and return the same directory.
    let pack_again = bit.pack(state.clone()).await.expect("pack again");
    let started = Instant::now();
    let repeat = tokio::task::spawn_blocking({
        let bit = bit.clone();
        let store = Arc::new(LocalObjectStore::new(store_dir.clone()).expect("store"));
        move || materialize_mlx_model(&bit, &pack_again, &store)
    })
    .await
    .expect("join")
    .expect("materialize again");
    println!(
        "\nrepeat materialize in {:?} -> same path: {}",
        started.elapsed(),
        repeat.path == materialized.path
    );
    assert_eq!(
        repeat.path, materialized.path,
        "cache key must be deterministic"
    );
}
