import { type Context, type ExecutionResult, NodeDefinition, PinDefinition, PinType } from "@flow-like/wasm-sdk-typescript";

export function getDefinition(): NodeDefinition {
  const node = new NodeDefinition("normalize_text", "Normalize Text", "Trims and lowercases text", "Custom/Text");
  node.addPin(PinDefinition.inputExec("exec"));
  node.addPin(PinDefinition.inputPin("text", PinType.STRING, { defaultValue: "" }));
  node.addPin(PinDefinition.inputPin("trim", PinType.BOOL, { defaultValue: true }));
  node.addPin(PinDefinition.inputPin("lowercase", PinType.BOOL, { defaultValue: true }));
  node.addPin(PinDefinition.outputExec("exec_out"));
  node.addPin(PinDefinition.outputPin("result", PinType.STRING));
  node.addPin(PinDefinition.outputPin("changed", PinType.BOOL));
  return node;
}

export function run(ctx: Context): ExecutionResult {
  const text = ctx.getString("text", "") ?? "";
  let result = text;
  if (ctx.getBool("trim", true) ?? true) result = result.trim();
  if (ctx.getBool("lowercase", true) ?? true) result = result.toLowerCase();
  ctx.setOutput("result", result);
  ctx.setOutput("changed", result !== text);
  return ctx.success();
}
