Use this class when a consumer needs more than one webhook.

An API Event exposes one configured HTTP endpoint. REST exposes a broader authenticated service; MCP exposes tools to compatible clients. REST and MCP require a supported server deployment. Internal exposure serves connected apps through their connection infrastructure; public exposure needs an explicit caller policy.

Open the app's **Endpoints** page. Inspect the app-scoped paths, SDK examples and full API reference. Use the actual generated examples for your deployment.

| Credential | Appropriate owner |
| --- | --- |
| Personal Access Token | A person's development or personal script |
| Technical User API Key | An app-scoped service integration |

For a practice handoff, record the consumer, permitted operations, credential owner, endpoint and revocation procedure. Create a least-privileged test technical user only if your app role allows it. Call one read-only endpoint with its key, then revoke that test key and confirm access fails. Do not include the key in your evidence.

Use the Release and Rollback course to pin and test the flow contract before another system depends on its shape. A key authenticates a caller; it does not freeze the implementation or define which result fields remain compatible.
