import { describe, expect, test } from "bun:test";

import { normalizeDatabaseTableIdentifier } from "../lib/database-table-name";

describe("normalizeDatabaseTableIdentifier", () => {
	test("keeps valid physical identifiers unchanged", () => {
		expect(normalizeDatabaseTableIdentifier("Existing.Table-v2")).toBe(
			"Existing.Table-v2",
		);
	});

	test("maps human-facing labels to stable semantic identifiers", () => {
		expect(normalizeDatabaseTableIdentifier("Library Files")).toBe(
			"library_files",
		);
		expect(normalizeDatabaseTableIdentifier("R&D / Reports")).toBe(
			"r_and_d_reports",
		);
	});
});
