Optional: restore a synthetic app onto a second disposable Linux host. Prepare both hosts and a host/volume backup tool first. Use the same deployment revision and image digests, compatible storage layout, and no external action Events.

Create the supplied file, table row and harmless Flow in the source app:

@RestoreProbe

## Capture a consistent recovery point

1. Stop new requests and scheduled event production. Confirm queued and active runs have settled.
2. Record file text, row count/value, Flow result and an external Audit trail head.
3. Run `docker compose stop` and verify all Compose services stopped.
4. Use your host/volume backup tool to capture all project state volumes: PostgreSQL, Redis, object data/logs, execution-manager state and enabled monitoring volumes. Capture protected `.env`, hub JSON and the pinned deployment revision with them.

Use the checked-in Compose `volumes` section and your project's allocated volume names to verify coverage. Keep backup access as restricted as the source instance.

## Restore and verify

5. Restore that recovery point to the isolated second host with the backup tool's restore operation. Preserve keys and storage identity state. Deliberately update host reachability/OIDC origins without replacing restored credentials.
6. Run preflight and start services. Keep outside traffic and action Events disabled during verification.
7. Sign in, read the marker file, query the probe row and run the harmless Flow. Expected values are in the fixture.
8. Verify audit evidence and check the saved head. Resolve mismatches before admitting traffic.

Completion requires a restored instance with matching results. Record the actual backup/restore commands or provider operations and elapsed recovery time. Tooling varies by host; exercise the mechanism you will operate. External storage/database setups substitute coordinated provider restores. An archive alone is insufficient evidence of recovery.
