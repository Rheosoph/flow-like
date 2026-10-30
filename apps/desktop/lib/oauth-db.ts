import {
	configureOAuthSession,
	oauthDB,
	oauthConsentStore as browserConsentStore,
	oauthTokenStore as browserTokenStore,
	oauthSessionKey,
	normalizeOAuthHub,
	useBackendStore,
} from "@flow-like/flow-like-ui";
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { AuthContextProps } from "react-oidc-context";

configureOAuthSession(() => {
	const backend = useBackendStore.getState().backend as {
		profile?: { id?: string; hub?: string };
		auth?: AuthContextProps;
	} | null;
	const user = backend?.auth?.user;
	const hub = backend?.profile?.hub;
	if (!hub || !user?.profile.sub) return undefined;
	return {
		hub,
		subject: user.profile.sub,
		issuer: user.profile.iss,
		profileId: backend?.profile?.id,
	};
});

export { oauthDB };

function owner() {
	const backend = useBackendStore.getState().backend as {
		profile?: { id?: string; hub?: string };
		auth?: AuthContextProps;
	} | null;
	const profile = backend?.profile;
	const user = backend?.auth?.user;
	if (!isTauri() || !profile?.id || !profile.hub || !user?.profile.sub)
		return undefined;
	return {
		profileId: profile.id,
		hub: normalizeOAuthHub(profile.hub),
		subject: user.profile.sub,
		issuer: user.profile.iss,
	};
}

type Owner = NonNullable<ReturnType<typeof owner>>;
let persistence: Promise<void> = Promise.resolve();

function queued<T>(operation: () => Promise<T>): Promise<T> {
	const pending = persistence.then(operation, operation);
	persistence = pending.then(
		() => undefined,
		() => undefined,
	);
	return pending;
}

function requireOwner(captured: Owner) {
	if (JSON.stringify(owner()) !== JSON.stringify(captured))
		throw new Error(
			"The OAuth profile or account changed while saving connections.",
		);
}

async function browserSnapshot(captured: Owner) {
	const sessionKey = oauthSessionKey(captured);
	const [tokens, consents] = await Promise.all([
		oauthDB.scopedTokens.where("sessionKey").equals(sessionKey).toArray(),
		oauthDB.scopedConsents.where("sessionKey").equals(sessionKey).toArray(),
	]);
	return { tokens, consents };
}

/** Preserve account-scoped grants and consent for execution after the webview closes. */
export function syncNativeOAuth(
	captured = owner(),
	generation?: string,
	previous?: Awaited<ReturnType<typeof browserSnapshot>>,
): Promise<void> {
	if (!captured) return Promise.resolve();
	return queued(() => writeNativeOAuth(captured, generation, previous));
}

async function writeNativeOAuth(
	captured: Owner,
	generation?: string,
	previous?: Awaited<ReturnType<typeof browserSnapshot>>,
): Promise<void> {
	const session = await invoke<{ generation: string; user?: string }>(
		"native_auth_read",
		{ profileId: captured.profileId, hub: captured.hub },
	);
	const nativeUser = session.user ? JSON.parse(session.user) : undefined;
	if (
		nativeUser?.profile?.sub !== captured.subject ||
		nativeUser?.profile?.iss !== captured.issuer
	) {
		if (generation)
			throw new Error(
				"The authentication account changed while saving OAuth connections.",
			);
		return;
	}
	if (generation && session.generation !== generation)
		throw new Error(
			"The authentication session changed while saving OAuth connections.",
		);
	const snapshot = await browserSnapshot(captured);
	await invoke("native_oauth_sync", {
		profileId: captured.profileId,
		hub: captured.hub,
		generation: session.generation,
		snapshot,
		previous,
		initializeOnly: previous === undefined,
	});
}

function persisted<T extends object>(store: T, mutations: Set<PropertyKey>): T {
	return new Proxy(store, {
		get(target, property, receiver) {
			const operation = Reflect.get(target, property, receiver);
			if (!mutations.has(property) || typeof operation !== "function")
				return operation;
			return async (...args: unknown[]) => {
				const captured = owner();
				const session = captured
					? await invoke<{ generation: string }>("native_auth_read", {
							profileId: captured.profileId,
							hub: captured.hub,
						})
					: undefined;
				return queued(async () => {
					if (captured) requireOwner(captured);
					if (captured) await writeNativeOAuth(captured, session?.generation);
					const previous = captured
						? await browserSnapshot(captured)
						: undefined;
					if (captured) requireOwner(captured);
					const result = await operation.apply(target, args);
					if (captured)
						await writeNativeOAuth(captured, session?.generation, previous);
					return result;
				});
			};
		},
	});
}

export const oauthTokenStore = persisted(
	browserTokenStore,
	new Set(["setToken", "deleteToken", "replaceToken"]),
);
export const oauthConsentStore = persisted(
	browserConsentStore,
	new Set(["setConsent", "revokeConsent", "revokeAllConsentsForApp"]),
);

let syncedOwner: string | undefined;
useBackendStore.subscribe(() => {
	const captured: Owner | undefined = owner();
	const key = captured ? JSON.stringify(captured) : undefined;
	if (key === syncedOwner) return;
	syncedOwner = key;
	if (captured) {
		void syncNativeOAuth(captured).catch(() => {
			if (syncedOwner === key) syncedOwner = undefined;
		});
	}
});
