import { describe, expect, test } from "bun:test";
import { QueryClient } from "@tanstack/react-query";
import {
	accountDraft,
	accountError,
	accountHasChanges,
	invalidateAccountIdentity,
	mergeAccountDraft,
} from "./account-model";
import { createAccountTokenProvider } from "./account-session";

describe("account drafts and identity refresh", () => {
	test("missing optional values do not create unsaved changes", () => {
		const saved = accountDraft({ name: "Sam" });
		expect(
			accountHasChanges(
				saved,
				accountDraft({ name: "Sam", description: undefined }),
			),
		).toBe(false);
	});
	test("a photo or email refresh keeps an unsaved name and bio", () => {
		const previous = accountDraft({
			name: "Sam",
			preferred_username: "sam",
			description: "Old bio",
		});
		const draft = { ...previous, name: "Samantha", description: "New bio" };
		const next = accountDraft({
			name: "Sam",
			preferred_username: "sam-updated",
			description: "Old bio",
		});
		expect(mergeAccountDraft(draft, previous, next)).toEqual({
			name: "Samantha",
			username: "sam-updated",
			description: "New bio",
		});
	});
	test("fresh remote values replace fields that have not been edited", () => {
		const previous = accountDraft({ name: "Sam" });
		const next = accountDraft({ name: "Sam Lee", description: "A new bio" });
		expect(mergeAccountDraft(previous, previous, next)).toEqual(next);
	});
	test("saving invalidates public profile and identity badge caches", async () => {
		const client = new QueryClient({
			defaultOptions: { queries: { staleTime: Number.POSITIVE_INFINITY } },
		});
		for (const key of [
			["getInfo"],
			["lookupUser", "me"],
			["lookupUserBatched", "me"],
			["lookupUsers", ["me"]],
			["getProfile"],
		])
			client.setQueryData(key, { name: "old" });
		await invalidateAccountIdentity(client);
		for (const key of [
			["getInfo"],
			["lookupUser", "me"],
			["lookupUserBatched", "me"],
			["lookupUsers", ["me"]],
		])
			expect(client.getQueryState(key)?.isInvalidated).toBe(true);
		expect(client.getQueryState(["getProfile"])?.isInvalidated).toBe(false);
		client.clear();
	});
	test("policy and expired code errors explain the right recovery", () => {
		const error = new Error();
		error.name = "InvalidPasswordException";
		expect(accountError(error, "failed")).toContain("password requirements");
		error.name = "ExpiredCodeException";
		expect(accountError(error, "failed")).toContain("Request a new code");
	});
});

function token(exp: number, id: string) {
	return JSON.stringify({ exp, id });
}
function decode(value: string) {
	return { payload: JSON.parse(value), toString: () => value };
}
const valid = () => ({
	access_token: token(Date.now() / 1000 + 600, "new"),
	id_token: token(Date.now() / 1000 + 600, "id"),
});

describe("account authentication follows the live OIDC session", () => {
	test("uses replacement context tokens after automatic renewal", async () => {
		let auth = {
			isAuthenticated: true,
			user: valid(),
			signinSilent: async () => valid(),
		};
		const provider = createAccountTokenProvider(() => auth, decode);
		const replacement = {
			...valid(),
			access_token: token(Date.now() / 1000 + 600, "replacement"),
		};
		auth = { ...auth, user: replacement };
		expect((await provider.getTokens())?.accessToken.toString()).toBe(
			replacement.access_token,
		);
	});
	test("renews expired credentials before returning tokens", async () => {
		const renewed = valid();
		let calls = 0;
		const provider = createAccountTokenProvider(
			() => ({
				isAuthenticated: true,
				user: { ...valid(), access_token: token(1, "expired") },
				signinSilent: async () => {
					calls++;
					return renewed;
				},
			}),
			decode,
		);
		expect((await provider.getTokens())?.accessToken.toString()).toBe(
			renewed.access_token,
		);
		expect(calls).toBe(1);
	});
	test("honors forced refresh and shares simultaneous renewals", async () => {
		let calls = 0;
		let resolve!: (value: ReturnType<typeof valid>) => void;
		const renewal = new Promise<ReturnType<typeof valid>>((done) => {
			resolve = done;
		});
		const provider = createAccountTokenProvider(
			() => ({
				isAuthenticated: true,
				user: valid(),
				signinSilent: () => {
					calls++;
					return renewal;
				},
			}),
			decode,
		);
		const first = provider.getTokens({ forceRefresh: true });
		const second = provider.getTokens({ forceRefresh: true });
		resolve(valid());
		await Promise.all([first, second]);
		expect(calls).toBe(1);
	});
	test("does not fall back to expired credentials after failed renewal", async () => {
		const provider = createAccountTokenProvider(
			() => ({
				isAuthenticated: true,
				user: { ...valid(), access_token: token(1, "expired") },
				signinSilent: async () => null,
			}),
			decode,
		);
		await expect(provider.getTokens()).rejects.toMatchObject({
			name: "AccountSessionExpired",
		});
	});
	test("returns no credentials once the current account has signed out", async () => {
		const provider = createAccountTokenProvider(
			() => ({
				isAuthenticated: false,
				user: valid(),
				signinSilent: async () => valid(),
			}),
			decode,
		);
		expect(await provider.getTokens()).toBeNull();
	});
});
