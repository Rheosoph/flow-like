import { expect, test } from "bun:test";
import { normalizePersonalAccessTokens } from "./personal-access-token";

test("accepts current and older hub permission fields without changing access levels", () => {
	const base = {
		id: "token",
		name: "Integration",
		created_at: 1800000000000,
		valid_until: null,
	};
	for (const permission of [1, 2, 4, 8]) {
		const plural = normalizePersonalAccessTokens([
			{ ...base, permissions: permission },
		]);
		const singular = normalizePersonalAccessTokens([{ ...base, permission }]);
		expect(plural).toEqual(singular);
		expect(plural[0].permission).toBe(permission);
		expect(plural[0].created_at).toBe("2027-01-15T08:00:00.000Z");
	}
});

test("does not turn missing or malformed permission values into Admin", () => {
	expect(normalizePersonalAccessTokens(null)).toEqual([]);
	expect(normalizePersonalAccessTokens([null, {}, "token"])).toEqual([]);
	expect(
		normalizePersonalAccessTokens([{ id: "t", name: "n", permission: "4" }])[0]
			.permission,
	).toBe(0);
	expect(
		normalizePersonalAccessTokens([
			{ id: "t", name: "n", permission: 1, permissions: 4 },
		])[0].permission,
	).toBe(1);
});
