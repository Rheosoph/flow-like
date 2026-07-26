import { describe, expect, test } from "bun:test";
import type { IApp } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";
import type { LibraryItem } from "./library-types";
import {
	sortAppPairsByRecency,
	sortItems,
	sortItemsByRank,
} from "./library-types";

const time = (secs?: number, nanos = 0) =>
	secs === undefined
		? undefined
		: { secs_since_epoch: secs, nanos_since_epoch: nanos };

/** `secs` is the metadata timestamp; `appSecs` the app's, when they differ. */
const item = (
	id: string,
	name: string,
	secs?: number,
	nanos = 0,
	appSecs?: number,
): LibraryItem =>
	({
		id,
		name,
		updated_at: time(secs, nanos),
		app: { id, updated_at: time(appSecs) },
	}) as unknown as LibraryItem;

const ids = (items: LibraryItem[]) => items.map((i) => i.id);

describe("sortItems", () => {
	test("orders by recency, newest first", () => {
		const items = [
			item("a", "A", 100),
			item("b", "B", 300),
			item("c", "C", 200),
		];
		expect(ids(sortItems(items, "recent"))).toEqual(["b", "c", "a"]);
	});

	test("orders alphabetically by name", () => {
		const items = [item("a", "Zeta"), item("b", "Alpha"), item("c", "Mid")];
		expect(ids(sortItems(items, "alpha"))).toEqual(["b", "c", "a"]);
	});

	test("is independent of input order when sort keys tie", () => {
		const forward = [item("a", "Same", 100), item("b", "Same", 100)];
		const reversed = [item("b", "Same", 100), item("a", "Same", 100)];

		expect(ids(sortItems(forward, "recent"))).toEqual(
			ids(sortItems(reversed, "recent")),
		);
		expect(ids(sortItems(forward, "alpha"))).toEqual(
			ids(sortItems(reversed, "alpha")),
		);
	});

	test("breaks equal seconds on nanoseconds", () => {
		const items = [item("a", "A", 100, 1), item("b", "B", 100, 999)];
		expect(ids(sortItems(items, "recent"))).toEqual(["b", "a"]);
	});

	test("treats missing timestamps as oldest", () => {
		const items = [item("a", "A"), item("b", "B", 50)];
		expect(ids(sortItems(items, "recent"))).toEqual(["b", "a"]);
	});

	test("recency follows app activity, not just metadata edits", () => {
		// "a" was renamed long ago but its boards changed recently; "b" is the
		// reverse. Each must be ranked by whichever of its timestamps is newer.
		const items = [
			item("a", "A", 100, 0, 900),
			item("b", "B", 500, 0, 200),
			item("c", "C", 50, 0, 50),
		];
		expect(ids(sortItems(items, "recent"))).toEqual(["a", "b", "c"]);
	});

	test("a metadata edit still counts when the app record is older", () => {
		const items = [item("a", "A", 900, 0, 100), item("b", "B", 200, 0, 300)];
		expect(ids(sortItems(items, "recent"))).toEqual(["a", "b"]);
	});
});

describe("sortAppPairsByRecency", () => {
	const pair = (id: string, name: string, metaSecs: number, appSecs: number) =>
		[
			{ id, updated_at: time(appSecs) } as unknown as IApp,
			{ name, updated_at: time(metaSecs) } as unknown as IMetadata,
		] as [IApp, IMetadata | undefined];

	test("matches the library's ordering for the same data", () => {
		const pairs = [
			pair("a", "A", 100, 900),
			pair("b", "B", 500, 200),
			pair("c", "C", 50, 50),
		];
		expect(sortAppPairsByRecency(pairs).map(([app]) => app.id)).toEqual([
			"a",
			"b",
			"c",
		]);
	});

	test("tolerates pairs without metadata", () => {
		const pairs: [IApp, IMetadata | undefined][] = [
			[{ id: "a", updated_at: time(100) } as unknown as IApp, undefined],
			[{ id: "b", updated_at: time(800) } as unknown as IApp, undefined],
		];
		expect(sortAppPairsByRecency(pairs).map(([app]) => app.id)).toEqual([
			"b",
			"a",
		]);
	});
});

describe("sortItemsByRank", () => {
	const ranks: Record<string, number> = { b: 0, a: 1 };

	test("explicit rank wins over the sort mode", () => {
		const items = [item("a", "A", 900), item("b", "B", 100)];
		expect(ids(sortItemsByRank(items, (id) => ranks[id], "recent"))).toEqual([
			"b",
			"a",
		]);
	});

	test("unranked items follow ranked ones, ordered by the sort mode", () => {
		const items = [
			item("z", "Z", 500),
			item("a", "A", 900),
			item("y", "Y", 700),
			item("b", "B", 100),
		];
		expect(ids(sortItemsByRank(items, (id) => ranks[id], "recent"))).toEqual([
			"b",
			"a",
			"y",
			"z",
		]);
	});

	test("a rank of zero is honoured rather than treated as absent", () => {
		const items = [item("a", "A", 900), item("b", "B", 100)];
		expect(
			ids(sortItemsByRank(items, (id) => (id === "b" ? 0 : null), "recent")),
		).toEqual(["b", "a"]);
	});

	test("is independent of input order", () => {
		const forward = [
			item("a", "A", 900),
			item("b", "B", 100),
			item("c", "C", 100),
		];
		const reversed = [...forward].reverse();
		expect(ids(sortItemsByRank(forward, (id) => ranks[id], "recent"))).toEqual(
			ids(sortItemsByRank(reversed, (id) => ranks[id], "recent")),
		);
	});
});
