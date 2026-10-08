import type { IProfile } from "@flow-like/flow-like-ui/types";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { del, fetcher, get, post, put } from "./api";
import { reportUnauthorized } from "./auth-session";
import { WebApiState } from "./web-states/api-state";
import { type WebBackendRef, apiFetch } from "./web-states/api-utils";

vi.mock("./auth-session", () => ({ reportUnauthorized: vi.fn() }));
vi.mock("@flow-like/flow-like-ui", () => ({
	getActiveTraceContext: () => undefined,
	getTelemetryTraceparent: () => undefined,
}));

const profile = { hub: "https://api.example.test" } as IProfile;
const makeAuth = (token: string | undefined = "request-token") =>
	({
		isAuthenticated: true,
		user: token ? { access_token: token } : undefined,
	}) as AuthContextProps;

interface Transport {
	name: string;
	request: (
		auth: AuthContextProps,
		path?: string,
		options?: RequestInit,
	) => Promise<unknown>;
	returnsUndefined?: boolean;
}

const transports: Transport[] = [
	{
		name: "apiFetch",
		request: (auth, path, options) =>
			apiFetch(path ?? "user/info", options, auth),
	},
	{
		name: "fetcher",
		request: (auth, path, options) =>
			fetcher(profile, path ?? "user/info", options, auth),
	},
	{
		name: "WebApiState.fetch",
		request: (auth, path, options) =>
			new WebApiState({ auth }).fetch(profile, path ?? "user/info", options),
	},
	{
		name: "WebApiState.stream",
		request: (auth, path, options) =>
			new WebApiState({ auth }).stream(profile, path ?? "user/info", options),
	},
	{
		name: "get",
		request: (auth, path = "user/info") => get(profile, path, auth),
		returnsUndefined: true,
	},
	{
		name: "post",
		request: (auth, path = "user/info") => post(profile, path, {}, auth),
		returnsUndefined: true,
	},
	{
		name: "put",
		request: (auth, path = "user/info") => put(profile, path, {}, auth),
		returnsUndefined: true,
	},
	{
		name: "del",
		request: (auth, path = "user/info") => del(profile, path, auth),
		returnsUndefined: true,
	},
];

beforeEach(() => {
	vi.mocked(reportUnauthorized).mockClear();
	vi.spyOn(console, "error").mockImplementation(() => {});
	vi.stubGlobal("navigator", { onLine: true });
});

afterEach(() => {
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe.each(transports)("$name authentication failures", (transport) => {
	test("reports the rejected token even when the session changes during the request", async () => {
		const auth = makeAuth();
		const fetchMock = vi.fn(
			async (_url: RequestInfo | URL, init?: RequestInit) => {
				expect(new Headers(init?.headers).get("Authorization")).toBe(
					"Bearer request-token",
				);
				if (auth.user) auth.user.access_token = "new-token";
				return new Response("Unauthorized", { status: 401 });
			},
		);
		vi.stubGlobal("fetch", fetchMock);

		if (transport.returnsUndefined) {
			await expect(transport.request(auth)).resolves.toBeUndefined();
		} else {
			await expect(transport.request(auth)).rejects.toMatchObject({
				status: 401,
			});
		}
		expect(reportUnauthorized).toHaveBeenCalledExactlyOnceWith(
			auth,
			"request-token",
		);
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});

	test.each([403, 500])(
		"does not invalidate the session after HTTP %i",
		async (status) => {
			vi.stubGlobal(
				"fetch",
				async () => new Response("Request failed", { status }),
			);
			await transport.request(makeAuth()).catch(() => undefined);
			expect(reportUnauthorized).not.toHaveBeenCalled();
		},
	);

	test("does not invalidate the session after a network error", async () => {
		vi.stubGlobal("fetch", async () => {
			throw new TypeError("Failed to fetch");
		});
		await expect(transport.request(makeAuth())).rejects.toThrow(
			"Failed to fetch",
		);
		expect(reportUnauthorized).not.toHaveBeenCalled();
	});

	test("reports a 401 sent without an account token", async () => {
		const auth = makeAuth();
		auth.user = undefined;
		vi.stubGlobal(
			"fetch",
			async () => new Response("Unauthorized", { status: 401 }),
		);
		await transport.request(auth).catch(() => undefined);
		expect(reportUnauthorized).toHaveBeenCalledExactlyOnceWith(auth, undefined);
	});
});

test.each(transports.filter(({ name }) => name !== "WebApiState.stream"))(
	"$name reports a missing token before sending a protected request",
	async ({ request }) => {
		const auth = makeAuth();
		auth.user = undefined;
		const fetchMock = vi.fn();
		vi.stubGlobal("fetch", fetchMock);
		await expect(request(auth, "apps/app-1/board/summary")).rejects.toThrow(
			"Authentication token required",
		);
		expect(reportUnauthorized).toHaveBeenCalledExactlyOnceWith(auth, undefined);
		expect(fetchMock).not.toHaveBeenCalled();
	},
);

test.each(transports.slice(0, 4).filter(({ name }) => name !== "fetcher"))(
	"$name does not attribute a caller's separate bearer token to the account",
	async ({ request }) => {
		const auth = makeAuth();
		vi.stubGlobal(
			"fetch",
			async (_url: RequestInfo | URL, init?: RequestInit) => {
				expect(new Headers(init?.headers).get("Authorization")).toBe(
					"Bearer separate-token",
				);
				return new Response("Unauthorized", { status: 401 });
			},
		);
		await expect(
			request(auth, "user/info", {
				headers: new Headers({ authorization: "Bearer separate-token" }),
			}),
		).rejects.toMatchObject({ status: 401 });
		expect(reportUnauthorized).not.toHaveBeenCalled();
	},
);

test("fetcher does not report a separate token when the account has no token", async () => {
	const auth = makeAuth();
	auth.user = undefined;
	vi.stubGlobal(
		"fetch",
		async () => new Response("Unauthorized", { status: 401 }),
	);
	await expect(
		fetcher(
			profile,
			"user/info",
			{ headers: { Authorization: "Bearer separate-token" } },
			auth,
		),
	).rejects.toMatchObject({ status: 401 });
	expect(reportUnauthorized).not.toHaveBeenCalled();
});

test.each(["fetch", "stream"] as const)(
	"WebApiState.%s reports the auth context that issued the request",
	async (method) => {
		const auth = makeAuth();
		const backend: WebBackendRef = { auth };
		vi.stubGlobal("fetch", async () => {
			backend.auth = makeAuth("new-session-token");
			return new Response("Unauthorized", { status: 401 });
		});
		await expect(
			new WebApiState(backend)[method](profile, "user/info"),
		).rejects.toMatchObject({ status: 401 });
		expect(reportUnauthorized).toHaveBeenCalledExactlyOnceWith(
			auth,
			"request-token",
		);
	},
);
