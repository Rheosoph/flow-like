pub mod local;
#[cfg(feature = "local-ml")]
pub mod multimodal;

// One request can prepare its next batch while another runs inference, without
// letting queued requests retain an unbounded number of prepared media tensors.
#[cfg(feature = "local-ml")]
pub(crate) const PREPARATION_SLOTS: usize = 2;

#[cfg(feature = "local-ml")]
pub(crate) fn run_embedding_session<T, R>(
    session: &flow_like_types::sync::Mutex<T>,
    run: impl FnOnce(&mut T) -> flow_like_types::Result<R>,
) -> flow_like_types::Result<R> {
    let waiting = std::time::Instant::now();
    let mut session = session.blocking_lock();
    let wait = waiting.elapsed();
    let started = std::time::Instant::now();
    let result = run(&mut session);
    let inference = started.elapsed();
    drop(session);
    tracing::debug!(
        wait_ms = wait.as_secs_f64() * 1000.0,
        inference_ms = inference.as_secs_f64() * 1000.0,
        "embedding inference"
    );
    result
}
