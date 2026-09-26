import { describe, expect, test } from "bun:test";
import { createComponentStore } from "./component-store";
import type { SurfaceComponent } from "./types";

const text = (id: string, value: string): SurfaceComponent => ({
	id,
	component: { id, type: "text", content: { literalString: value } },
});

function surface(...components: SurfaceComponent[]) {
	return Object.fromEntries(components.map((c) => [c.id, c]));
}

describe("component store", () => {
	test("commit notifies only the ids whose entries changed identity", () => {
		const a = text("a", "1");
		const b = text("b", "1");
		const store = createComponentStore(surface(a, b));
		const fired: string[] = [];
		store.subscribe("a", () => fired.push("a"));
		store.subscribe("b", () => fired.push("b"));

		const next = surface(text("a", "2"), b);
		store.replace(next);
		expect(store.get("a")?.component).toMatchObject({
			content: { literalString: "2" },
		});
		expect(fired).toEqual([]);

		store.commit(next);
		expect(fired).toEqual(["a"]);

		store.commit(next);
		expect(fired).toEqual(["a"]);
	});

	test("diffs committed-to-committed, ignoring records a render threw away", () => {
		const a = text("a", "1");
		const b = text("b", "1");
		const store = createComponentStore(surface(a, b));
		const fired: string[] = [];
		store.subscribe("a", () => fired.push("a"));
		store.subscribe("b", () => fired.push("b"));

		// The discarded render still swapped the record for same-pass reads.
		store.replace(surface(text("a", "2"), b));
		const landed = surface(text("a", "3"), text("b", "2"));
		store.replace(landed);
		store.commit(landed);
		expect(fired.sort()).toEqual(["a", "b"]);
		expect(store.get("a")).toBe(landed.a);
	});

	test("an entry changed and reverted before commit is not notified", () => {
		const a = text("a", "1");
		const initial = surface(a);
		const store = createComponentStore(initial);
		const fired: string[] = [];
		store.subscribe("a", () => fired.push("a"));

		store.replace(surface(text("a", "2")));
		store.replace(initial);
		store.commit(initial);
		expect(fired).toEqual([]);
	});

	test("appearing and disappearing entries notify their subscribers", () => {
		const a = text("a", "1");
		const store = createComponentStore(surface(a));
		const fired: string[] = [];
		store.subscribe("a", () => fired.push("a"));
		store.subscribe("c", () => fired.push("c"));

		const next = surface(text("c", "1"));
		store.replace(next);
		store.commit(next);
		expect(fired.sort()).toEqual(["a", "c"]);
		expect(store.get("a")).toBeUndefined();
	});

	test("unsubscribing stops notifications and frees the id", () => {
		const a = text("a", "1");
		const store = createComponentStore(surface(a));
		const fired: string[] = [];
		const unsubscribe = store.subscribe("a", () => fired.push("a"));
		unsubscribe();

		const next = surface(text("a", "2"));
		store.replace(next);
		store.commit(next);
		expect(fired).toEqual([]);
	});
});
