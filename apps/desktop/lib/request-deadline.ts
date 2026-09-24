/**
 * Desktop requests go through `@tauri-apps/plugin-http`, whose Rust side honours
 * only `connectTimeout` — `request.send()` on a half-open socket waits forever.
 * Aborting a deadline's signal reaches Rust as `plugin:http|fetch_cancel` and
 * frees the connection.
 */
export * from "@flow-like/flow-like-ui/lib/request-deadline";
