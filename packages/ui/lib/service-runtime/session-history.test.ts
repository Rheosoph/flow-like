import { describe, expect, test } from "bun:test";
import type { DBCore, DBCoreMutateRequest, DBCoreMutateResponse } from "dexie";
import { createFrontendStateStore } from "../../components/a2ui/frontend-state";
import { pageLocalState } from "../idb-storage";
import { guardRuntimeHistoryCore } from "./history-guard";
import {
	openRuntimeNamespace,
	runtimeChatSessionId,
	runtimeNamespaceActive,
	runtimeRowAppId,
} from "./session-scope";

const namespace = () => `device-runtime:${crypto.randomUUID()}`;
const mutation = (values: unknown[]): DBCoreMutateRequest => ({
	type: "put",
	values,
	trans: { abort() {} },
});

function coreHarness() {
	const writes: DBCoreMutateRequest[] = [];
	let failures: DBCoreMutateResponse["failures"] = {};
	const core = guardRuntimeHistoryCore({
		table: () => ({
			schema: { primaryKey: { extractKey: (row: { id: string }) => row.id } },
			mutate: async (request: DBCoreMutateRequest) => {
				writes.push(request);
				const results =
					"values" in request ? request.values.map((row) => row.id) : [];
				return {
					failures,
					numFailures: Object.keys(failures).length,
					results,
					lastResult: results.at(-1),
				};
			},
		}),
	} as unknown as DBCore);
	return {
		table: core.table("messages"),
		writes,
		failAt: (index: number) => {
			failures = { [index]: new Error("write refused") };
		},
	};
}

describe("deployed interface session history", () => {
	test("isolates session query IDs from Studio and other runtime chat histories", () => {
		const one = namespace();
		const two = namespace();
		const hostSession = "studio-chat-session";
		const first = runtimeChatSessionId(one, hostSession);
		const second = runtimeChatSessionId(two, hostSession);
		expect(first).toBe(`${one}:chat:${hostSession}`);
		expect(first).not.toBe(hostSession);
		expect(second).not.toBe(first);
		expect(runtimeChatSessionId(one, first)).toBe(first);
		expect(runtimeChatSessionId(two, first)).not.toBe(first);
		expect(runtimeChatSessionId("studio-project", hostSession)).toBe(
			hostSession,
		);
	});

	test("suppresses retired direct and encoded rows while preserving normal Studio writes", async () => {
		const appId = namespace();
		const retire = openRuntimeNamespace(appId);
		const { table, writes } = coreHarness();
		const row = { id: "direct", appId };
		await table.mutate(mutation([row]));
		expect(writes).toHaveLength(1);
		retire();
		const encoded = {
			id: "encoded",
			payload: JSON.stringify({ appId, body: "private" }),
		};
		expect(runtimeRowAppId(encoded)).toBe(appId);
		expect(await table.mutate(mutation([row, encoded]))).toEqual({
			numFailures: 0,
			failures: {},
			results: ["direct", "encoded"],
			lastResult: "encoded",
		});
		expect(writes).toHaveLength(1);
		const normal = { id: "normal", appId: "project" };
		expect(await table.mutate(mutation([row, normal, encoded]))).toMatchObject({
			results: ["direct", "normal", "encoded"],
			numFailures: 0,
		});
		expect(writes[1]).toMatchObject({ values: [normal] });
		expect(() => openRuntimeNamespace(appId)).toThrow("new session identity");
	});

	test("fences queued mutations at dispatch and retains original bulk error positions", async () => {
		const appId = namespace();
		const retire = openRuntimeNamespace(appId);
		const { table, writes, failAt } = coreHarness();
		const queued = Promise.resolve().then(() =>
			table.mutate(mutation([{ id: "late", appId }])),
		);
		retire();
		await queued;
		expect(writes).toHaveLength(0);
		failAt(0);
		const response = await table.mutate(
			mutation([
				{ id: "closed", appId },
				{ id: "normal", appId: "project" },
			]),
		);
		expect(response.numFailures).toBe(1);
		expect(response.failures[0]).toBeUndefined();
		expect(response.failures[1]?.message).toBe("write refused");
		await table.mutate({
			type: "delete",
			keys: ["closed"],
			trans: { abort() {} },
		});
		expect(writes.at(-1)).toMatchObject({ type: "delete", keys: ["closed"] });
	});

	test("keeps runtime frontend state in memory and clears retained store references immediately", async () => {
		const appId = namespace();
		const retire = openRuntimeNamespace(appId);
		let reads = 0;
		let writes = 0;
		const persistence = {
			global: {
				getAll: async () => {
					reads++;
					return {};
				},
				set: async () => {
					writes++;
				},
			},
			page: {
				getAll: async () => {
					reads++;
					return {};
				},
				set: async () => {
					writes++;
				},
				clearPage: async () => {
					writes++;
				},
			},
		};
		const store = createFrontendStateStore(appId, persistence);
		await store.ensureLoaded("page");
		store.setGlobalState("answer", "private");
		store.setPageState("page", "input", "private");
		await Promise.resolve();
		expect(store.getSnapshot().globalState).toEqual({ answer: "private" });
		expect([reads, writes]).toEqual([0, 0]);
		retire();
		expect(runtimeNamespaceActive(appId)).toBe(false);
		expect(store.getSnapshot()).toEqual({ globalState: {}, pageStates: {} });
		store.setGlobalState("later", "private");
		store.handleMessage({
			type: "setPageState",
			page_id: "page",
			key: "later",
			value: "private",
		});
		expect(store.getSnapshot()).toEqual({ globalState: {}, pageStates: {} });
		const late = createFrontendStateStore(appId, persistence);
		late.setGlobalState("later", "private");
		expect(late.getSnapshot().globalState).toEqual({});
	});

	test("feedback state remains scoped in memory and ignores delayed writes after close", async () => {
		const one = namespace();
		const two = namespace();
		const retireOne = openRuntimeNamespace(one);
		const retireTwo = openRuntimeNamespace(two);
		await pageLocalState.set(one, "page", "rating", "private one");
		await pageLocalState.set(two, "page", "rating", "private two");
		expect(await pageLocalState.getAll(one, "page")).toEqual({
			rating: "private one",
		});
		retireOne();
		await pageLocalState.set(one, "page", "rating", "late write");
		expect(await pageLocalState.getAll(one, "page")).toEqual({});
		expect(await pageLocalState.get<string>(two, "page", "rating")).toBe(
			"private two",
		);
		await pageLocalState.clearPage(two, "page");
		expect(await pageLocalState.getAll(two, "page")).toEqual({});
		retireTwo();
	});
});
