import Dexie, { type EntityTable } from "dexie";
import {
	type OAuthSession,
	oauthBinding,
	oauthCredentialKey,
	oauthSessionKey,
} from "../lib/oauth/scope";
import { hasRequiredScopes } from "../lib/oauth/token-scopes";
import type {
	IOAuthBinding,
	IOAuthPendingAuth,
	IOAuthProvider,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../lib/oauth/types";

interface StoredToken extends IStoredOAuthToken {
	id: string;
	sessionKey: string;
}
interface StoredConsent {
	id: string;
	sessionKey: string;
	credentialKey: string;
	appId: string;
	providerId: string;
	consentedAt: number;
	scopes: string[];
}
interface RefreshLease {
	id: string;
	owner: string;
	expiresAt: number;
}

const oauthDB = new Dexie("OAuthTokens") as Dexie & {
	scopedTokens: EntityTable<StoredToken, "id">;
	scopedPendingAuth: EntityTable<IOAuthPendingAuth, "state">;
	scopedConsents: EntityTable<StoredConsent, "id">;
	refreshLeases: EntityTable<RefreshLease, "id">;
};
oauthDB.version(1).stores({
	tokens: "providerId, expiresAt",
	pendingAuth: "state, providerId, initiatedAt",
});
oauthDB.version(2).stores({
	tokens: "providerId, expiresAt",
	pendingAuth: "state, providerId, initiatedAt",
	consents: "id, appId, providerId, consentedAt",
});
// Legacy records have no owner or client identity and cannot be assigned safely.
oauthDB.version(3).stores({
	tokens: null,
	pendingAuth: null,
	consents: null,
	scopedTokens: "id, sessionKey, providerId",
	scopedPendingAuth: "state, initiatedAt",
	scopedConsents: "id, sessionKey, appId",
	refreshLeases: "id",
});
export { oauthDB };

let sessionResolver: () => OAuthSession | undefined = () => undefined;
const bindings = new Map<string, IOAuthBinding>();
/** The returned release reinstates the previous resolver unless another replaced this one since. */
export function configureOAuthSession(
	resolver: () => OAuthSession | undefined,
): () => void {
	const previous = sessionResolver;
	sessionResolver = resolver;
	return () => {
		if (sessionResolver === resolver) sessionResolver = previous;
	};
}
function currentSessionKey(): string | undefined {
	const session = sessionResolver();
	return session?.subject ? oauthSessionKey(session) : undefined;
}
function bindingFor(
	providerId: string,
	provider?: IOAuthProvider,
): IOAuthBinding | undefined {
	const session = sessionResolver();
	if (!session?.subject) return undefined;
	const key = JSON.stringify([oauthSessionKey(session), providerId]);
	if (provider) bindings.set(key, oauthBinding(session, provider));
	return bindings.get(key);
}
function isCurrent(binding: IOAuthBinding): boolean {
	if (binding.sessionKey !== currentSessionKey()) return false;
	const [providerId] = JSON.parse(binding.providerKey) as [string];
	return bindingFor(providerId)?.providerKey === binding.providerKey;
}
function requireBinding(binding: IOAuthBinding | undefined): IOAuthBinding {
	if (!binding || !isCurrent(binding))
		throw new Error("The OAuth account or provider changed. Authorize again.");
	return binding;
}
function sameToken(
	a: IStoredOAuthToken | undefined,
	b: IStoredOAuthToken,
): boolean {
	return (
		!!a &&
		a.access_token === b.access_token &&
		a.refresh_token === b.refresh_token &&
		a.storedAt === b.storedAt
	);
}
async function consentFor(
	appId: string,
	providerId: string,
): Promise<StoredConsent | undefined> {
	const binding = bindingFor(providerId);
	if (!binding) return undefined;
	const consent = await oauthDB.scopedConsents.get(
		JSON.stringify([appId, oauthCredentialKey(binding)]),
	);
	return isCurrent(binding) ? consent : undefined;
}
async function currentConsents(): Promise<StoredConsent[]> {
	const sessionKey = currentSessionKey();
	if (!sessionKey) return [];
	const consents = await oauthDB.scopedConsents
		.where("sessionKey")
		.equals(sessionKey)
		.toArray();
	return sessionKey === currentSessionKey()
		? consents.filter((c) => {
				const binding = bindingFor(c.providerId);
				return binding && oauthCredentialKey(binding) === c.credentialKey;
			})
		: [];
}
export interface IOAuthConsentStore {
	/** Check if user has consented to a provider for a specific app/workflow */
	hasConsent(appId: string, providerId: string): Promise<boolean>;
	/** Check if user has consented to a provider with all required scopes */
	hasConsentWithScopes(
		appId: string,
		providerId: string,
		requiredScopes: string[],
	): Promise<boolean>;
	/** Save consent for a provider in a specific app/workflow */
	setConsent(
		appId: string,
		providerId: string,
		scopes: string[],
	): Promise<void>;
	/** Revoke consent for a provider in a specific app/workflow */
	revokeConsent(appId: string, providerId: string): Promise<void>;
	/** Revoke all consents for an app/workflow */
	revokeAllConsentsForApp(appId: string): Promise<void>;
	/** Get all consents */
	getAllConsents(): Promise<
		{
			appId: string;
			providerId: string;
			consentedAt: number;
			scopes: string[];
		}[]
	>;
	/** Get consented provider IDs for a specific app/workflow */
	getConsentedProviderIds(appId: string): Promise<Set<string>>;
	/** Get consented provider IDs that have all required scopes */
	getConsentedProviderIdsWithScopes(
		appId: string,
		requiredScopesMap: Map<string, string[]>,
	): Promise<Set<string>>;
}

export const oauthConsentStore: IOAuthConsentStore = {
	async hasConsent(appId, providerId) {
		return !!(await consentFor(appId, providerId));
	},
	async hasConsentWithScopes(appId, providerId, requiredScopes) {
		const consent = await consentFor(appId, providerId);
		return !!consent && hasRequiredScopes(consent, requiredScopes);
	},
	async setConsent(appId, providerId, scopes) {
		const binding = requireBinding(bindingFor(providerId));
		const credentialKey = oauthCredentialKey(binding);
		await oauthDB.transaction(
			"rw",
			oauthDB.scopedTokens,
			oauthDB.scopedConsents,
			async () => {
				const token = await oauthDB.scopedTokens.get(credentialKey);
				if (!isCurrent(binding) || !token || !hasRequiredScopes(token, scopes))
					throw new Error(
						"Authorize all required scopes before remembering consent",
					);
				await oauthDB.scopedConsents.put({
					id: JSON.stringify([appId, credentialKey]),
					sessionKey: binding.sessionKey,
					credentialKey,
					appId,
					providerId,
					scopes,
					consentedAt: Date.now(),
				});
			},
		);
	},
	async revokeConsent(appId, providerId) {
		const binding = bindingFor(providerId);
		if (binding)
			await oauthDB.scopedConsents.delete(
				JSON.stringify([appId, oauthCredentialKey(binding)]),
			);
	},
	async revokeAllConsentsForApp(appId) {
		await oauthDB.scopedConsents.bulkDelete(
			(await currentConsents())
				.filter((c) => c.appId === appId)
				.map((c) => c.id),
		);
	},
	async getAllConsents() {
		return currentConsents();
	},
	async getConsentedProviderIds(appId) {
		return new Set(
			(await currentConsents())
				.filter((c) => c.appId === appId)
				.map((c) => c.providerId),
		);
	},
	async getConsentedProviderIdsWithScopes(appId, requiredScopesMap) {
		return new Set(
			(await currentConsents())
				.filter(
					(c) =>
						c.appId === appId &&
						hasRequiredScopes(c, requiredScopesMap.get(c.providerId) ?? []),
				)
				.map((c) => c.providerId),
		);
	},
};

export const oauthTokenStore: IOAuthTokenStoreWithPending = {
	async getBinding(provider) {
		const binding = bindingFor(provider.id, provider);
		if (!binding)
			throw new Error(
				"Sign in to your Hub before connecting an OAuth provider.",
			);
		return binding;
	},
	async isCurrentBinding(binding) {
		return isCurrent(binding);
	},
	async getToken(providerId, provider) {
		const binding = bindingFor(providerId, provider);
		if (!binding) return undefined;
		const token = await oauthDB.scopedTokens.get(oauthCredentialKey(binding));
		return isCurrent(binding) ? token : undefined;
	},
	async setToken(token) {
		const binding = requireBinding(token.binding);
		await oauthDB.scopedTokens.put({
			...token,
			id: oauthCredentialKey(binding),
			sessionKey: binding.sessionKey,
		});
	},
	async deleteToken(providerId) {
		const binding = bindingFor(providerId);
		if (binding) await oauthDB.scopedTokens.delete(oauthCredentialKey(binding));
	},
	async replaceToken(expected, replacement) {
		const binding = requireBinding(expected.binding);
		const id = oauthCredentialKey(binding);
		return oauthDB.transaction("rw", oauthDB.scopedTokens, async () => {
			const current = await oauthDB.scopedTokens.get(id);
			if (!isCurrent(binding) || !sameToken(current, expected)) return false;
			if (replacement)
				await oauthDB.scopedTokens.put({
					...replacement,
					id,
					sessionKey: binding.sessionKey,
				});
			else await oauthDB.scopedTokens.delete(id);
			return true;
		});
	},
	async getAllTokens() {
		const sessionKey = currentSessionKey();
		if (!sessionKey) return [];
		const tokens = await oauthDB.scopedTokens
			.where("sessionKey")
			.equals(sessionKey)
			.toArray();
		return sessionKey === currentSessionKey() ? tokens : [];
	},
	isExpired(token, bufferMs = 60000) {
		return (
			token.expires_at !== undefined &&
			Date.now() + bufferMs >= token.expires_at
		);
	},
	async setPendingAuth(pending) {
		requireBinding(pending.binding);
		await oauthDB.scopedPendingAuth.put(pending);
	},
	async getPendingAuth(state) {
		const pending = await oauthDB.scopedPendingAuth.get(state);
		if (
			!pending?.binding ||
			pending.initiatedAt < Date.now() - 600000 ||
			pending.binding.sessionKey !== currentSessionKey()
		)
			return undefined;
		if (!bindingFor(pending.providerId) && pending.provider)
			bindingFor(pending.providerId, pending.provider);
		return isCurrent(pending.binding) ? pending : undefined;
	},
	async consumePendingAuth(state) {
		return oauthDB.transaction("rw", oauthDB.scopedPendingAuth, async () => {
			const pending = await this.getPendingAuth(state);
			if (pending) await oauthDB.scopedPendingAuth.delete(state);
			return pending;
		});
	},
	async cleanupPendingAuth() {
		await oauthDB.scopedPendingAuth
			.where("initiatedAt")
			.below(Date.now() - 600000)
			.delete();
	},
	async withRefreshLock(token, operation) {
		const binding = requireBinding(token.binding);
		const id = oauthCredentialKey(binding);
		if (typeof navigator !== "undefined" && navigator.locks)
			return navigator.locks.request(`flow-like-oauth:${id}`, operation);
		// IndexedDB leases coordinate webviews that do not implement Web Locks.
		const owner = crypto.randomUUID();
		for (;;) {
			requireBinding(binding);
			const acquired = await oauthDB.transaction(
				"rw",
				oauthDB.refreshLeases,
				async () => {
					const lease = await oauthDB.refreshLeases.get(id);
					if (lease && lease.expiresAt > Date.now()) return false;
					await oauthDB.refreshLeases.put({
						id,
						owner,
						expiresAt: Date.now() + 30000,
					});
					return true;
				},
			);
			if (acquired) break;
			await new Promise((resolve) => setTimeout(resolve, 100));
		}
		const renew = setInterval(() => {
			void oauthDB
				.transaction("rw", oauthDB.refreshLeases, async () => {
					if ((await oauthDB.refreshLeases.get(id))?.owner === owner)
						await oauthDB.refreshLeases.put({
							id,
							owner,
							expiresAt: Date.now() + 30000,
						});
				})
				.catch(() => {});
		}, 10000);
		try {
			return await operation();
		} finally {
			clearInterval(renew);
			await oauthDB.transaction("rw", oauthDB.refreshLeases, async () => {
				if ((await oauthDB.refreshLeases.get(id))?.owner === owner)
					await oauthDB.refreshLeases.delete(id);
			});
		}
	},
};
