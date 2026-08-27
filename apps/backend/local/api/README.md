# Local API

The local API listens on `http://localhost:8080`, which is also the default
proxy base for hosted model calls made inside API routes. Set `API_BASE_URL` in
`.env` when the API listens on another address. Upstream provider credentials
belong in this API environment and must not be copied into the runtime
environment.
