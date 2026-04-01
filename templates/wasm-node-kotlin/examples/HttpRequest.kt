/// HTTP Request Node — Demonstrates declaring HTTP permissions (Kotlin)
///
/// This example shows how to declare the "http" permission and use the
/// raw host import to make outbound HTTP requests from a Kotlin WASM node.
/// Copy this pattern into your Main.kt when you need network access.

package node.examples

import sdk.*
import kotlin.wasm.WasmImport

// ============================================================================
// Raw host import — the Kotlin SDK declares this privately, so examples
// wanting HTTP access need their own import or a wrapper in the SDK.
// ============================================================================

@WasmImport("flowlike_http", "request")
private external fun hostHttpRequest(
    method: Int,
    urlPtr: Int, urlLen: Int,
    headersPtr: Int, headersLen: Int,
    bodyPtr: Int, bodyLen: Int,
): Int

/// Convenience wrapper around the raw import.
fun httpRequest(method: Int, url: String, headers: String, body: String): Boolean {
    val urlBytes = url.encodeToByteArray()
    val hdrBytes = headers.encodeToByteArray()
    val bodyBytes = body.encodeToByteArray()
    val result = hostHttpRequest(
        method,
        stringToPtr(url), urlBytes.size,
        stringToPtr(headers), hdrBytes.size,
        stringToPtr(body), bodyBytes.size,
    )
    return result != -1
}

// ============================================================================
// Node definition — note `def.addPermission("http")`
// ============================================================================

fun buildHttpGetDefinition(): NodeDefinition {
    val def = NodeDefinition(
        name = "http_get_request_kt",
        friendlyName = "HTTP GET Request (Kotlin)",
        description = "Sends a GET request to a URL and reports the result",
        category = "Network/HTTP",
    )
    def.addPermission("http")

    def.addPin(PinDefinition.input("exec", "Execute", "Trigger execution", DataType.EXEC))
    def.addPin(PinDefinition.input("url", "URL", "Target URL", DataType.STRING))
    def.addPin(PinDefinition.input("headers_json", "Headers (JSON)", "Request headers as JSON", DataType.STRING))
    def.addPin(PinDefinition.output("exec_out", "Done", "Fires after the request", DataType.EXEC))
    def.addPin(PinDefinition.output("success", "Success", "Whether the HTTP call was accepted", DataType.BOOL))

    return def
}

// ============================================================================
// Run handler
// ============================================================================

fun runHttpGet(ctx: Context): ExecutionResult {
    val url = ctx.getString("url", "https://httpbin.org/get")
    val headers = ctx.getString("headers_json", "{}")

    ctx.info("Sending GET request to $url")

    // Method 0 = GET.  The host checks the "http" capability.
    val ok = httpRequest(0, url, headers, "")

    if (ok) {
        ctx.info("HTTP capability granted — request dispatched")
    } else {
        ctx.error("HTTP capability denied — is the 'http' permission declared?")
    }

    ctx.setOutput("success", if (ok) "true" else "false")
    return ctx.success()
}
