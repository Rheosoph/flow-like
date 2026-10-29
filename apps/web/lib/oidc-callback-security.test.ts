// @vitest-environment happy-dom
import { createHash } from "node:crypto";
import {
	type INavigator,
	type NavigateParams,
	UserManager,
	type UserManagerSettings,
} from "oidc-client-ts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getWebOidcSettings } from "./oidc-settings";

const config: UserManagerSettings = {
	authority: "https://issuer.example.test",
	client_id: "web-client",
	redirect_uri: "https://app.example.test/callback",
	response_type: "code",
	scope: "openid profile email",
	metadata: {
		issuer: "https://issuer.example.test",
		authorization_endpoint: "https://issuer.example.test/authorize",
		token_endpoint: "https://issuer.example.test/token",
	},
};

const managers: UserManager[] = [];
const fetchMock = vi.fn<typeof fetch>();

function createTab() {
	const navigate = vi.fn(async ({ url }: NavigateParams) => ({ url }));
	const navigator: INavigator = {
		prepare: async () => ({ navigate, close: () => {} }),
		callback: async () => {},
	};
	const manager = new UserManager(getWebOidcSettings(config), navigator);
	managers.push(manager);
	return { manager, navigate };
}

function callback(state?: string, code = "rejected-code") {
	const url = new URL(config.redirect_uri);
	url.searchParams.set("code", code);
	if (state !== undefined) url.searchParams.set("state", state);
	return url.href;
}

async function startSignIn(tab: ReturnType<typeof createTab>) {
	await tab.manager.signinRedirect();
	const url = tab.navigate.mock.lastCall?.[0].url;
	if (!url) throw new Error("Sign-in did not navigate");
	const request = new URL(url);
	const stateId = request.searchParams.get("state");
	if (!stateId) throw new Error("Sign-in request has no state");
	const serialized = localStorage.getItem(`oidc.${stateId}`);
	if (!serialized) throw new Error("Sign-in state is not in localStorage");
	const state = JSON.parse(serialized) as {
		id: string;
		code_verifier: string;
	};
	return { request, state, stateId };
}

beforeEach(() => {
	localStorage.clear();
	sessionStorage.clear();
	vi.stubEnv("NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG", "0");
	vi.stubEnv("NEXT_PUBLIC_REDIRECT_URL", undefined);
	vi.stubEnv("NEXT_PUBLIC_REDIRECT_LOGOUT_URL", undefined);
	fetchMock.mockReset();
	// Stop at the token endpoint so these tests never manufacture a signed-in user.
	fetchMock.mockResolvedValue(
		new Response(JSON.stringify({ error: "invalid_grant" }), {
			status: 400,
			headers: { "Content-Type": "application/json" },
		}),
	);
	vi.stubGlobal("fetch", fetchMock);
});

afterEach(async () => {
	for (const manager of managers.splice(0)) {
		manager.stopSilentRenew();
		await manager.events.unload();
	}
	vi.unstubAllGlobals();
	vi.unstubAllEnvs();
	localStorage.clear();
	sessionStorage.clear();
});

describe("web OIDC callback state and PKCE", () => {
	it("shares pending state between independent managers and sends its PKCE verifier", async () => {
		const initiatingTab = createTab();
		const { request, state, stateId } = await startSignIn(initiatingTab);
		expect(initiatingTab.manager.settings.disablePKCE).toBe(false);
		expect(request.searchParams.get("response_type")).toBe("code");
		expect(request.searchParams.get("code_challenge_method")).toBe("S256");
		expect(state.code_verifier.length).toBeGreaterThanOrEqual(43);
		expect(request.searchParams.get("code_challenge")).toBe(
			createHash("sha256").update(state.code_verifier).digest("base64url"),
		);

		// A new tab has no initiating tab's sessionStorage, but shares localStorage.
		sessionStorage.clear();
		const receivingTab = createTab();
		await expect(
			receivingTab.manager.signinRedirectCallback(callback(stateId)),
		).rejects.toThrow("invalid_grant");
		expect(fetchMock).toHaveBeenCalledOnce();
		const [url, init] = fetchMock.mock.calls[0];
		expect(url).toBe(config.metadata?.token_endpoint);
		const body = new URLSearchParams(init?.body as string);
		expect(body.get("grant_type")).toBe("authorization_code");
		expect(body.get("code")).toBe("rejected-code");
		expect(body.get("code_verifier")).toBe(state.code_verifier);
		expect(localStorage.getItem(`oidc.${stateId}`)).toBeNull();

		await expect(
			receivingTab.manager.signinRedirectCallback(callback(stateId)),
		).rejects.toThrow("No matching state found in storage");
		expect(fetchMock).toHaveBeenCalledOnce();
	});

	it.each([
		[undefined, "No state in response"],
		["unrecognized-state", "No matching state found in storage"],
	])(
		"rejects missing or unknown state %s before requesting tokens",
		async (state, error) => {
			const tab = createTab();
			await expect(
				tab.manager.signinRedirectCallback(callback(state)),
			).rejects.toThrow(error);
			expect(fetchMock).not.toHaveBeenCalled();
		},
	);

	it("rejects a tampered state ID before requesting tokens", async () => {
		const tab = createTab();
		const { stateId, state } = await startSignIn(tab);
		localStorage.setItem(
			`oidc.${stateId}`,
			JSON.stringify({ ...state, id: "different-state" }),
		);
		await expect(
			tab.manager.signinRedirectCallback(callback(stateId)),
		).rejects.toThrow("State does not match");
		expect(fetchMock).not.toHaveBeenCalled();
	});

	it("restarts with fresh state and PKCE credentials after rejecting a callback", async () => {
		const tab = createTab();
		const original = await startSignIn(tab);
		localStorage.removeItem(`oidc.${original.stateId}`);
		await expect(
			tab.manager.signinRedirectCallback(
				callback(original.stateId, "old-code"),
			),
		).rejects.toThrow("No matching state found in storage");

		const restarted = await startSignIn(tab);
		expect(restarted.stateId).not.toBe(original.stateId);
		expect(restarted.state.code_verifier).not.toBe(
			original.state.code_verifier,
		);
		expect(restarted.request.searchParams.get("code_challenge")).not.toBe(
			original.request.searchParams.get("code_challenge"),
		);
		expect(restarted.request.searchParams.has("code")).toBe(false);
		expect(restarted.request.href).not.toContain("old-code");
		expect(fetchMock).not.toHaveBeenCalled();
	});
});
