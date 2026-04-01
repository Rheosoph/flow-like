/// HTTP Request Node — Demonstrates declaring HTTP permissions (MoonBit)
///
/// This example shows how to declare the "http" permission and use the
/// host function to make outbound HTTP requests from a MoonBit WASM node.
/// Copy this pattern into your node.mbt when you need network access.

// ============================================================================
// Node definition — note `def.add_permission("http")`
// ============================================================================

fn build_http_get_definition() -> @sdk.NodeDefinition {
  let def = @sdk.NodeDefinition::new(
    "http_get_request_moonbit",
    "HTTP GET Request (MoonBit)",
    "Sends a GET request to a URL and reports the result",
    "Network/HTTP",
  )
  def.add_permission("http")

  def.add_pin(@sdk.input_pin("exec", "Execute", "Trigger execution", @sdk.DataType::Exec))
  def.add_pin(
    @sdk.input_pin("url", "URL", "Target URL", @sdk.DataType::StringType)
      .with_default("\"https://httpbin.org/get\""),
  )
  def.add_pin(
    @sdk.input_pin("headers_json", "Headers (JSON)", "Request headers as JSON", @sdk.DataType::StringType)
      .with_default("\"{}\""),
  )
  def.add_pin(@sdk.output_pin("exec_out", "Done", "Fires after the request", @sdk.DataType::Exec))
  def.add_pin(@sdk.output_pin("response", "Response", "Response body", @sdk.DataType::StringType))

  def
}

// ============================================================================
// Run handler
// ============================================================================

fn handle_http_get(ctx : @sdk.Context) -> @sdk.ExecutionResult {
  let url = ctx.get_string("url", default="https://httpbin.org/get")
  let _headers = ctx.get_string("headers_json", default="{}")

  ctx.info("Sending GET request to " + url)

  match ctx.http_get(url) {
    Some(resp) => {
      ctx.info("HTTP request succeeded")
      ctx.set_output("response", @sdk.json_string(resp))
    }
    None => {
      ctx.error("HTTP request failed — is the 'http' permission declared?")
      ctx.set_output("response", @sdk.json_string(""))
    }
  }

  ctx.success()
}
