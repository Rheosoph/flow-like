# Local runtime

Run the local API on `http://localhost:8080` before starting this runtime. The
runtime receives the API callback address with each authenticated run and sends
hosted completion and remote embedding requests through that API. The local
fallback is `http://localhost:8080`. Provider credentials belong on the API
process because the runtime never calls hosted providers directly.

Change `API_BASE_URL` when the API listens on another address. `API_URL` remains
accepted as a compatibility alias, but new deployments should use
`API_BASE_URL`.
