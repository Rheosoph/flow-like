// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { oauthDB, oauthTokenStore } from "@flow-like/flow-like-ui/db/oauth-db";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { normalizeOAuthHub } from "@flow-like/flow-like-ui/lib/oauth/scope";
import type { IStoredOAuthToken } from "@flow-like/flow-like-ui/lib/oauth/types";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	type HostedBootstrap,
	createHostedBackend,
} from "../lib/hosted-backend";
import { HostedOAuth } from "../lib/hosted-oauth";
import { broadcastOAuthCallbackCompletion } from "../lib/oauth-callback-storage";
import { HostedOAuthPrompt } from "./hosted-oauth-prompt";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

/** Delivers within this window, as the browser does between the callback tab and this one. */
class LocalChannel {
	static readonly open = new Set<LocalChannel>();
	private readonly listeners = new Set<(event: MessageEvent) => void>();
	constructor(readonly name: string) {
		LocalChannel.open.add(this);
	}
	addEventListener(_type: "message", listener: (event: MessageEvent) => void) {
		this.listeners.add(listener);
	}
	removeEventListener(
		_type: "message",
		listener: (event: MessageEvent) => void,
	) {
		this.listeners.delete(listener);
	}
	postMessage(data: unknown) {
		for (const channel of LocalChannel.open) {
			if (channel === this || channel.name !== this.name) continue;
			for (const listener of channel.listeners)
				listener({ data: structuredClone(data) } as MessageEvent);
		}
	}
	close() {
		LocalChannel.open.delete(this);
	}
}

const PROMPT_TITLE = "Third-Party Authorization Required";
const hub = getApiOrigin();
const requirement = {
	provider_id: "google",
	scopes: ["https://www.googleapis.com/auth/drive.readonly"],
};
const google = {
	name: "Google",
	client_id: "google-client",
	auth_url: "https://accounts.google.test/auth",
	token_url: "https://accounts.google.test/token",
	scopes: ["openid"],
	pkce_required: true,
	requires_secret_proxy: true,
};

let root: Root;
let container: HTMLDivElement;
let oauth: HostedOAuth;
let release: () => void;
let visitor: string;
const calls: { suffix: string; body?: Record<string, unknown> }[] = [];
const opened: string[] = [];

function hostedBackend(auth_proxy: boolean, token: string | undefined) {
	return createHostedBackend(
		{
			app_id: "published-app",
			auth_proxy,
			bootstrap: { event: { id: "published-event" } },
		} as HostedBootstrap,
		async (suffix, init) => {
			calls.push({
				suffix: suffix ?? "",
				body: init?.body ? JSON.parse(String(init.body)) : undefined,
			});
			if (suffix === "/prerun")
				return Response.json({ oauth_requirements: [requirement] });
			return new Response(
				[
					'data: {"event_type":"run_initiated","payload":{"run_id":"run-1"}}',
					'data: {"event_type":"completed","payload":{"status":"completed"}}',
					"",
				].join("\n\n"),
			);
		},
		() => token,
		undefined,
		oauth.tokensFor,
	);
}

const run = (backend: ReturnType<typeof hostedBackend>) =>
	backend.eventState.executeEvent(
		"published-app",
		"published-event",
		{ id: "node", payload: { question: "status" } },
		false,
	);

const invokes = () =>
	calls.filter(({ suffix }) => suffix === "/invoke").map(({ body }) => body);

const pageText = () => document.body.textContent ?? "";

const button = (label: string) =>
	Array.from(document.body.querySelectorAll("button")).find(
		(candidate) => candidate.textContent?.trim() === label,
	);

async function until(condition: () => boolean) {
	for (let attempt = 0; attempt < 200 && !condition(); attempt++)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
	expect(condition()).toBe(true);
}

async function click(element: Element | null | undefined) {
	expect(element).toBeTruthy();
	await act(async () => {
		(element as HTMLElement).click();
	});
}

beforeEach(async () => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	vi.stubGlobal("BroadcastChannel", LocalChannel);
	vi.stubGlobal(
		"fetch",
		vi.fn(async (input: RequestInfo | URL) => {
			if (String(input) === `${hub}/api/v1`)
				return Response.json({ oauth_providers: { google } });
			throw new Error(`Unexpected request to ${String(input)}`);
		}),
	);
	vi.spyOn(window, "open").mockImplementation((url) => {
		opened.push(String(url));
		return null;
	});
	await Promise.all([
		oauthDB.scopedTokens.clear(),
		oauthDB.scopedConsents.clear(),
		oauthDB.scopedPendingAuth.clear(),
	]);
	calls.length = 0;
	opened.length = 0;
	visitor = crypto.randomUUID();
	const auth = {
		user: {
			access_token: "visitor-token",
			profile: { sub: visitor, iss: "https://login.test" },
		},
	} as unknown as AuthContextProps;
	oauth = new HostedOAuth("published-app", () => auth);
	release = oauth.open();
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
	await act(async () => root.render(<HostedOAuthPrompt oauth={oauth} />));
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	release();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe("HostedOAuthPrompt", () => {
	it("asks a signed-in visitor to connect and sends the connected sign-in with the run", async () => {
		const backend = hostedBackend(true, "visitor-token");
		const running = run(backend);
		await until(
			() => pageText().includes(PROMPT_TITLE) && pageText().includes("Google"),
		);
		expect(invokes()).toEqual([]);

		await click(document.getElementById("acknowledge"));
		await click(button("Authorize"));
		await until(() => opened.length === 1);
		expect(opened[0].startsWith(`${google.auth_url}?`)).toBe(true);

		// The callback tab exchanges the code, stores the sign-in and announces it.
		const state = new URL(opened[0]).searchParams.get("state") ?? "";
		const pending = await oauthTokenStore.getPendingAuth(state);
		if (!pending) throw new Error("Authorize stored no pending sign-in");
		const token: IStoredOAuthToken = {
			providerId: "google",
			binding: pending.binding,
			access_token: "visitor-google",
			refresh_token: "fl_oauth_visitor",
			token_type: "Bearer",
			scopes: ["openid", ...requirement.scopes],
			storedAt: Date.now(),
		};
		await oauthTokenStore.setToken(token);
		await act(async () => broadcastOAuthCallbackCompletion(pending, token));

		await click(button("Continue"));
		await running;
		const carried = {
			payload: { question: "status" },
			oauth_tokens: {
				google: {
					hub_origin: normalizeOAuthHub(hub),
					hub_subject: visitor,
					access_token: "visitor-google",
					refresh_token: "fl_oauth_visitor",
					token_type: "Bearer",
				},
			},
		};
		expect(invokes()).toEqual([carried]);
		expect(pageText()).not.toContain(PROMPT_TITLE);

		// "Don't ask me again" starts checked, so the next run carries it unasked.
		await run(backend);
		expect(invokes()).toEqual([carried, carried]);
		expect(pageText()).not.toContain(PROMPT_TITLE);
	});

	it("runs on the app's saved sign-ins once the visitor skips, without asking again", async () => {
		const backend = hostedBackend(true, "visitor-token");
		const running = run(backend);
		await until(() => button("Skip") !== undefined);
		expect(pageText()).toContain(
			"Connect your own accounts, or skip to use the sign-ins saved for this app.",
		);

		await click(button("Skip"));
		await running;
		await run(backend);

		expect(invokes()).toEqual([
			{ payload: { question: "status" } },
			{ payload: { question: "status" } },
		]);
		expect(pageText()).not.toContain(PROMPT_TITLE);
	});

	it("never shows an anonymous visitor the prompt", async () => {
		await run(hostedBackend(false, "ambient-token"));
		await run(hostedBackend(true, undefined));

		expect(calls.map(({ suffix }) => suffix)).toEqual(["/invoke", "/invoke"]);
		expect(invokes()).toEqual([
			{ payload: { question: "status" } },
			{ payload: { question: "status" } },
		]);
		expect(pageText()).not.toContain(PROMPT_TITLE);
		expect(fetch).not.toHaveBeenCalled();
	});
});
