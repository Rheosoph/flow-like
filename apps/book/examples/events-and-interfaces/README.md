# Events and interfaces fixture

This Chapter 13 fixture keeps one deterministic incident decision in a typed Function and adapts
it to two different boundaries: a Simple Event suitable for a Quick Action or Cron App Event, and
a REST setup Event that registers one Generic handler, API-key authentication, and OpenAPI routes.

The REST App Event must target `configureIncidentRest`, use Remote execution, and receive the
secret `incidentApiKey` as an App Event variable override. Saving the App Event runs the setup Flow
and persists the registered `/triage`, `/openapi.json`, and `/docs` routes.

The fixture is parser-tested as canonical FlowScript. Catalog-aware setup and inbound integration
tests remain in the Web catalog and API crates; the book states the current gaps around remote
OpenAPI schemas, edge validation, rejected requests, and canary dispatch explicitly.
