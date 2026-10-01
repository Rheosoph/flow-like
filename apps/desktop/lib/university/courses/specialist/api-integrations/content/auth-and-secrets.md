Replace the practice token pin with a runtime value.

1. In the flow variables panel, create a String variable `PRACTICE_API_TOKEN`. Enable **Runtime Configured** and **Secret**.
2. Create `PRACTICE_API_URL` as a runtime-configured String without Secret.
3. Open the app's **Runtime Variables**. Set the token to `practice-token` and the URL to `http://127.0.0.1:8765`.
4. Use the generated **Get PRACTICE_API_TOKEN** node as Set Bearer Auth's Token input. Build the customer URL from the configured base URL.
5. Run the successful request. Then set the runtime token to `wrong` and run again. Expect 401. Restore the fixture token.

The definition stores names and settings; the configured values stay on the device. Secret values are masked and excluded from remote execution payloads. Another device needs its own values.

For a real remote integration, use the credential mechanism supported by that deployment and verify it through the remote Event. Laptop runtime secrets do not become server credentials. This local fixture deliberately does not simulate that provisioning.

Log a request identifier and status, not authorization headers. Before exporting a real flow, inspect pin defaults and fixture files for accidental credentials.
