/**
 * Tests for the local-sub helpers: executions without an authenticated caller
 * report the "local" sub, which the frontend maps to the current user.
 */
import { describe, expect, test } from "bun:test";
import {
	LOCAL_USER_SUB,
	isLocalUserSub,
	resolveAccountId,
	userLookupFromClaims,
} from "./user-state";

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
