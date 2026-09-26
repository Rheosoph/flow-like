use super::*;
use std::convert::Infallible;
use std::future::{pending, poll_fn};
use std::task::Waker;
use tokio::sync::mpsc;
use tower::ServiceExt;

type Exports = Arc<Mutex<Vec<Vec<SpanData>>>>;

fn harness(
    callback: Arc<dyn Fn(Vec<SpanData>) -> FlushFuture + Send + Sync>,
    sample_rate: f64,
) -> (
    Telemetry,
    InvocationSpans,
    SdkTracerProvider,
    tracing::Dispatch,
) {
    let spans = InvocationSpans::default();
    let provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOn)
        .with_span_processor(spans.clone())
        .build();
    let layer = tracing_opentelemetry::layer()
        .with_tracer(provider.tracer("lifecycle-test"))
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.is_span() && metadata.target() == TARGET
        }));
    let dispatch = tracing::Dispatch::new(tracing_subscriber::registry().with(layer));
    (
        Telemetry::for_test(spans.clone(), sample_rate, callback),
        spans,
        provider,
        dispatch,
    )
}

fn warm(telemetry: &Telemetry) {
    telemetry.cold.store(false, Ordering::Relaxed);
}

async fn complete<S>(telemetry: &Telemetry, service: S, request: Request) -> hyper::body::Bytes
where
    S: Service<Request, Response = Response<Body>, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    let response = telemetry.wrap(service).oneshot(request).await.unwrap();
    axum::body::to_bytes(response.into_body(), 64)
        .await
        .unwrap()
}

fn root(batch: &[SpanData]) -> &SpanData {
    batch.iter().find(|span| span.name == INVOCATION).unwrap()
}

fn recorder(exports: &Exports) -> Arc<dyn Fn(Vec<SpanData>) -> FlushFuture + Send + Sync> {
    let exports = exports.clone();
    Arc::new(move |batch| {
        let exports = exports.clone();
        Box::pin(async move { exports.lock().unwrap().push(batch) })
    })
}

fn request(sampled: bool) -> Request {
    let mut context = lambda_http::Context::default();
    context.request_id = "12345678-1234-1234-1234-123456789abc".into();
    context.xray_trace_id = Some(format!(
        "Root=1-69abcdef-0123456789abcdef01234567;Parent=0123456789abcdef;Sampled={}",
        u8::from(sampled)
    ));
    hyper::Request::builder()
        .uri("/items/private-value")
        .header("authorization", "Bearer sensitive-token")
        .header(
            "traceparent",
            "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
        )
        .body(lambda_http::Body::Empty)
        .unwrap()
        .with_lambda_context(context)
}

struct Frames(mpsc::Receiver<Result<Frame<hyper::body::Bytes>, std::io::Error>>);

impl HttpBody for Frames {
    type Data = hyper::body::Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.0.poll_recv(cx)
    }
}

// Model the route body's ownership contract without relying on private API
// middleware types. Its span must outlive service response creation.
struct RouteBody {
    body: Body,
    span: Option<Span>,
}

impl HttpBody for RouteBody {
    type Data = hyper::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let polled = {
            let span = self.span.clone().unwrap_or_else(Span::none);
            let _entered = span.enter();
            Pin::new(&mut self.body).poll_frame(cx)
        };
        if matches!(&polled, Poll::Ready(None) | Poll::Ready(Some(Err(_))))
            || matches!(&polled, Poll::Ready(Some(Ok(frame))) if frame.is_trailers())
        {
            self.span.take();
        }
        polled
    }
}

fn service(
    body: Body,
) -> impl Service<Request, Response = Response<Body>, Error = Infallible, Future: Send + 'static>
+ Clone
+ Send
+ 'static {
    let body = Arc::new(Mutex::new(Some(body)));
    tower::service_fn(move |_request: Request| {
        let body = body.lock().unwrap().take().unwrap();
        async move {
            let span = tracing::info_span!(target: "flow_like::observability", "http.request", http.route = "/items/{id}");
            async {
                let query = tracing::info_span!(target: "flow_like::observability", "db.query", db.operation = "select", db.table = "App");
                async {}.instrument(query).await;
            }
            .instrument(span.clone())
            .await;
            Ok(Response::new(Body::new(RouteBody {
                body,
                span: Some(span),
            })))
        }
    })
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_flush_waits_for_eof_and_exports_the_complete_parent_chain() {
    let exports = Exports::default();
    let (telemetry, queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    let (sender, receiver) = mpsc::channel(2);
    let response = telemetry
        .wrap(service(Body::new(Frames(receiver))))
        .oneshot(request(true))
        .await
        .unwrap();
    let mut body = response.into_body();
    assert!(exports.lock().unwrap().is_empty());
    assert_eq!(
        queue.queue.lock().unwrap().len(),
        1,
        "only the completed query is queued"
    );
    let waker = Waker::noop();
    assert!(
        Pin::new(&mut body)
            .poll_frame(&mut Context::from_waker(waker))
            .is_pending()
    );
    sender
        .send(Ok(Frame::data(hyper::body::Bytes::from_static(b"first"))))
        .await
        .unwrap();
    let frame = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(frame.into_data().unwrap(), "first");
    assert!(
        exports.lock().unwrap().is_empty(),
        "a data frame must not flush or close roots"
    );
    drop(sender);
    assert!(
        poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
            .await
            .is_none()
    );
    assert!(body.is_end_stream());
    assert!(queue.queue.lock().unwrap().is_empty());
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1);
    let batch = &exports[0];
    assert_eq!(batch.len(), 3);
    let named = |name: &str| batch.iter().find(|span| span.name == name).unwrap();
    let invocation = named("lambda.invocation");
    let route = named("http.request");
    let query = named("db.query");
    assert_eq!(invocation.parent_span_id.to_string(), "0123456789abcdef");
    assert_eq!(route.parent_span_id, invocation.span_context.span_id());
    assert_eq!(query.parent_span_id, route.span_context.span_id());
    assert!(
        batch
            .iter()
            .all(|span| span.span_context.trace_id().to_string()
                == "69abcdef0123456789abcdef01234567")
    );
    assert!(!format!("{batch:?}").contains("sensitive-token"));
    assert!(!format!("{batch:?}").contains("private-value"));
}

#[tokio::test(flavor = "current_thread")]
async fn an_ordinary_warm_unsampled_trace_is_dropped_even_with_a_sampled_viewer_header() {
    let exports = Exports::default();
    let (telemetry, queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    warm(&telemetry);
    assert_eq!(
        complete(&telemetry, service(Body::from("ok")), request(false)).await,
        "ok"
    );
    assert!(queue.queue.lock().unwrap().is_empty());
    assert!(exports.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn an_unsampled_platform_parent_starts_a_new_root_trace() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 1.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    warm(&telemetry);
    complete(&telemetry, service(Body::from("ok")), request(false)).await;
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1, "the ratio still applies to new roots");
    let batch = &exports[0];
    assert_eq!(batch.len(), 3);
    let invocation = root(batch);
    let trace_id = invocation.span_context.trace_id();
    assert_eq!(invocation.parent_span_id, SpanId::INVALID);
    assert_ne!(trace_id.to_string(), "69abcdef0123456789abcdef01234567");
    assert_ne!(trace_id.to_string(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(
        batch
            .iter()
            .all(|span| span.span_context.trace_id() == trace_id)
    );
    let route = batch
        .iter()
        .find(|span| span.name == "http.request")
        .unwrap();
    assert_eq!(route.parent_span_id, invocation.span_context.span_id());
}

#[tokio::test(flavor = "current_thread")]
async fn a_sampled_platform_parent_is_honoured_for_a_warm_fast_trace() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    warm(&telemetry);
    complete(&telemetry, service(Body::from("ok")), request(true)).await;
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1);
    let invocation = root(&exports[0]);
    assert_eq!(invocation.parent_span_id.to_string(), "0123456789abcdef");
    assert_eq!(
        invocation.span_context.trace_id().to_string(),
        "69abcdef0123456789abcdef01234567"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_cold_start_is_exported_without_a_platform_decision() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    complete(&telemetry, service(Body::from("ok")), request(false)).await;
    complete(&telemetry, service(Body::from("ok")), request(false)).await;
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1, "only the cold invocation is exported");
    assert!(
        root(&exports[0])
            .attributes
            .contains(&KeyValue::new("faas.coldstart", true))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_server_error_is_exported_without_a_platform_decision() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    warm(&telemetry);
    let failing = service(Body::from("failed")).map_response(|mut response: Response<Body>| {
        *response.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
        response
    });
    complete(&telemetry, failing, request(false)).await;
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].len(), 3);
    assert_eq!(
        root(&exports[0]).status,
        opentelemetry::trace::Status::error("")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_late_span_from_a_previous_invocation_does_not_change_the_current_decision() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    let background = Arc::new(Mutex::new(None::<Span>));
    let slot = background.clone();
    let spawns_background = tower::service_fn(move |_request: Request| {
        let slot = slot.clone();
        async move {
            // A tracing child would hold the invocation open; link only the OTel parent.
            let task = tracing::info_span!(target: "flow_like::observability", parent: None, "background.task", otel.status_code = tracing::field::Empty);
            let _ = task.set_parent(Span::current().context());
            *slot.lock().unwrap() = Some(task);
            Ok::<_, Infallible>(Response::new(Body::from("ok")))
        }
    });
    complete(&telemetry, spawns_background, request(false)).await;
    let previous_trace = {
        let exports = exports.lock().unwrap();
        assert_eq!(exports.len(), 1, "the cold invocation is kept");
        root(&exports[0]).span_context.trace_id()
    };

    let response = telemetry
        .wrap(service(Body::from("ok")))
        .oneshot(request(false))
        .await
        .unwrap();
    let late = background.lock().unwrap().take().unwrap();
    late.record("otel.status_code", "ERROR");
    drop(late);
    axum::body::to_bytes(response.into_body(), 16)
        .await
        .unwrap();

    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 2);
    let batch = &exports[1];
    assert_eq!(
        batch.len(),
        1,
        "the ordinary current invocation stays dropped"
    );
    assert_eq!(batch[0].name, "background.task");
    assert_eq!(batch[0].span_context.trace_id(), previous_trace);
    assert_eq!(batch[0].status, opentelemetry::trace::Status::error(""));
}

#[tokio::test(flavor = "current_thread")]
async fn an_unresponsive_exporter_cannot_hold_response_eof_indefinitely() {
    let started = Arc::new(AtomicBool::new(false));
    let callback_started = started.clone();
    let callback = Arc::new(move |_batch| -> FlushFuture {
        callback_started.store(true, Ordering::Relaxed);
        Box::pin(pending())
    });
    let (telemetry, queue, _provider, dispatch) = harness(callback, 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    let response = telemetry
        .wrap(service(Body::from("ok")))
        .oneshot(request(true))
        .await
        .unwrap();
    let bytes = tokio::time::timeout(
        EXPORT_BUDGET * 4,
        axum::body::to_bytes(response.into_body(), 16),
    )
    .await
    .expect("local export timeout must release the runtime response")
    .unwrap();
    assert_eq!(bytes, "ok");
    assert!(started.load(Ordering::Relaxed));
    assert!(
        queue.queue.lock().unwrap().is_empty(),
        "a timed-out batch is discarded, not retried on the response path"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn body_error_is_preserved_after_completed_spans_are_exported() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    warm(&telemetry);
    let (sender, receiver) = mpsc::channel(1);
    sender
        .send(Err(std::io::Error::other("sensitive-upstream-error")))
        .await
        .unwrap();
    let response = telemetry
        .wrap(service(Body::new(Frames(receiver))))
        .oneshot(request(false))
        .await
        .unwrap();
    let error = axum::body::to_bytes(response.into_body(), 16)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("sensitive-upstream-error"));
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].len(), 3);
    assert!(!format!("{exports:?}").contains("sensitive-upstream-error"));
    let root = exports[0]
        .iter()
        .find(|span| span.name == "lambda.invocation")
        .unwrap();
    assert_eq!(root.status, opentelemetry::trace::Status::error(""));
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_a_stream_closes_both_roots_before_best_effort_export() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    warm(&telemetry);
    let (_sender, receiver) = mpsc::channel(1);
    let response = telemetry
        .wrap(service(Body::new(Frames(receiver))))
        .oneshot(request(false))
        .await
        .unwrap();
    drop(response);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !exports.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("body drop should schedule a best-effort flush while the runtime is alive");
    let exports = exports.lock().unwrap();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].len(), 3);
    assert_eq!(
        root(&exports[0]).status,
        opentelemetry::trace::Status::error("")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn trailers_flush_before_lambda_http_treats_the_frame_as_stream_eof() {
    let exports = Exports::default();
    let (telemetry, _queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    let (sender, receiver) = mpsc::channel(1);
    let mut trailers = hyper::HeaderMap::new();
    trailers.insert("x-test", "finished".parse().unwrap());
    sender.send(Ok(Frame::trailers(trailers))).await.unwrap();
    let response = telemetry
        .wrap(service(Body::new(Frames(receiver))))
        .oneshot(request(true))
        .await
        .unwrap();
    let mut body = response.into_body();
    let frame = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx))
        .await
        .unwrap()
        .unwrap();
    assert!(frame.is_trailers());
    assert_eq!(
        exports.lock().unwrap().len(),
        1,
        "the Lambda adapter stops polling after trailers"
    );
    assert_eq!(exports.lock().unwrap()[0].len(), 3);
}

#[tokio::test(flavor = "current_thread")]
async fn warm_invocations_keep_separate_parents_batches_and_coldstart_flags() {
    let exports = Exports::default();
    let (telemetry, queue, _provider, dispatch) = harness(recorder(&exports), 0.0);
    let _subscriber = tracing::dispatcher::set_default(&dispatch);
    let contexts = [
        (
            "69abcdef0123456789abcdef01234567",
            "0123456789abcdef",
            "12345678-1234-1234-1234-123456789abc",
        ),
        (
            "69abcdf00123456789abcdef76543210",
            "fedcba9876543210",
            "87654321-4321-4321-4321-cba987654321",
        ),
    ];

    for (index, (trace_id, parent_id, request_id)) in contexts.iter().enumerate() {
        let mut context = lambda_http::Context::default();
        context.request_id = (*request_id).into();
        context.xray_trace_id = Some(format!(
            "Root=1-{}-{};Parent={parent_id};Sampled=1",
            &trace_id[..8],
            &trace_id[8..]
        ));
        let request = request(true).with_lambda_context(context);
        let response = telemetry
            .wrap(service(Body::from("ok")))
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(
            exports.lock().unwrap().len(),
            index,
            "the current invocation cannot export before its body finishes"
        );
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 16)
                .await
                .unwrap(),
            "ok"
        );
        assert!(queue.queue.lock().unwrap().is_empty());
        let exports = exports.lock().unwrap();
        assert_eq!(exports.len(), index + 1);
        let batch = &exports[index];
        assert_eq!(batch.len(), 3);
        assert!(
            batch
                .iter()
                .all(|span| span.span_context.trace_id().to_string() == *trace_id)
        );
        let root = batch
            .iter()
            .find(|span| span.name == "lambda.invocation")
            .unwrap();
        assert_eq!(root.parent_span_id.to_string(), *parent_id);
        assert!(
            root.attributes
                .contains(&KeyValue::new("faas.coldstart", index == 0))
        );
        assert!(root.attributes.contains(&KeyValue::new(
            "faas.invocation_id",
            (*request_id).to_string()
        )));
    }

    let exports = exports.lock().unwrap();
    let root_id = |batch: &[SpanData]| {
        batch
            .iter()
            .find(|span| span.name == "lambda.invocation")
            .unwrap()
            .span_context
            .span_id()
    };
    assert_ne!(root_id(&exports[0]), root_id(&exports[1]));
}
