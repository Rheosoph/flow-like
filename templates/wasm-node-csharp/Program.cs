using System.Text.Json;
using System.Text;
using FlowLike.Wasm.Sdk;
using FlowLike.Wasm.Node;

/// <summary>
/// WASM Component Model entry point for Flow-Like C# nodes.
///
/// Build:
///   dotnet workload install wasi-experimental
///   dotnet publish -c Release
/// </summary>

var cliArgs = Environment.GetCommandLineArgs();
if (cliArgs.Length >= 2)
{
    var command = cliArgs[1];
    if (string.Equals(command, "get-node", StringComparison.OrdinalIgnoreCase))
    {
        Console.Write(WitExports.GetNode());
        return;
    }

    if (string.Equals(command, "run", StringComparison.OrdinalIgnoreCase))
    {
        var inputJson = cliArgs.Length >= 3 ? cliArgs[2] : Console.In.ReadToEnd();
        if (string.IsNullOrWhiteSpace(inputJson))
        {
            inputJson = "{}";
        }
        Console.Write(WitExports.Run(inputJson));
        return;
    }

    if (string.Equals(command, "run-b64", StringComparison.OrdinalIgnoreCase))
    {
        var encoded = cliArgs.Length >= 3 ? cliArgs[2] : string.Empty;
        var inputJson = string.IsNullOrWhiteSpace(encoded)
            ? "{}"
            : Encoding.UTF8.GetString(Convert.FromBase64String(encoded));
        Console.Write(WitExports.Run(inputJson));
        return;
    }

    if (string.Equals(command, "get-abi-version", StringComparison.OrdinalIgnoreCase))
    {
        Console.Write(WitExports.GetAbiVersion());
        return;
    }
}

// Default startup path for wasm runtimes that execute the command world directly.
_ = WitExports.GetAbiVersion();

public static class WitExports
{
    public static string GetNode()
    {
        var definition = CustomNode.GetDefinition();
        return Json.Serialize(new[] { definition.ToDictionary() });
    }

    public static string GetNodes()
    {
        var definition = CustomNode.GetDefinition();
        return Json.Serialize(new[] { definition.ToDictionary() });
    }

    public static string Run(string inputJson)
    {
        var ctx = Context.FromJson(inputJson);
        var result = CustomNode.Run(ctx);
        return result.ToJson();
    }

    public static int GetAbiVersion() => 1;
}
