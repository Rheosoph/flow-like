//! End-to-end MLX check: a real user bit (pinned Hugging Face manifest) is
//! packed, downloaded, materialized and answered by the Swift runtime.
//!
//! Ignored by default because it needs network access, ~300 MB of disk and the
//! `flow-like-mlx-service` sidecar staged next to the test binary.
//!
//! ```sh
//! MLX_E2E_BIT=/path/to/bit.json MLX_E2E_STORE=/path/to/store \
//!   cargo test -p flow-like --test mlx_e2e -- --ignored --nocapture
//! ```

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use flow_like::{
    bit::Bit,
    models::llm::{ExecutionSettings, ModelFactory, mlx::MlxModel},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_model_provider::{
    history::{History, HistoryMessage, Role},
    llm::ModelLogic,
    response_chunk::ResponseChunk,
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::{sync::Mutex, tokio};
use std::{
    env,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

fn state_with_store(store_dir: &PathBuf) -> Arc<FlowLikeState> {
    std::fs::create_dir_all(store_dir).expect("store dir");
    let mut config = FlowLikeConfig::new();
    let store = LocalObjectStore::new(store_dir.clone()).expect("local store");
    config.register_bits_store(FlowLikeStore::Local(Arc::new(store)));
    let http_client = HTTPClient::new_without_refetch();
    Arc::new(FlowLikeState::new(config, http_client))
}

fn load_bit() -> Bit {
    let path = env::var("MLX_E2E_BIT").expect("MLX_E2E_BIT must point at the imported bit JSON");
    let raw = std::fs::read_to_string(&path).expect("read bit json");
    flow_like_types::json::from_str::<Bit>(&raw).expect("deserialize bit")
}

#[flow_like_types::tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn mlx_model_answers_end_to_end() {
    let store_dir = PathBuf::from(
        env::var("MLX_E2E_STORE").expect("MLX_E2E_STORE must point at a bit store directory"),
    );
    let bit = load_bit();

    println!("== bit ==");
    println!("id={} type={:?}", bit.id, bit.bit_type);
    println!("is_mlx_model={}", bit.is_mlx_model());
    println!("runtime_cache_key={:?}", bit.mlx_runtime_model_cache_key());

    let state = state_with_store(&store_dir);

    // 1. Manifest expansion: inline assets must materialize without a hub.
    let inline = bit.inline_mlx_asset_bits().expect("inline asset bits");
    println!("\n== inline assets ({}) ==", inline.len());
    for asset in &inline {
        println!(
            "  {} -> {} ({} bytes)",
            asset.id,
            asset.file_name.as_deref().unwrap_or("<none>"),
            asset.size.unwrap_or(0)
        );
    }

    let pack = bit.pack(state.clone()).await.expect("pack");
    println!("\n== pack: {} bits ==", pack.bits.len());

    // 2. Download + materialize + sidecar spawn.
    let started = Instant::now();
    let settings = ExecutionSettings::new();
    let model = MlxModel::new(&bit, state.clone(), &settings)
        .await
        .expect("MlxModel::new");
    println!(
        "\n== runtime up on 127.0.0.1:{} in {:?} ==",
        model.port,
        started.elapsed()
    );

    // 3. Non-streaming completion through the real provider path.
    let mut history = History::new(
        bit.id.clone(),
        vec![HistoryMessage::from_string(
            Role::User,
            "Reply with exactly one word: banana",
        )],
    );
    history.stream = Some(false);
    history.max_completion_tokens = Some(32);

    let started = Instant::now();
    let response = model
        .invoke(&history, None)
        .await
        .expect("non-stream invoke");
    let text = response
        .last_message()
        .and_then(|message| message.content.clone())
        .unwrap_or_default();
    println!("\n== non-streaming ({:?}) ==", started.elapsed());
    println!("text: {text:?}");
    println!("usage: {:?}", response.usage);
    assert!(!text.trim().is_empty(), "expected non-empty completion");

    // 4. Streaming completion: chunks must arrive and usage must be reported.
    let chunks = Arc::new(AtomicUsize::new(0));
    let collected = Arc::new(Mutex::new(String::new()));
    let last_usage = Arc::new(Mutex::new(None));
    let (chunk_counter, sink, usage_sink) = (chunks.clone(), collected.clone(), last_usage.clone());

    let callback = Arc::new(move |chunk: ResponseChunk| {
        let sink = sink.clone();
        let usage_sink = usage_sink.clone();
        let counter = chunk_counter.clone();
        Box::pin(async move {
            counter.fetch_add(1, Ordering::Relaxed);
            for choice in &chunk.choices {
                if let Some(delta) = &choice.delta
                    && let Some(content) = &delta.content
                {
                    sink.lock().await.push_str(content);
                }
            }
            if let Some(usage) = chunk.usage.clone() {
                *usage_sink.lock().await = Some(usage);
            }
            Ok(())
        }) as Pin<Box<dyn Future<Output = flow_like_types::Result<()>> + Send>>
    });

    let mut stream_history = History::new(
        bit.id.clone(),
        vec![HistoryMessage::from_string(
            Role::User,
            "Count from 1 to 5, comma separated.",
        )],
    );
    stream_history.stream = Some(true);
    stream_history.max_completion_tokens = Some(64);

    let started = Instant::now();
    let stream_response = model
        .invoke(&stream_history, Some(callback))
        .await
        .expect("streaming invoke");
    let streamed = collected.lock().await.clone();
    println!("\n== streaming ({:?}) ==", started.elapsed());
    println!("chunks: {}", chunks.load(Ordering::Relaxed));
    println!("streamed text: {streamed:?}");
    println!("stream usage: {:?}", last_usage.lock().await);
    println!("response usage: {:?}", stream_response.usage);

    assert!(
        chunks.load(Ordering::Relaxed) > 1,
        "expected multiple chunks"
    );
    assert!(!streamed.trim().is_empty(), "expected streamed content");

    // 5. A stop sequence still has to report real token usage: the Swift
    //    runtime never emits its terminal info event on that path.
    let mut stop_history = History::new(
        bit.id.clone(),
        vec![HistoryMessage::from_string(
            Role::User,
            "Count slowly: 1 2 3 4 5 6",
        )],
    );
    stop_history.stream = Some(false);
    stop_history.max_completion_tokens = Some(40);
    stop_history.stop = Some(vec!["3".to_string()]);

    let stopped = model
        .invoke(&stop_history, None)
        .await
        .expect("stop-sequence invoke");
    println!("\n== stop sequence ==");
    println!(
        "text: {:?}",
        stopped
            .last_message()
            .and_then(|message| message.content.clone())
            .unwrap_or_default()
    );
    println!("usage: {:?}", stopped.usage);
    assert!(
        stopped.usage.prompt_tokens > 0,
        "stop sequence must still report prompt tokens, got {:?}",
        stopped.usage
    );
    assert!(
        stopped.usage.total_tokens > 0,
        "stop sequence must still report total tokens, got {:?}",
        stopped.usage
    );

    // 6. Second construction must reuse the materialized cache quickly.
    let started = Instant::now();
    let cached = MlxModel::new(&bit, state.clone(), &settings)
        .await
        .expect("second MlxModel::new");
    println!(
        "\n== second runtime up on port {} in {:?} (cache reuse) ==",
        cached.port,
        started.elapsed()
    );

    // 7. Factory cache key must dedupe the runtime.
    let factory = Arc::new(Mutex::new(ModelFactory::new()));
    let first = factory
        .lock()
        .await
        .build(&bit, state.clone(), None, None)
        .await
        .expect("factory build");
    let second = factory
        .lock()
        .await
        .build(&bit, state.clone(), None, None)
        .await
        .expect("factory build 2");
    println!(
        "\n== factory cache: same instance = {} ==",
        Arc::ptr_eq(&first, &second)
    );
    assert!(
        Arc::ptr_eq(&first, &second),
        "factory must cache the runtime"
    );
}
