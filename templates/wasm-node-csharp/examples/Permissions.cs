/// HTTP Request Example — Demonstrates how to declare permissions and make HTTP requests
///
/// Permissions tell the runtime which host capabilities your node requires.
/// When users place the node, they see the requested permissions and must
/// consent before execution.
///
/// This example declares the "http" permission and makes a GET request
/// to a public API.

using FlowLike.Wasm.Sdk;

public static class HttpRequestExample
{
    // ========================================================================
    // Node definition — note nd.AddPermission("http")
    // ========================================================================

    public static NodeDefinition GetDefinition()
    {
        var nd = new NodeDefinition(
            name: "http_request_example_csharp",
            friendlyName: "HTTP Request Example",
            description: "Fetches data from a public API using HTTP",
            category: "Examples/HTTP");

        nd.AddPermission("http");

        nd.AddPin(PinDefinition.InputExec("exec"));
        nd.AddPin(PinDefinition.InputPin("url", PinType.String,
            defaultValue: "https://httpbin.org/get"));
        nd.AddPin(PinDefinition.OutputExec("exec_out"));
        nd.AddPin(PinDefinition.OutputPin("status", PinType.I64));
        nd.AddPin(PinDefinition.OutputPin("body", PinType.String));

        return nd;
    }

    // ========================================================================
    // Run handler — makes an HTTP GET request
    // ========================================================================

    public static ExecutionResult Run(Context ctx)
    {
        var url = ctx.GetString("url", "https://httpbin.org/get") ?? "https://httpbin.org/get";

        ctx.Info($"Making GET request to: {url}");

        var response = ctx.HttpGet(url);
        if (response == null)
        {
            return ctx.Fail("HTTP request failed or permission denied");
        }

        ctx.SetOutput("status", response.GetValueOrDefault("status"));
        ctx.SetOutput("body", response.GetValueOrDefault("body"));
        ctx.Info($"Response status: {response.GetValueOrDefault("status")}");

        return ctx.Success();
    }
}
