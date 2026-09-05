import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../../lib/schema/bit/bit";
import {
	appendProfileBitReference,
	findProfileBit,
	profileBitDetails,
	profileBitReference,
} from "./profile-bits-helpers";

const bit = (hub: string, id: string): IBit => ({
	hub,
	id,
	type: IBitTypes.Llm,
	meta: {},
	parameters: {},
	authors: [],
	created: "",
	updated: "",
	dependencies: [],
	dependency_tree_hash: "",
	hash: "",
});

describe("profile bit references", () => {
	test("preserves existing unknown references and distinguishes the same ID on different hubs", () => {
		const existing = Object.freeze([
			"unknown-hub:legacy-id",
			"first.example:model",
		]);
		expect(
			appendProfileBitReference(existing, " second.example:model "),
		).toEqual([...existing, "second.example:model"]);
		expect(
			appendProfileBitReference(existing, " first.example:model "),
		).toEqual(existing);
		expect(appendProfileBitReference(existing, "  ")).toEqual(existing);
	});
	test("matches qualified references exactly, including hubs with a scheme and port", () => {
		const first = bit("https://first.example:8443", "same-id");
		const second = bit("second.example", "same-id");
		expect(profileBitReference(first)).toBe(
			"https://first.example:8443:same-id",
		);
		expect(findProfileBit("second.example:same-id", [first, second])).toBe(
			second,
		);
		expect(
			findProfileBit("unknown.example:same-id", [first, second]),
		).toBeUndefined();
		expect(findProfileBit("same-id", [first])).toBe(first);
	});
	test("uses available metadata and provider without requiring English metadata", () => {
		const item = bit("hub.example", "model");
		item.meta = {
			de: {
				name: "Sprachmodell",
				description: "Beschreibung",
			} as IBit["meta"][string],
		};
		item.parameters = { provider: { provider_name: "Example provider" } };
		expect(profileBitDetails(item)).toEqual({
			name: "Sprachmodell",
			description: "Beschreibung",
			provider: "Example provider",
		});
	});
});
