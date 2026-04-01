using FlowLike.Wasm.Sdk;

namespace FlowLike.Wasm.Node;

public static class CustomNode
{
    public static NodeDefinition GetDefinition()
    {
        var nd = new NodeDefinition(
            name: "my_custom_node_csharp",
            friendlyName: "My Custom Node",
            description: "A template WASM node that demonstrates basic functionality",
            category: "Custom/WASM"
        );
        nd.AddPermission("streaming");

        nd.AddPin(PinDefinition.InputExec("exec"));
        nd.AddPin(PinDefinition.InputPin("input_text", PinType.String, defaultValue: ""));
        nd.AddPin(PinDefinition.InputPin("multiplier", PinType.I64, defaultValue: 1));

        nd.AddPin(PinDefinition.OutputExec("exec_out"));
        nd.AddPin(PinDefinition.OutputPin("output_text", PinType.String));
        nd.AddPin(PinDefinition.OutputPin("char_count", PinType.I64));

        return nd;
    }

    public static ExecutionResult Run(Context ctx)
    {
        var inputText = ctx.GetString("input_text", "") ?? "";
        var multiplier = ctx.GetI64("multiplier", 1) ?? 1;

        ctx.Debug($"Processing: '{inputText}' x {multiplier}");

        var repeated = multiplier > 0 ? string.Concat(Enumerable.Repeat(inputText, (int)multiplier)) : "";
        var charCount = repeated.Length;

        ctx.StreamText($"Generated {charCount} characters");

        ctx.SetOutput("output_text", repeated);
        ctx.SetOutput("char_count", charCount);

        return ctx.Success();
    }
}
