Call a Flow-Like Event from a small client program. Prerequisites: an online app with a working remotely executable Generic Event, a scoped app API key, and Python or TypeScript tooling. Choose one language; both use the same fixture.

Build the harmless Event in your practice app. Its payload has string fields `request_id` and `message`. Trim and lowercase message, then use **Return Generic Result** to return the normalized text with the request ID. Confirm a manual call returns `ready` for `  READY  `. Pin the Event to the tested board version.

Download the payload and your chosen client:

@SDKRequest
@PythonClient
@TypeScriptClient
@NodePackage

Record your actual app ID and Event ID. These fixtures contain no deployed IDs. Supply `FLOW_LIKE_BASE_URL` as the API origin without `/api/v1`, `FLOW_LIKE_API_KEY` through your secret configuration, and `APP_ID`/`EVENT_ID` as environment variables. The SDK adds `/api/v1` and chooses the authentication header for the credential.

Completion: the Event already works, the key has only required app permissions, and your client uses the intended API origin. See the [platform SDK overview](https://docs.flow-like.com/dev/sdks/overview/). This is the API client SDK, separate from custom-node WASM SDKs.
