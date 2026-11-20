import { describe, expect, test } from "bun:test";
import { isValidAlias, suggestAlias } from "./external-database-dialog";

describe("external database schema names", () => {
	test("follow the server's alias rule", () => {
		for (const alias of ["crm", "_staging", "warehouse_2", "a".repeat(63)]) {
			expect(isValidAlias(alias)).toBe(true);
		}
		for (const alias of [
			"",
			"CRM",
			"1crm",
			"crm-eu",
			"crm.eu",
			"public",
			"information_schema",
			"a".repeat(64),
		]) {
			expect(isValidAlias(alias)).toBe(false);
		}
	});

	test("are suggested from the display name", () => {
		expect(suggestAlias("Sales EU")).toBe("sales_eu");
		expect(suggestAlias("  Müller & Söhne GmbH ")).toBe("muller_sohne_gmbh");
		expect(suggestAlias("2024 Orders")).toBe("_2024_orders");
		expect(suggestAlias("Public")).toBe("public_db");
		expect(suggestAlias("!!!")).toBe("");
		expect(suggestAlias("x".repeat(80))).toHaveLength(63);
		for (const name of ["Sales EU", "2024 Orders", "Public", "x".repeat(80)]) {
			expect(isValidAlias(suggestAlias(name))).toBe(true);
		}
	});
});
