import type { AuthContextProps } from "react-oidc-context";
import { afterEach, describe, expect, test, vi } from "vitest";
import { accountQueryDeps } from "./account-query";

function auth(subject = "user-a", issuer = "https://login.test") {
	return {
		isAuthenticated: true,
		user: { profile: { sub: subject, iss: issuer } },
	} as AuthContextProps;
}

afterEach(() => vi.unstubAllEnvs());

describe("account query identity", () => {
	test("shares an account's observers across token renewal", () => {
		const backend = {};
		expect(accountQueryDeps(backend, auth())).toEqual(
			accountQueryDeps(backend, auth()),
		);
	});

	test("separates backend, hub, issuer, account and signed-out identities", () => {
		const backend = {};
		const original = accountQueryDeps(backend, auth());
		expect(accountQueryDeps({}, auth())).not.toEqual(original);
		expect(accountQueryDeps(backend, auth("user-b"))).not.toEqual(original);
		expect(
			accountQueryDeps(backend, auth("user-a", "https://other.test")),
		).not.toEqual(original);
		expect(
			accountQueryDeps(backend, { ...auth(), isAuthenticated: false }),
		).not.toEqual(original);
		vi.stubEnv("NEXT_PUBLIC_API_URL", "https://other-api.test");
		expect(accountQueryDeps(backend, auth())).not.toEqual(original);
	});
});
