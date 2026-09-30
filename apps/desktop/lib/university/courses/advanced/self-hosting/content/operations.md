Record recovery and alert readiness before treating the practice deployment as a maintained service.

Backups include PostgreSQL, bundled object data and storage identity state, Redis, protected hub configuration/signing keys and `execution_manager_state`. External services need their own backup procedures. `docker compose down -v` deletes state volumes.

Before upgrading, stop new requests and event production, drain/reconcile queued and active work, and follow the [upgrade procedure](https://docs.flow-like.com/self-hosting/docker-compose/installation/#upgrade-an-existing-installation). Preserve keys and matched image pins. Replacing a container image does not imply a schema rollback.

If monitoring is enabled, confirm the queried metrics exist, an alert fires and the configured receiver receives it. A dashboard or loaded rule does not prove paging. Use the [monitoring guide](https://docs.flow-like.com/self-hosting/docker-compose/monitoring/) for the current configuration.

Make a harmless app change and inspect its Audit trail as Owner. Check that a signed, anchored record becomes exportable. Audit signing keys and policy are separate from backend execution JWTs; retain their key histories as required by their operations.

Completion: record the recovery destination, protected state inventory, observed alert result or an explicit unconfigured status, and audit export result. Do not label unverified checks ready. The optional restore lab rehearses recovery.
