Use the actual Audit trail page to investigate a change. This exercise requires the app's Owner account. Its signing configuration and worker must be running for sealed records to become exportable.

1. Make one harmless membership or role change in the practice app. Note its time and affected resource.
2. Open app settings → **Audit trail**. Find the matching evidence record by actor, action and resource. A new record may be Pending while it waits to be sealed.
3. Select **Verify chain**. Inspect the checked sequence range. Use **Re-check from the first seal** when you need a complete reread of retained seals.
4. Under **Chain head**, use **Copy head JSON**. Store it outside Flow-Like, then use **Check a saved head** to verify that checkpoint.
5. Export sealed, anchored evidence through the pull endpoint below. Use your own app ID and an Owner PAT supplied through the environment.

```bash
curl --fail --silent --show-error   -H "Authorization: $FLOW_LIKE_PAT"   "$FLOW_LIKE_BASE_URL/api/v1/apps/$APP_ID/audit/export?class=evidence&after_seq=0&limit=100"   -o evidence.ndjson
```

`FLOW_LIKE_BASE_URL` is the API origin. For multiple pages, continue from the `X-FlowLike-Audit-Next-Seq` response header. An empty export can mean the record has not yet been anchored; a self-hosted platform without an audit key never anchors exports. Check that state instead of assuming no change occurred.

Completion: retain the matched record identifier, verification result, saved head and export. A redacted value can still verify because the chain covers its commitment. See [Audit trail and export](https://docs.flow-like.com/apps/audit-trail/) for retention, webhook delivery and offline verification.
