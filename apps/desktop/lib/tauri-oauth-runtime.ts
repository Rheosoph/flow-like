import {
	type IOAuthBinding,
	type IOAuthRuntime,
	type IProfile,
	normalizeOAuthHub,
	oauthSessionKey,
	useBackendStore,
} from "@flow-like/flow-like-ui";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AuthContextProps } from "react-oidc-context";
import { hubAuthHeaders, withHubAuth } from "./hub-auth";

async function oauthRequest(
	url: string,
	init: RequestInit,
	binding?: IOAuthBinding,
): Promise<Response> {
	if (!new URL(url).pathname.includes("/api/v1/oauth/"))
		return tauriFetch(url, init);
	const backend = useBackendStore.getState().backend as {
		profile?: IProfile;
		auth?: AuthContextProps;
	} | null;
	const profile = backend?.profile;
	const hub = profile?.hub;
	if (
		!profile ||
		!hub ||
		!url.startsWith(`${normalizeOAuthHub(hub)}/api/v1/oauth/`) ||
		!backend.auth?.user?.access_token ||
		!backend.auth.user.profile.sub ||
		(binding &&
			binding.sessionKey !==
				oauthSessionKey({
					profileId: profile.id,
					hub,
					subject: backend.auth.user?.profile.sub ?? "",
					issuer: backend.auth.user?.profile.iss,
				}))
	)
		throw new Error("Sign in to the OAuth connection's Hub before continuing.");
	return withHubAuth(profile, backend.auth, (token) =>
		tauriFetch(url, { ...init, headers: hubAuthHeaders(init.headers, token) }),
	);
}

export const tauriOAuthRuntime: IOAuthRuntime = {
	async openUrl(url) {
		await openUrl(url);
	},
	async httpPost(url, body, headers, binding) {
		return oauthRequest(url, { method: "POST", body, headers }, binding);
	},
	async httpGet(url, headers, binding) {
		return oauthRequest(url, { method: "GET", headers }, binding);
	},
};
