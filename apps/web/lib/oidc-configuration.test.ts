import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const harness = vi.hoisted(() => ({
	apiUrl: "https://api.test",
	configurationError: false,
}));
vi.mock("./public-config", () => ({
	getPublicApiUrl: () => {
		if (harness.configurationError) throw new Error("Missing runtime config");
		return harness.apiUrl;
	},
}));

const config = {
	client_id: "web-client",
	authority: "https://login.test",
	redirect_uri: "https://web.test/callback",
	cognito: { user_pool_id: "pool-1" },
};
const fetchMock = vi.fn<typeof fetch>();
let loader: typeof import("./oidc-configuration");

beforeEach(async () => {
	vi.resetModules();
	harness.apiUrl = "https://api.test";
	harness.configurationError = false;
	vi.stubEnv("NEXT_PUBLIC_API_URL", undefined);
	vi.stubEnv("NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG", "0");
	vi.stubGlobal("fetch", fetchMock);
	fetchMock.mockReset();
	loader = await import("./oidc-configuration");
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.unstubAllEnvs();
	vi.useRealTimers();
	vi.restoreAllMocks();
});

function holdResponse() {
	let resolve!: (response: Response) => void;
	fetchMock.mockReturnValueOnce(
		new Promise<Response>((done) => {
			resolve = done;
		}),
	);
	return () => resolve(Response.json(config));
}

describe("public OIDC startup configuration", () => {
	it("shares a prefetch with concurrent auth mounts without CORS-unsafe headers", async () => {
		const complete = holdResponse();
		loader.prefetchWebOidcConfiguration();
		const first = loader.loadWebOidcConfiguration();
		const second = loader.loadWebOidcConfiguration();
		expect(fetchMock).toHaveBeenCalledTimes(1);
		expect(fetchMock).toHaveBeenCalledWith(
			"https://api.test/api/v1/auth/openid",
			{ method: "GET" },
		);
		complete();
		expect(await first).toEqual(config);
		expect(await second).toEqual(config);
	});

	it("hands off an already completed prefetch and refreshes on a later auth mount", async () => {
		fetchMock.mockResolvedValueOnce(Response.json(config));
		loader.prefetchWebOidcConfiguration();
		// Let both fetch and JSON decoding finish before the provider chunk arrives.
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(await loader.loadWebOidcConfiguration()).toEqual(config);
		expect(fetchMock).toHaveBeenCalledTimes(1);
		fetchMock.mockResolvedValueOnce(Response.json(config));
		await loader.loadWebOidcConfiguration();
		expect(fetchMock).toHaveBeenCalledTimes(2);
	});

	it("expires a prefetch that was never consumed", async () => {
		vi.useFakeTimers();
		fetchMock.mockResolvedValueOnce(Response.json(config));
		loader.prefetchWebOidcConfiguration();
		await vi.advanceTimersByTimeAsync(30_001);
		fetchMock.mockResolvedValueOnce(Response.json(config));
		expect(await loader.loadWebOidcConfiguration()).toEqual(config);
		expect(fetchMock).toHaveBeenCalledTimes(2);
	});

	it("does not reuse config after the public API origin changes", async () => {
		const completeOld = holdResponse();
		loader.prefetchWebOidcConfiguration();
		harness.apiUrl = "https://another-api.test";
		const completeNew = holdResponse();
		const next = loader.loadWebOidcConfiguration();
		completeOld();
		await new Promise((resolve) => setTimeout(resolve, 0));
		const concurrent = loader.loadWebOidcConfiguration();
		expect(fetchMock).toHaveBeenCalledTimes(2);
		completeNew();
		expect(await next).toEqual(config);
		expect(await concurrent).toEqual(config);
		expect(fetchMock).toHaveBeenLastCalledWith(
			"https://another-api.test/api/v1/auth/openid",
			{ method: "GET" },
		);
	});

	it("honors a deployment's runtime API origin", async () => {
		vi.stubEnv("NEXT_PUBLIC_FLOW_LIKE_RUNTIME_CONFIG", "1");
		vi.stubGlobal("window", {
			location: { origin: "https://web.test" },
			__FLOW_LIKE_PUBLIC_CONFIG__: {
				version: 1,
				apiUrl: "https://runtime-api.test/",
			},
		});
		fetchMock.mockResolvedValueOnce(Response.json(config));
		await loader.loadWebOidcConfiguration();
		expect(fetchMock).toHaveBeenCalledWith(
			"https://runtime-api.test/api/v1/auth/openid",
			{ method: "GET" },
		);
	});

	it("honors the build-time API override", async () => {
		vi.stubEnv("NEXT_PUBLIC_API_URL", "https://build-api.test/");
		fetchMock.mockResolvedValueOnce(Response.json(config));
		await loader.loadWebOidcConfiguration();
		expect(fetchMock).toHaveBeenCalledWith(
			"https://build-api.test/api/v1/auth/openid",
			{ method: "GET" },
		);
	});

	it("allows the auth mount to retry a failed prefetch", async () => {
		fetchMock.mockRejectedValueOnce(new Error("Network unavailable"));
		loader.prefetchWebOidcConfiguration();
		await new Promise((resolve) => setTimeout(resolve, 0));
		fetchMock.mockResolvedValueOnce(Response.json(config));
		expect(await loader.loadWebOidcConfiguration()).toEqual(config);
		expect(fetchMock).toHaveBeenCalledTimes(2);
	});

	it("clears HTTP failures and malformed JSON instead of caching them", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		fetchMock.mockResolvedValueOnce(
			new Response("Unavailable", { status: 503 }),
		);
		expect(await loader.loadWebOidcConfiguration()).toBeUndefined();
		fetchMock.mockResolvedValueOnce(new Response("invalid JSON"));
		await expect(loader.loadWebOidcConfiguration()).rejects.toThrow();
		fetchMock.mockResolvedValueOnce(Response.json(config));
		expect(await loader.loadWebOidcConfiguration()).toEqual(config);
		expect(fetchMock).toHaveBeenCalledTimes(3);
	});

	it("leaves runtime configuration errors to the auth provider without fetching a fallback", async () => {
		harness.configurationError = true;
		expect(() => loader.prefetchWebOidcConfiguration()).not.toThrow();
		await expect(loader.loadWebOidcConfiguration()).rejects.toThrow(
			"Missing runtime config",
		);
		expect(fetchMock).not.toHaveBeenCalled();
	});
});
