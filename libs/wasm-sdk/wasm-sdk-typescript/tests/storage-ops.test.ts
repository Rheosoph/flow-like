import { describe, expect, it, vi } from "vitest";
import { Context } from "../src/context";
import { MockHostBridge, type FlowPath, type HostBridge } from "../src/host";
import { MockStorageOps } from "../src/storage-ops-mock";
import {
	createStorageOpsBridge,
	StorageError,
	writeStorageChunks,
	type StorageOpsImports,
} from "../src/storage-ops";

const path = (name: string, store = "mock"): FlowPath => ({
	path: name,
	store_ref: store,
	cache_store_ref: null,
});
const bytes = (text: string) => new TextEncoder().encode(text);

describe("typed storage", () => {
	it("does not require storageOps on an existing bridge", () => {
		const legacy: HostBridge = new MockHostBridge();
		legacy.storageOps = undefined;
		const context = Context.fromDict({}, legacy);
		expect(() => context.storageOps).toThrowError(
			expect.objectContaining({ code: "unsupported" }),
		);
	});

	it("uses metadata preconditions for writes and range reads", () => {
		const storage = new MockStorageOps();
		const file = path("project/a");
		const stat = storage.write(file, bytes("abcdef"), { create_only: true });
		expect(storage.readRange(file, 1n, 3n, stat.e_tag!)).toEqual(bytes("bcd"));
		expect(() =>
			storage.write(file, bytes("x"), { create_only: true }),
		).toThrowError(expect.objectContaining({ code: "conflict" }));
		storage.write(file, bytes("new"), { if_match: stat.e_tag! });
		expect(() => storage.readRange(file, 0, 1, stat.e_tag!)).toThrowError(
			expect.objectContaining({ code: "conflict" }),
		);
		expect(() => storage.stat(path("project/a", "other"))).toThrowError(
			expect.objectContaining({ code: "not_found" }),
		);
	});

	it("requires exact streamed size and aborts when the iterable throws", () => {
		const storage = new MockStorageOps();
		const file = path("a");
		const id = storage.writeStart(file, 3);
		storage.writeChunk(id, bytes("ab"));
		expect(() => storage.writeFinish(id)).toThrowError(
			expect.objectContaining({ code: "incomplete_write" }),
		);
		expect(() => storage.stat(file)).toThrowError(
			expect.objectContaining({ code: "not_found" }),
		);
		function* failing() {
			yield bytes("ab");
			throw new Error("source failed");
		}
		expect(() => writeStorageChunks(storage, file, failing())).toThrow(
			"source failed",
		);
		expect(storage.pendingWrites).toBe(0);
		expect(
			writeStorageChunks(storage, file, [bytes("ab"), bytes("cd")], 4).size,
		).toBe(4);
	});

	it("scopes, filters, consumes and closes pagination cursors", () => {
		const storage = new MockStorageOps();
		for (const name of [
			"project/a",
			"project/b",
			"project/node_modules/c",
			"project-other/a",
		])
			storage.write(path(name), bytes("x"));
		const request = {
			path: path("project"),
			limit: 1,
			exclude_directories: ["node_modules"],
		};
		const first = storage.listPage(request);
		expect(first.entries.map((file) => file.path)).toEqual(["project/a"]);
		expect(() =>
			storage.listPage({
				...request,
				path: path("other"),
				cursor: first.cursor!,
			}),
		).toThrowError(expect.objectContaining({ code: "invalid_cursor" }));
		const second = storage.listPage({ ...request, cursor: first.cursor! });
		expect(second.entries.map((file) => file.path)).toEqual(["project/b"]);
		expect(second.cursor).toBeNull();
		expect(() =>
			storage.listPage({ ...request, cursor: first.cursor! }),
		).toThrowError(expect.objectContaining({ code: "invalid_cursor" }));
		const reopened = storage.listPage(request);
		storage.closeList(reopened.cursor!);
		expect(storage.openListings).toBe(0);
	});

	it("copies and moves without silently overwriting a destination", () => {
		const storage = new MockStorageOps();
		storage.write(path("a"), bytes("source"));
		storage.write(path("b"), bytes("destination"));
		expect(() => storage.copy(path("a"), path("b"))).toThrowError(
			expect.objectContaining({ code: "conflict" }),
		);
		storage.move(path("a"), path("b"), true);
		expect(storage.readRange(path("b"), 0, 100)).toEqual(bytes("source"));
		expect(() => storage.stat(path("a"))).toThrowError(
			expect.objectContaining({ code: "not_found" }),
		);
	});

	it("preserves WIT structured failures and converts u64 without loss", () => {
		const readFileRange = vi.fn(() => bytes("ok"));
		const imports = {
			statFile: () => {
				throw {
					payload: { code: "io", message: "temporary", retryable: true },
				};
			},
			readFileRange,
		} as unknown as StorageOpsImports;
		const storage = createStorageOpsBridge(imports);
		expect(() => storage.stat(path("a"))).toThrowError(
			expect.objectContaining({ code: "io", retryable: true }),
		);
		expect(storage.readRange(path("a"), 10, 2n)).toEqual(bytes("ok"));
		expect(readFileRange).toHaveBeenCalledWith(
			JSON.stringify(path("a")),
			10n,
			2n,
			undefined,
		);
		expect(() =>
			storage.readRange(path("a"), Number.MAX_SAFE_INTEGER + 1, 1),
		).toThrow(StorageError);
	});
});
