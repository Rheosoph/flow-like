import { invoke, isTauri } from "@tauri-apps/api/core";
import {
	type StateStore,
	User,
	type UserManager,
	type UserManagerSettings,
} from "oidc-client-ts";

interface NativeSnapshot {
	generation: string;
	initialized: boolean;
	user: string | null;
	authority: string | null;
	clientId: string | null;
}

/** OIDC supplies React state; Rust owns sign-in, persisted grants, and renewal. */
export class NativeUserStore implements StateStore {
	private snapshot?: NativeSnapshot;
	private tokenEndpoint?: () => Promise<string | undefined>;
	private loading?: Promise<string | null>;

	constructor(
		private readonly profileId: string,
		private readonly hub: string,
		private readonly settings: UserManagerSettings,
		private readonly legacy: StateStore,
	) {}

	bind(manager: UserManager): void {
		this.tokenEndpoint = () => manager.metadataService.getTokenEndpoint();
		const userKey = `user:${this.settings.authority}:${this.settings.client_id}`;
		let loadedUser: string | null | undefined;
		manager.getUser = async (raiseEvent = false) => {
			const serialized = await this.get(userKey);
			if (!serialized) {
				loadedUser = null;
				return null;
			}
			const user = User.fromStorageString(serialized);
			// OIDC's default read rearms the expiry timer. Renewal also reads the
			// user, so a failed request near expiry would retry every second.
			if (raiseEvent || serialized !== loadedUser) {
				loadedUser = serialized;
				await manager.events.load(user, raiseEvent);
			}
			return user;
		};
		manager.signinRedirect = async () => {
			await invoke<void>("begin_login", { profileId: this.profileId });
		};
		manager.signoutRedirect = async () => {
			await invoke<void>("native_auth_logout", { profileId: this.profileId });
			this.snapshot = await this.read();
			loadedUser = null;
			await manager.events.unload();
		};
		manager.signinSilent = async () => {
			const user = await manager.getUser();
			if (!user) return null;
			const generation = this.snapshot?.generation;
			try {
				const snapshot = await invoke<NativeSnapshot>("native_auth_refresh", {
					profileId: this.profileId,
					hub: this.hub,
					subject: user.profile.sub,
					generation,
					rejectedToken: user.access_token,
				});
				this.snapshot = snapshot;
				const renewed = snapshot.user
					? User.fromStorageString(snapshot.user)
					: null;
				if (renewed) {
					loadedUser = snapshot.user;
					await manager.events.load(renewed);
				}
				return renewed;
			} catch (error) {
				if (error === "invalid_grant") {
					this.snapshot = await this.read();
					throw Object.assign(new Error("Sign in again to continue."), {
						error: "invalid_grant",
					});
				}
				throw error;
			}
		};
	}

	private read(): Promise<NativeSnapshot> {
		return invoke<NativeSnapshot>("native_auth_read", {
			profileId: this.profileId,
			hub: this.hub,
		});
	}

	get(key: string): Promise<string | null> {
		this.loading ??= (async () => {
			this.snapshot = await this.read();
			if (!this.snapshot.initialized) {
				const legacyUser = await this.legacy.get(key);
				if (legacyUser) await this.set(key, legacyUser);
			} else {
				// A previous window may have committed native auth just before closing.
				// Once that commit is readable, no browser refresh grant is needed.
				await this.legacy.remove(key);
			}
			return this.snapshot.authority === this.settings.authority &&
				this.snapshot.clientId === this.settings.client_id
				? this.snapshot.user
				: null;
		})().finally(() => {
			this.loading = undefined;
		});
		return this.loading;
	}

	async set(key: string, user: string): Promise<void> {
		this.snapshot ??= await this.read();
		const generation = this.snapshot.generation;
		const tokenEndpoint = await this.tokenEndpoint?.();
		if (!tokenEndpoint)
			throw new Error("The identity provider has no token endpoint.");
		this.snapshot = await invoke<NativeSnapshot>("native_auth_write", {
			profileId: this.profileId,
			hub: this.hub,
			generation,
			configuration: {
				authority: this.settings.authority,
				clientId: this.settings.client_id,
				tokenEndpoint,
			},
			user,
		});
		// Delete the browser copy only after the native encrypted commit succeeded.
		await this.legacy.remove(key);
	}

	async remove(key: string): Promise<string | null> {
		this.snapshot ??= await this.read();
		const previous = this.snapshot.user;
		this.snapshot = await invoke<NativeSnapshot>("native_auth_clear", {
			profileId: this.profileId,
			hub: this.hub,
			generation: this.snapshot.generation,
		});
		await this.legacy.remove(key);
		return previous;
	}

	async getAllKeys(): Promise<string[]> {
		return [`user:${this.settings.authority}:${this.settings.client_id}`];
	}
}

export function nativeUserStore(
	profileId: string,
	hub: string,
	settings: UserManagerSettings,
	legacy: StateStore,
): StateStore {
	return isTauri()
		? new NativeUserStore(profileId, hub, settings, legacy)
		: legacy;
}
