import { describe, expect, test } from "bun:test";
import {
	databaseSelectorFromParams,
	databaseSelectorKey,
	isDatabaseSnapshot,
	setDatabaseSelectorParams,
} from "./database-reference";

describe("database view identity", () => {
	test("explicit empty references do not fall back to latest main", () => {
		for (const query of [
			"branch=",
			"branch=%20",
			"tag=",
			"tag=%20",
			"version=",
		]) {
			expect(() =>
				databaseSelectorFromParams(new URLSearchParams(query)),
			).toThrow();
		}
	});
	test("distinguishes branches, pinned versions, tags and readonly views", () => {
		const selectors = [
			{},
			{ branch: "experiment" },
			{ version: 3 },
			{ tag: "training" },
			{ read_only: true },
		];
		expect(new Set(selectors.map(databaseSelectorKey)).size).toBe(5);
		expect(databaseSelectorKey({})).toBe(
			databaseSelectorKey({ branch: "main" }),
		);
	});
	test("snapshots and explicit readonly handles cannot expose writes", () => {
		expect(isDatabaseSnapshot({ branch: "experiment" })).toBe(false);
		for (const selector of [
			{ version: 1 },
			{ tag: "training" },
			{ read_only: true },
		]) {
			expect(isDatabaseSnapshot(selector)).toBe(true);
		}
	});
	test("changing branches clears an earlier tag, version and page", () => {
		const params = new URLSearchParams(
			"table=events&scope=user&page=8&tag=training&version=7",
		);
		setDatabaseSelectorParams(params, { branch: "experiment" });
		expect(params.toString()).toBe("table=events&scope=user&branch=experiment");
	});
	test("tags resolve their own branch and version", () => {
		expect(
			databaseSelectorFromParams(
				new URLSearchParams("branch=wrong&version=99&tag=training"),
			),
		).toEqual({ tag: "training" });
	});
	test("does not round version identities that exceed JavaScript precision", () => {
		expect(() =>
			databaseSelectorFromParams(
				new URLSearchParams("version=9007199254740993"),
			),
		).toThrow("invalid");
		expect(
			databaseSelectorFromParams(
				new URLSearchParams("version=42&branch=experiment"),
			),
		).toEqual({ branch: "experiment", version: 42 });
	});
});
