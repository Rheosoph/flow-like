package com.example.node;

import com.flowlike.sdk.*;
import org.teavm.interop.Export;

public class Node {

    private static Types.NodeDefinition buildDefinition() {
        Types.NodeDefinition def = new Types.NodeDefinition();
        def.setName("my_custom_node_java")
           .setFriendlyName("My Custom Node (Java)")
           .setDescription("A template WASM node built with Java (TeaVM)")
           .setCategory("Custom/WASM");

        def.addPin(Types.inputExec("exec"));
        def.addPin(Types.inputPin("input_text", "Input Text", "Text to process", Types.DATA_TYPE_STRING).withDefault("\"\""));
        def.addPin(Types.inputPin("multiplier", "Multiplier", "Number of times to repeat", Types.DATA_TYPE_I64).withDefault("1"));

        def.addPin(Types.outputExec("exec_out"));
        def.addPin(Types.outputPin("output_text", "Output Text", "Processed text", Types.DATA_TYPE_STRING));
        def.addPin(Types.outputPin("char_count", "Character Count", "Number of characters in output", Types.DATA_TYPE_I64));

        return def;
    }

    private static Types.ExecutionResult handleRun(Context ctx) {
        String inputText = ctx.getString("input_text", "");
        long multiplier = ctx.getI64("multiplier", 1);

        ctx.debug("Processing: '" + inputText + "' x " + multiplier);

        StringBuilder sb = new StringBuilder();
        for (long i = 0; i < multiplier; i++) {
            sb.append(inputText);
        }
        String outputText = sb.toString();
        int charCount = outputText.length();

        ctx.streamText("Generated " + charCount + " characters");

        ctx.setOutput("output_text", Json.quote(outputText));
        ctx.setOutput("char_count", String.valueOf(charCount));

        return ctx.success();
    }

    @Export(name = "get_node")
    public static long getNode() {
        return Memory.serializeDefinition(buildDefinition());
    }

    @Export(name = "get_nodes")
    public static long getNodes() {
        Types.NodeDefinition def = buildDefinition();
        return Memory.packResult("[" + def.toJson() + "]");
    }

    @Export(name = "run")
    public static long run(int ptr, int len) {
        Types.ExecutionInput input = Memory.parseInput(ptr, len);
        Context ctx = new Context(input);
        Types.ExecutionResult result = handleRun(ctx);
        return Memory.serializeResult(result);
    }

    /** Required by TeaVM WASI target: _start calls main() during initialization. */
    public static void main(String[] args) {
        // No-op: node logic is invoked via the exported get_node / run functions.
    }
}
