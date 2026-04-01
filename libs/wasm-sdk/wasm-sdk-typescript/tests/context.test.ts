import { beforeEach, describe, expect, it } from "vitest";
import { Context } from "../src/context";
import { MockHostBridge, setHost } from "../src/host";
import { ExecutionInput, LogLevel } from "../src/types";

function makeInput(overrides: Record<string, unknown> = {}): ExecutionInput {
	return ExecutionInput.fromDict({
		inputs: { text: "hello", count: 3, flag: true, ratio: 0.5 },
		node_id: "node-1",
		run_id: "run-1",
		app_id: "app-1",
		board_id: "board-1",
		user_id: "user-1",
		stream_state: false,
		log_level: LogLevel.DEBUG,
		node_name: "TestNode",
		...overrides,
	});
}

describe("Context", () => {
	let host: MockHostBridge;
	let ctx: Context;

	beforeEach(() => {
		host = new MockHostBridge();
		setHost(host);
		ctx = new Context(makeInput(), host);
	});

	describe("construction", () => {
		it("creates from ExecutionInput", () => {
			expect(ctx.nodeId).toBe("node-1");
			expect(ctx.runId).toBe("run-1");
			expect(ctx.appId).toBe("app-1");
			expect(ctx.boardId).toBe("board-1");
			expect(ctx.userId).toBe("user-1");
			expect(ctx.nodeName).toBe("TestNode");
		});

		it("creates from dict", () => {
			const ctx2 = Context.fromDict({ node_id: "n2", inputs: {} }, host);
			expect(ctx2.nodeId).toBe("n2");
		});

		it("creates from JSON", () => {
			const json = JSON.stringify({ node_id: "n3", inputs: { x: 1 } });
			const ctx3 = Context.fromJSON(json, host);
			expect(ctx3.nodeId).toBe("n3");
			expect(ctx3.getInput("x")).toBe(1);
		});
	});

	describe("input getters", () => {
		it("gets raw input", () => {
			expect(ctx.getInput("text")).toBe("hello");
			expect(ctx.getInput("missing")).toBeNull();
		});

		it("gets string input", () => {
			expect(ctx.getString("text")).toBe("hello");
			expect(ctx.getString("missing", "default")).toBe("default");
			expect(ctx.getString("missing")).toBeUndefined();
		});

		it("gets i64 input", () => {
			expect(ctx.getI64("count")).toBe(3);
			expect(ctx.getI64("missing", 99)).toBe(99);
			expect(ctx.getI64("missing")).toBeUndefined();
		});

		it("gets f64 input", () => {
			expect(ctx.getF64("ratio")).toBe(0.5);
			expect(ctx.getF64("missing", 1.0)).toBe(1.0);
		});

		it("gets bool input", () => {
			expect(ctx.getBool("flag")).toBe(true);
			expect(ctx.getBool("missing", false)).toBe(false);
		});

		it("requires input, throwing if missing", () => {
			expect(ctx.requireInput("text")).toBe("hello");
			expect(() => ctx.requireInput("missing")).toThrow(
				"Required input 'missing' not provided",
			);
		});
	});

	describe("output setters", () => {
		it("sets output values", () => {
			ctx.setOutput("result", 42);
			const result = ctx.finish();
			expect(result.outputs.result).toBe(42);
		});

		it("activates exec pins", () => {
			ctx.activateExec("branch_a");
			const result = ctx.finish();
			expect(result.activateExec).toContain("branch_a");
		});

		it("sets pending", () => {
			ctx.setPending(true);
			const result = ctx.finish();
			expect(result.pending).toBe(true);
		});
	});

	describe("logging", () => {
		it("logs debug when level allows", () => {
			ctx.debug("dbg msg");
			expect(host.logs).toHaveLength(1);
			expect(host.logs[0]).toEqual([LogLevel.DEBUG, "dbg msg"]);
		});

		it("skips debug when log level is higher", () => {
			const ctx2 = new Context(makeInput({ log_level: LogLevel.WARN }), host);
			ctx2.debug("should not appear");
			ctx2.info("should not appear either");
			ctx2.warn("should appear");
			expect(host.logs).toHaveLength(1);
			expect(host.logs[0][0]).toBe(LogLevel.WARN);
		});

		it("logs all levels", () => {
			ctx.debug("d");
			ctx.info("i");
			ctx.warn("w");
			ctx.error("e");
			expect(host.logs).toHaveLength(4);
		});
	});

	describe("streaming", () => {
		it("does not stream when disabled", () => {
			ctx.streamText("hi");
			ctx.streamJSON({ x: 1 });
			ctx.streamProgress(50, "halfway");
			expect(host.streams).toHaveLength(0);
		});

		it("streams when enabled", () => {
			const ctx2 = new Context(makeInput({ stream_state: true }), host);
			ctx2.streamText("hello");
			ctx2.streamJSON({ x: 1 });
			ctx2.streamProgress(75, "almost");

			expect(host.streams).toHaveLength(3);
			expect(host.streams[0]).toEqual(["text", "hello"]);
			expect(host.streams[1][0]).toBe("json");
			expect(JSON.parse(host.streams[1][1])).toEqual({ x: 1 });
			expect(host.streams[2][0]).toBe("progress");
		});
	});

	describe("variables", () => {
		it("gets and sets variables via host", () => {
			ctx.setVariable("myvar", { data: [1, 2] });
			expect(ctx.getVariable("myvar")).toEqual({ data: [1, 2] });
		});
	});

	describe("storage", () => {
		it("returns storage directories", () => {
			expect(ctx.storageDir()).not.toBeNull();
			expect(ctx.uploadDir()).not.toBeNull();
			expect(ctx.cacheDir()).not.toBeNull();
			expect(ctx.userDir()).not.toBeNull();
		});

		it("reads and writes storage", () => {
			const dir = ctx.storageDir()!;
			const path = { ...dir, path: `${dir.path}/test.bin` };
			const data = new Uint8Array([10, 20, 30]);

			expect(ctx.storageRead(path)).toBeNull();
			expect(ctx.storageWrite(path, data)).toBe(true);
			expect(ctx.storageRead(path)).toEqual(data);
		});

		it("lists storage entries", () => {
			const dir = ctx.storageDir()!;
			host.storage[`${dir.path}/a.txt`] = new Uint8Array([1]);
			const list = ctx.storageList(dir);
			expect(list).not.toBeNull();
			expect(list!.length).toBeGreaterThanOrEqual(1);
		});
	});

	describe("embeddings", () => {
		it("embeds text via host", () => {
			const result = ctx.embedText({}, ["hello"]);
			expect(result).not.toBeNull();
			expect(result!).toHaveLength(1);
			expect(result![0]).toEqual([0.1, 0.2, 0.3]);
		});
	});

	describe("finalization", () => {
		it("success activates exec_out", () => {
			ctx.setOutput("val", "ok");
			const result = ctx.success();
			expect(result.activateExec).toContain("exec_out");
			expect(result.outputs.val).toBe("ok");
			expect(result.error).toBeNull();
		});

		it("fail sets error", () => {
			const result = ctx.fail("something went wrong");
			expect(result.error).toBe("something went wrong");
		});

		it("finish returns raw result", () => {
			ctx.setOutput("x", 1);
			ctx.activateExec("custom_out");
			const result = ctx.finish();
			expect(result.outputs.x).toBe(1);
			expect(result.activateExec).toContain("custom_out");
			expect(result.activateExec).not.toContain("exec_out");
		});
	});
});
