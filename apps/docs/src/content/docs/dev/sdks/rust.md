---
title: Rust SDK
description: Publish workflows, invoke Events, inspect executions, and manage data with the async Rust platform client.
sidebar:
  order: 3
---

Use `flow-like-platform` to call the Flow-Like REST API from a Rust application.
The crate uses HTTP and does not load the workflow engine or its model runtimes.
It is separate from the [WASM node SDK](/dev/wasm-nodes/rust/), which builds nodes
that execute inside a workflow.

## Add the source dependency

The crate is available in this repository. Until it is published, use a path
dependency pointing at your checkout:

```toml
[dependencies]
flow-like-platform = { path = "../flow-like/libs/platform/rust" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Inside the Flow-Like workspace, use `flow-like-platform = { workspace = true }`.

## Create a client

Set `FLOW_LIKE_BASE_URL` and one of `FLOW_LIKE_PAT` or `FLOW_LIKE_API_KEY`, then
call `Client::from_env()`. For explicit configuration:

```rust
use flow_like_platform::{Auth, Client};
use std::time::Duration;

let client = Client::builder(
    "https://api.flow-like.com",
    Auth::PersonalAccessToken(std::env::var("FLOW_LIKE_PAT")?),
)
.timeout(Duration::from_secs(300))
.build()?;
```

The client preserves a configured URL prefix and adds `/api/v1` when absent.
The request timeout also applies to streaming responses, so choose a duration
that covers the workflow or use asynchronous invocation and polling.

## Publish and invoke an Event

Publish the saved board, pin the Event to that version, and invoke it. The
publish call reuses the current version when the board has not changed.

```rust
use flow_like_platform::{Client, InvokeEventRequest, RequestOptions, WaitOptions, json};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::from_env()?;
    let options = RequestOptions::default();
    let published = client
        .publish_board_if_changed("app-id", "board-id", &options)
        .await?;
    let mut event = client.get_event("app-id", "event-id", &options).await?;
    event["board_version"] = published["version"].clone();
    client.upsert_event(
        "app-id", "event-id",
        &RequestOptions::new().json(json!({"event": event})),
    ).await?;

    let run = client.trigger_event_async(
        "app-id", "event-id",
        &InvokeEventRequest {
            payload: Some(json!({"message": "Hello"})),
            ..Default::default()
        },
        &options,
    ).await?;
    let completed = client
        .wait_for_execution(&run.poll_token, &WaitOptions::default())
        .await?;
    println!("{}: {}", completed.run_id, completed.status);
    Ok(())
}
```

For incremental progress, call `poll_execution()` with `PollOptions` and advance
its `after_sequence` cursor using `PollResponse::last_sequence()`. The default
cursor is `-1`, which includes event sequence `0`. Polling sends only the returned
poll token as Bearer authorization. `cancel_run()` uses your platform credential
and the backend's cancellation permissions.

`trigger_event()` and `invoke_board()` return an `EventStream`. Use
`futures_util::StreamExt::next()` to read it. Dropping the stream stops that HTTP
read; cancelling the workflow itself requires `cancel_run()`.

## Management requests

Management methods take resource IDs and `RequestOptions`. Each ID is encoded
as one URL path segment. JSON bodies and query names follow the backend contract:

```rust
use flow_like_platform::{RequestOptions, json};

let runs = client.list_runs(
    "app-id", "board-id", &RequestOptions::new().query("limit", 20),
).await?;
let logs = client.get_run_logs(
    "app-id", "board-id",
    &RequestOptions::new().json(json!({
        "run_id": "run-id",
        "query": {"levels": [2, 3, 4]},
        "offset": 0,
        "limit": 50,
    })),
).await?;
```

The client includes app, board, Event, run history, database, page, route, widget,
connection, package, role, team, API-key, and device hub methods. Flexible domain
documents return `serde_json::Value`; invocation and polling use typed structures.
For a route without a convenience method, `request()` and `request_raw()` accept
an HTTP method and a slice of path segments, using the same transport.

Device hub calls manage inventory and approvals. Encrypted device commands and
device-local execution logs use the separate `flow-like-device-client` crate or
the [Studio device views](/devices/service-access/).

## Files and errors

`upload_file()` and `download_file()` obtain signed storage URLs and transfer the
bytes. `sign_uploads()` and `sign_downloads()` return grants when another process
should perform the transfer. Uploads support signed PUT requests and multipart
POST grants. Platform credentials are never added to storage requests, and
redirects are not followed. The `user_scope` argument selects private user storage.

Errors distinguish configuration, transport, API, JSON, stream, execution, and
wait-timeout failures. `Error::Api` includes the status, response body, and any
request ID or retry-after header. `is_auth()` and `is_not_found()` identify common
recovery cases. Requests are not automatically retried, because repeating a
mutation could create an additional run or resource.
