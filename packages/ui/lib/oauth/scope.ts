import type { IOAuthBinding, IOAuthProvider } from "./types";

export interface OAuthSession {
	profileId?: string;
	hub: string;
	subject: string;
	issuer?: string;
	/** Signed out of the Hub: grants stay on this device, apart from every Hub account. */
	local?: true;
}

export function localOAuthSession(
	hub: string,
	profileId?: string,
): OAuthSession {
	return { hub, subject: "", profileId, local: true };
}

export function normalizeOAuthHub(hub: string): string {
	const url = new URL(hub.includes("://") ? hub : `https://${hub}`);
	if (url.protocol !== "https:" && url.protocol !== "http:")
		throw new Error("Invalid OAuth Hub URL");
	return url.href.replace(/\/+$/, "");
}

export function oauthSessionKey(session: OAuthSession): string {
	if (session.local)
		return JSON.stringify([
			"local",
			normalizeOAuthHub(session.hub),
			session.profileId ?? "",
		]);
	return JSON.stringify([
		normalizeOAuthHub(session.hub),
		session.issuer ?? "",
		session.subject,
		...(session.profileId ? [session.profileId] : []),
	]);
}

export function oauthProviderKey(provider: IOAuthProvider): string {
	return JSON.stringify([
		provider.id,
		provider.client_id,
		provider.auth_url,
		provider.token_url,
	]);
}

export function oauthBinding(
	session: OAuthSession,
	provider: IOAuthProvider,
): IOAuthBinding {
	return {
		sessionKey: oauthSessionKey(session),
		providerKey: oauthProviderKey(provider),
		apiBaseUrl: normalizeOAuthHub(session.hub),
		...(session.local ? { local: true as const } : {}),
	};
}

/** The binding when a Hub account owns it; local grants never reach the Hub as connections. */
export function hubBinding(binding?: IOAuthBinding): IOAuthBinding | undefined {
	return binding?.local ? undefined : binding;
}

export function oauthCredentialKey(binding: IOAuthBinding): string {
	return JSON.stringify([binding.sessionKey, binding.providerKey]);
}
