import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { IBit } from "../schema/bit/bit";
import {
	type BitContentDigest,
	bitContentDigest,
	bitSources,
} from "./bit-sources";

interface FixtureCase {
	name: string;
	bit: IBit;
	root?: IBit;
	sources: string[];
	digest: BitContentDigest | null;
}

const fixture: { cases: FixtureCase[] } = JSON.parse(
	readFileSync(
		new URL("../../../core/runtime/fixtures/bit-sources.json", import.meta.url),
		"utf8",
	),
);

describe("bit sources mirror core", () => {
	for (const entry of fixture.cases)
		test(entry.name, () => {
			const root = entry.root ?? entry.bit;
			expect(bitSources(entry.bit, root)).toEqual(entry.sources);
			expect(bitContentDigest(entry.bit, root) ?? null).toEqual(entry.digest);
		});

	test("the root defaults to the Bit itself", () => {
		const [hub] = fixture.cases;
		if (!hub) throw new Error("fixture is empty");
		expect(bitSources(hub.bit)).toEqual(hub.sources);
		expect(bitContentDigest(hub.bit) ?? null).toEqual(hub.digest);
	});

	test("an empty query or fragment marker is still refused", () => {
		const bit = {
			id: "marker",
			hash: "marker",
			file_name: "weights.bin",
		} as IBit;
		for (const download_link of [
			"https://cdn.example.com/weights.bin?",
			"https://cdn.example.com/weights.bin#",
		])
			expect(bitSources({ ...bit, download_link })).toEqual([]);
	});
});
