import { afterEach, expect, test, vi } from "vitest";

vi.mock("./runtime-client", () => {
	throw new Error("The shared worker imported its browser worker factory");
});
vi.mock("../execution/runtime-client", () => {
	throw new Error("The shared worker imported the browser execution client");
});
vi.mock("../execution/chat-persistence", () => ({
	persistExecutionChat: async () => {},
}));
vi.mock("../channel/transport", () => ({ rawPushChannel: async () => {} }));

afterEach(() => vi.unstubAllGlobals());

test("the shared worker starts without importing browser worker factories", async () => {
	vi.stubGlobal("onconnect", undefined);
	await import("./runtime.shared-worker");
	expect(
		(globalThis as unknown as { onconnect: unknown }).onconnect,
	).toBeTypeOf("function");
});
