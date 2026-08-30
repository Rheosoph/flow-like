/**
 * Tests for the local-sub helpers: executions without an authenticated caller
 * report the "local" sub, which the frontend maps to the current user.
 */
import { describe, expect, test } from "bun:test";
import {
	LOCAL_USER_SUB,
	USER_LOOKUP_BATCH_LIMIT,
	accountIdFromValue,
	chunkLookupIds,
	isLocalUserSub,
	partitionLookupIds,
	resolveAccountId,
	userLookupFromClaims,
} from "./user-state";

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";
const OTHER_SUB = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";

describe("isLocalUserSub", () => {
	test("matches the local sub regardless of casing or padding", () => {
		expect(isLocalUserSub(LOCAL_USER_SUB)).toBe(true);
		expect(isLocalUserSub(" Local ")).toBe(true);
	});

	test("rejects real subs and empty values", () => {
		expect(isLocalUserSub("auth0|abc123")).toBe(false);
		expect(isLocalUserSub("")).toBe(false);
		expect(isLocalUserSub(undefined)).toBe(false);
		expect(isLocalUserSub(null)).toBe(false);
	});
});

describe("resolveAccountId", () => {
	test("prefers the resolved account over the local placeholder", () => {
		expect(resolveAccountId("user-1", LOCAL_USER_SUB)).toBe("user-1");
		expect(resolveAccountId(undefined, "user-1")).toBe("user-1");
	});

	test("drops the placeholder entirely when nothing resolved", () => {
		expect(resolveAccountId(LOCAL_USER_SUB, LOCAL_USER_SUB)).toBeUndefined();
		expect(resolveAccountId(undefined, null)).toBeUndefined();
	});
});

describe("userLookupFromClaims", () => {
	test("maps auth claims onto a lookup record", () => {
		expect(
			userLookupFromClaims({
				sub: "user-1",
				name: "Ada Lovelace",
				preferred_username: "ada",
				nickname: "ada_l",
				email: "ada@example.com",
				picture: "https://example.com/ada.png",
			}),
		).toEqual({
			id: "user-1",
			name: "Ada Lovelace",
			preferred_username: "ada",
			username: "ada_l",
			email: "ada@example.com",
			avatar_url: "https://example.com/ada.png",
			created_at: "",
		});
	});

	test("falls back to a viewer label when no account is signed in", () => {
		const lookup = userLookupFromClaims(undefined);
		expect(lookup.id).toBe(LOCAL_USER_SUB);
		expect(lookup.name).toBe("You");
		expect(resolveAccountId(lookup.id)).toBeUndefined();
	});

	test("keeps a signed-in account nameless rather than labeling it You", () => {
		expect(userLookupFromClaims({ sub: "user-1" }).name).toBeUndefined();
	});
});

describe("accountIdFromValue", () => {
	test("reads an account id out of stored text", () => {
		expect(accountIdFromValue(SUB)).toBe(SUB);
		expect(accountIdFromValue(` ${SUB} `)).toBe(SUB);
		expect(accountIdFromValue("google_110293847561029384756")).toBe(
			"google_110293847561029384756",
		);
	});

	test("keeps the local placeholder, which no shape rule would recognise", () => {
		expect(accountIdFromValue(LOCAL_USER_SUB)).toBe(LOCAL_USER_SUB);
		expect(accountIdFromValue(` ${LOCAL_USER_SUB} `)).toBe(LOCAL_USER_SUB);
	});

	test("does not read the word Local as the signed-in user", () => {
		// An ACL export whose `owner` column reads Local/LOCAL is text, not a person.
		expect(accountIdFromValue("Local")).toBeNull();
		expect(accountIdFromValue("LOCAL")).toBeNull();
	});

	test("leaves values that name no account", () => {
		expect(accountIdFromValue("system")).toBeNull();
		expect(accountIdFromValue("")).toBeNull();
		expect(accountIdFromValue("   ")).toBeNull();
	});

	test("only reads text", () => {
		expect(accountIdFromValue(42)).toBeNull();
		expect(accountIdFromValue(null)).toBeNull();
		expect(accountIdFromValue(undefined)).toBeNull();
		expect(accountIdFromValue({ id: SUB })).toBeNull();
		expect(accountIdFromValue([SUB])).toBeNull();
	});
});

describe("partitionLookupIds", () => {
	test("dedupes and drops what the directory cannot be asked about", () => {
		expect(
			partitionLookupIds([SUB, SUB, ` ${SUB} `, OTHER_SUB, "", "   "]),
		).toEqual({ subs: [SUB, OTHER_SUB], local: false });
	});

	test("lifts the local placeholder out of the batch", () => {
		expect(partitionLookupIds([SUB, LOCAL_USER_SUB, "LOCAL"])).toEqual({
			subs: [SUB],
			local: true,
		});
	});

	test("has nothing to send for an empty request", () => {
		expect(partitionLookupIds([])).toEqual({ subs: [], local: false });
		expect(partitionLookupIds([LOCAL_USER_SUB])).toEqual({
			subs: [],
			local: true,
		});
	});
});

describe("chunkLookupIds", () => {
	test("slices to what the hub answers rather than truncates", () => {
		const subs = Array.from({ length: 250 }, (_, index) => `sub-${index}`);
		expect(chunkLookupIds(subs).map((chunk) => chunk.length)).toEqual([
			100, 100, 50,
		]);
		expect(chunkLookupIds(subs).flat()).toEqual(subs);
	});

	test("leaves a request that already fits alone", () => {
		expect(chunkLookupIds([SUB, OTHER_SUB])).toEqual([[SUB, OTHER_SUB]]);
		expect(chunkLookupIds([])).toEqual([]);
	});

	test("honours a smaller size", () => {
		expect(chunkLookupIds(["a", "b", "c"], 2)).toEqual([["a", "b"], ["c"]]);
	});

	test("matches the cap the hub enforces", () => {
		expect(USER_LOOKUP_BATCH_LIMIT).toBe(100);
	});
});
