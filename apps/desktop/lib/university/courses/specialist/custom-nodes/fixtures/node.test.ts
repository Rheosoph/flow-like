import { Context, ExecutionInput, LogLevel, MockHostBridge, setHost } from "@flow-like/wasm-sdk-typescript";
import { describe, expect, it } from "vitest";
import { getDefinition, run } from "../src/node";

function context(inputs: Record<string, unknown>) {
  const host = new MockHostBridge();
  setHost(host);
  return new Context(ExecutionInput.fromDict({ inputs, node_id: "practice-node", run_id: "practice-run", app_id: "practice-app", board_id: "practice-board", user_id: "practice-user", stream_state: false, log_level: LogLevel.DEBUG, node_name: "normalize_text" }), host);
}
describe("normalize_text", () => {
  it("preserves the serialized node and pin contract", () => {
    const definition = getDefinition();
    expect(definition.toDict().name).toBe("normalize_text");
    expect(definition.pins.map(pin => pin.name)).toEqual(["exec", "text", "trim", "lowercase", "exec_out", "result", "changed"]);
  });
  it.each([
    [{ text: "  Mixed CASE  " }, "mixed case", true],
    [{ text: "clean" }, "clean", false],
    [{ text: "   " }, "", true],
    [{ text: "" }, "", false],
    [{}, "", false],
    [{ text: " Ab ", trim: false }, " ab ", true],
    [{ text: " Ab ", lowercase: false }, "Ab", true],
    [{ text: " Ab ", trim: false, lowercase: false }, " Ab ", false],
  ])("handles %j", (inputs, expected, changed) => {
    const output = run(context(inputs));
    expect(output.outputs.result).toBe(expected);
    expect(output.outputs.changed).toBe(changed);
    expect(output.activateExec).toContain("exec_out");
  });
});
