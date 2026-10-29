import {
	type OAuthSession,
	normalizeOAuthHub,
	oauthSessionKey,
} from "@flow-like/flow-like-ui/lib/oauth/scope";
import type { OAuthPlatform } from "@flow-like/flow-like-ui/lib/oauth/service";
import type {
	IOAuthBinding,
	IOAuthRuntime,
} from "@flow-like/flow-like-ui/lib/oauth/types";
import type { AuthContextProps } from "react-oidc-context";
import { fetchWithSession } from "./session-request";

/** The Hub user web OAuth connections belong to. */
export interface OAuthAccount {
	hub?: string;
	auth?: AuthContextProps;
}

export function oauthAccountSession(
	account: OAuthAccount | null | undefined,
): OAuthSession | undefined {
	const user = account?.auth?.user;
	if (!account?.hub || !user?.profile.sub) return undefined;
	return {
		hub: account.hub,
		subject: user.profile.sub,
		issuer: user.profile.iss,
	};
}

/** Providers open in a new tab; the account's session goes only to its own Hub's OAuth routes. */
export function createWebOAuthRuntime(
	account: () => OAuthAccount | null | undefined,
): IOAuthRuntime {
	const request = async (
		url: string,
		init: RequestInit,
		binding?: IOAuthBinding,
	): Promise<Response> => {
		if (!new URL(url).pathname.includes("/api/v1/oauth/"))
			return fetch(url, init);
		const current = account();
		const session = oauthAccountSession(current);
		if (
			!session ||
			!url.startsWith(`${normalizeOAuthHub(session.hub)}/api/v1/oauth/`) ||
			!current?.auth?.user?.access_token ||
			(binding && binding.sessionKey !== oauthSessionKey(session))
		)
			throw new Error(
				"Sign in to the OAuth connection's Hub before continuing.",
			);
		return fetchWithSession(url, init, current.auth);
	};
	return {
		async openUrl(url) {
			window.open(url, "_blank", "noopener,noreferrer");
		},
		async httpPost(url, body, headers, binding) {
			return request(url, { method: "POST", body, headers }, binding);
		},
		async httpGet(url, headers, binding) {
			return request(url, { method: "GET", headers }, binding);
		},
	};
}

/** Encoded into the OAuth state, it tells the website callback which web app to return to. */
export function webOAuthPlatform(): OAuthPlatform {
	if (typeof window === "undefined") return "web-prod";
	const { hostname } = window.location;
	return hostname === "localhost" || hostname === "127.0.0.1"
		? "web-dev"
		: "web-prod";
}
