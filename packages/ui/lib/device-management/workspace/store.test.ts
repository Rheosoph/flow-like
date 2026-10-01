import { expect, test } from "bun:test";
import { createWorkspaceStore } from "./store";

test("bump advances the version and notifies subscribers until they leave", () => {
	const store = createWorkspaceStore();
	const seen: number[] = [];
	const stop = store.subscribe(() => seen.push(store.getVersion()));
	expect(store.getVersion()).toBe(0);
	store.bump();
	store.bump();
	stop();
	store.bump();
	expect(seen).toEqual([1, 2]);
	expect(store.getVersion()).toBe(3);
});

test("stores are independent per workspace", () => {
	const first = createWorkspaceStore();
	const second = createWorkspaceStore();
	first.bump();
	expect(first.getVersion()).toBe(1);
	expect(second.getVersion()).toBe(0);
});
