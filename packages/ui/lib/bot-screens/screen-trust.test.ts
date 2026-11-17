import "fake-indexeddb/auto";
import { describe, expect, test } from "bun:test";
import {
	type TrustedHost,
	indexedDbTrustStore,
	memoryTrustStore,
} from "./screen-trust";

const host = (keyId: string): TrustedHost => ({
	key_id: keyId,
	public_key: `pk-${keyId}`,
	label: "Studio Mac",
	host_peer_id: "hostpeer0123",
	paired_at: 1763337600,
});

for (const [name, make] of [
	["memory", memoryTrustStore],
	["indexeddb", indexedDbTrustStore],
] as const) {
	describe(`${name} trust store`, () => {
		test("keeps pins per account and replaces by key id", async () => {
			const store = make();
			await store.put("sub-a", host("ik_1"));
			await store.put("sub-a", { ...host("ik_1"), label: "Renamed" });
			await store.put("sub-a", host("ik_2"));
			await store.put("sub-b", host("ik_3"));
			const listed = await store.list("sub-a");
			expect(listed.map((entry) => entry.key_id).sort()).toEqual([
				"ik_1",
				"ik_2",
			]);
			expect(listed.find((entry) => entry.key_id === "ik_1")?.label).toBe(
				"Renamed",
			);
			expect((await store.list("sub-b")).length).toBe(1);
		});

		test("removes one pin", async () => {
			const store = make();
			await store.put("sub-r", host("ik_1"));
			await store.put("sub-r", host("ik_2"));
			await store.remove("sub-r", "ik_1");
			expect((await store.list("sub-r")).map((entry) => entry.key_id)).toEqual([
				"ik_2",
			]);
		});
	});
}

describe("indexeddb trust store without a database", () => {
	test("falls back to memory for the page", async () => {
		const saved = globalThis.indexedDB;
		const store = indexedDbTrustStore();
		Object.defineProperty(globalThis, "indexedDB", {
			value: undefined,
			configurable: true,
		});
		try {
			await store.put("sub-x", host("ik_1"));
			expect((await store.list("sub-x")).length).toBe(1);
		} finally {
			Object.defineProperty(globalThis, "indexedDB", {
				value: saved,
				configurable: true,
			});
		}
	});
});
