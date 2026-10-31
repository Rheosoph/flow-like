"use client";
import {
	useBackend,
	useInvalidateInfiniteInvoke,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import type { IProfile } from "@flow-like/flow-like-ui";
import { createAccountTokenProvider } from "@flow-like/flow-like-ui/components/account/account-session";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { coordinateOidcRenewal } from "@flow-like/flow-like-ui/lib/auth/oidc-renewal";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrent } from "@tauri-apps/plugin-deep-link";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Amplify } from "aws-amplify";
import {
	type AuthTokens,
	type TokenProvider,
	decodeJWT,
} from "aws-amplify/auth";
import {
	type INavigator,
	type IWindow,
	type NavigateParams,
	type NavigateResponse,
	UserManager,
	type UserManagerSettings,
	WebStorageStateStore,
} from "oidc-client-ts";
import {
	Fragment,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import {
	AuthContext,
	type AuthContextProps,
	AuthProvider,
	useAuth,
} from "react-oidc-context";
import { get } from "../lib/api";
import { bindHubAuth, renewHubAuth, setHubAuthActive } from "../lib/hub-auth";
import { NativeUserStore, nativeUserStore } from "../lib/native-auth";
import { SignInLinkDialog } from "./sign-in-link-dialog";
import { ProfileSyncer, TauriBackend } from "./tauri-provider";

const AUTH_CHANGED_EVENT = "fl-auth-changed";

function emitAuthChanged() {
	window.dispatchEvent(new CustomEvent(AUTH_CHANGED_EVENT));
}

const UserManagerContext = createContext<UserManager | null>(null);

interface OpenIdConfigResponse extends UserManagerSettings {
	cognito?: { readonly user_pool_id: string };
	userManager?: UserManager;
}

const OPEN_ID_CONFIG_CACHE_PREFIX = "flow-like.openid-config:";

/**
 * The last configuration the hub served. Without one an offline start builds no
 * UserManager, so the stored session is never loaded and the app runs signed out
 * for the whole session.
 */
function readCachedOpenIdConfig(hub: string): OpenIdConfigResponse | undefined {
	try {
		const raw = localStorage.getItem(`${OPEN_ID_CONFIG_CACHE_PREFIX}${hub}`);
		const parsed: unknown = raw ? JSON.parse(raw) : undefined;
		return isRecord(parsed) &&
			typeof parsed.authority === "string" &&
			typeof parsed.client_id === "string"
			? (parsed as unknown as OpenIdConfigResponse)
			: undefined;
	} catch {
		return undefined;
	}
}

function writeCachedOpenIdConfig(hub: string, config: OpenIdConfigResponse) {
	try {
		localStorage.setItem(
			`${OPEN_ID_CONFIG_CACHE_PREFIX}${hub}`,
			JSON.stringify(config),
		);
	} catch {
		// Best effort: only an offline start benefits from the copy.
	}
}

export class OIDCTokenProvider implements TokenProvider {
	private readonly provider;

	constructor(userManager: UserManager) {
		this.provider = createAccountTokenProvider(async () => {
			const user = await userManager.getUser();
			return {
				isAuthenticated: Boolean(user),
				user,
				signinSilent: () => userManager.signinSilent(),
			};
		}, decodeJWT);
	}

	getTokens(options?: { forceRefresh?: boolean }): Promise<AuthTokens | null> {
		return this.provider.getTokens(options);
	}
}

/** Sign-out redirects share the navigator; only an authorization request carries response_type. */
function isAuthorizeRequest(url: string): boolean {
	try {
		return new URL(url).searchParams.has("response_type");
	} catch {
		return false;
	}
}

class TauriWindow implements IWindow {
	private readonly onSignInOpened: (url: string) => void;

	constructor(onSignInOpened: (url: string) => void) {
		this.onSignInOpened = onSignInOpened;
	}

	close() {
		return;
	}
	async navigate(params: NavigateParams): Promise<NavigateResponse> {
		if (isAuthorizeRequest(params.url)) this.onSignInOpened(params.url);
		await openUrl(params.url);
		return { url: params.url };
	}
}

class TauriRedirectNavigator implements INavigator {
	private readonly onSignInOpened: (url: string) => void;

	constructor(onSignInOpened: (url: string) => void) {
		this.onSignInOpened = onSignInOpened;
	}

	async prepare(params: unknown): Promise<IWindow> {
		return new TauriWindow(this.onSignInOpened);
	}

	async callback(url: string, params?: unknown): Promise<void> {
		return;
	}
}

interface AuthConfigState {
	readonly profileId: string;
	readonly settings: UserManagerSettings;
	readonly userManager: UserManager;
	readonly scope: number;
	readonly hub: string;
}

interface LiftedAuth {
	readonly auth: AuthContextProps;
	readonly providerKey: string;
	readonly scope: number;
}

function AuthContextBridge({
	providerKey,
	scope,
	hub,
	active,
	onChange,
}: Readonly<{
	providerKey: string;
	scope: number;
	hub: string;
	active: boolean;
	onChange: (lifted: LiftedAuth) => void;
}>) {
	const auth = useAuth();
	useLayoutEffect(() => {
		const bound = bindHubAuth(auth, hub);
		setHubAuthActive(auth, active && bound);
		onChange({ auth, providerKey, scope });
	}, [auth, providerKey, scope, hub, active, onChange]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: events identifies the manager whose session is being detached.
	useLayoutEffect(() => () => setHubAuthActive(auth, false), [auth.events]);
	return null;
}

export function DesktopAuthProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [authConfig, setAuthConfig] = useState<AuthConfigState>();
	const [lifted, setLifted] = useState<LiftedAuth>();
	const [signInUrl, setSignInUrl] = useState<string>();
	const clearSignInUrl = useCallback(() => setSignInUrl(undefined), []);
	const openIdAuthConfig = authConfig?.settings;
	const userManager = authConfig?.userManager;
	const scope = authConfig?.scope ?? 0;
	const providerKey = authConfig
		? `${authConfig.profileId}|${authConfig.hub}|${authConfig.settings.authority}|${authConfig.settings.client_id}`
		: "loading-auth-config";
	const backend = useBackend();
	const currentProfile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const hubUrl = currentProfile.data?.hub ?? "api.flow-like.com";
	const hubSecure = currentProfile.data?.secure ?? true;
	const currentHub = getApiOrigin({
		hub: hubUrl,
		secure: hubSecure,
	} as IProfile);
	const profileId = currentProfile.data?.id ?? "";
	const matchesHub =
		authConfig?.hub === currentHub && authConfig.profileId === profileId;
	const visibleAuth =
		lifted?.auth && !matchesHub && authConfig
			? { ...lifted.auth, user: null, isAuthenticated: false, isLoading: true }
			: lifted?.auth;
	const signedInToken = visibleAuth?.isAuthenticated
		? visibleAuth.user?.access_token
		: undefined;

	useEffect(() => {
		if (signedInToken) setSignInUrl(undefined);
	}, [signedInToken]);

	useEffect(() => {
		if (matchesHub) userManager?.startSilentRenew();
		else userManager?.stopSilentRenew();
		return () => userManager?.stopSilentRenew();
	}, [matchesHub, userManager]);

	useEffect(() => {
		if (isTauri() && !profileId) return;
		const effectiveProfile = {
			hub: hubUrl,
			secure: hubSecure,
			bits: [],
			created: new Date().toISOString(),
			updated: new Date().toISOString(),
			name: "default",
		} as IProfile;
		let cancelled = false;
		let retryTimer: ReturnType<typeof setTimeout> | undefined;
		let retryDelayMs = 5_000;
		let cachedConfig: OpenIdConfigResponse | undefined;
		const hub = getApiOrigin(effectiveProfile);

		const configure = (response: OpenIdConfigResponse) => {
			if (process.env.NEXT_PUBLIC_REDIRECT_URL)
				response.redirect_uri = process.env.NEXT_PUBLIC_REDIRECT_URL;
			if (process.env.NEXT_PUBLIC_REDIRECT_LOGOUT_URL)
				response.post_logout_redirect_uri =
					process.env.NEXT_PUBLIC_REDIRECT_LOGOUT_URL;
			const store = new WebStorageStateStore({
				store: localStorage,
			});
			response.userStore = nativeUserStore(profileId, hub, response, store);
			response.automaticSilentRenew = true;
			setSignInUrl(undefined);
			const navigator = new TauriRedirectNavigator(setSignInUrl);
			const userManagerInstance = new UserManager(response, navigator);
			if (response.userStore instanceof NativeUserStore)
				response.userStore.bind(userManagerInstance);
			coordinateOidcRenewal(userManagerInstance);
			response.userManager = userManagerInstance;
			const tokenProvider = new OIDCTokenProvider(userManagerInstance);
			if (response.cognito)
				Amplify.configure(
					{
						Auth: {
							Cognito: {
								userPoolClientId: response.client_id,
								userPoolId: response.cognito.user_pool_id,
							},
						},
					},
					{
						Auth: {
							tokenProvider: tokenProvider,
						},
					},
				);
			console.log("[DESKTOPAUTH] Setting openIdAuthConfig and userManager");
			setAuthConfig((previous) => ({
				profileId,
				settings: response,
				userManager: userManagerInstance,
				hub,
				scope:
					previous &&
					(previous.settings.client_id !== response.client_id ||
						previous.settings.authority !== response.authority ||
						previous.hub !== hub ||
						previous.profileId !== profileId)
						? previous.scope + 1
						: (previous?.scope ?? 0),
			}));
		};

		const loadConfig = async () => {
			try {
				const response = await get<OpenIdConfigResponse | undefined>(
					effectiveProfile,
					"auth/openid",
				);
				if (cancelled) return;
				if (response) {
					writeCachedOpenIdConfig(hub, response);
					// The provider keeps the UserManager it mounted with, so a matching
					// fresh copy must not replace the one built from the cache.
					const unchanged =
						cachedConfig?.authority === response.authority &&
						cachedConfig.client_id === response.client_id;
					if (!unchanged) configure(response);
				} else {
					console.warn("OpenID response was falsy, not configuring auth");
				}
			} catch (error) {
				console.error("Failed to fetch OpenID config:", error);
				// Without a config sign-in stays impossible until restart, so an
				// unreachable or failing hub is retried; a 4xx refusal is final.
				if (
					!cancelled &&
					!(error instanceof ApiResponseError && error.status < 500)
				) {
					const cached = cachedConfig ? undefined : readCachedOpenIdConfig(hub);
					if (cached) {
						cachedConfig = cached;
						configure(cached);
					}
					retryTimer = setTimeout(() => {
						retryTimer = undefined;
						void loadConfig();
					}, retryDelayMs);
					retryDelayMs = Math.min(retryDelayMs * 2, 300_000);
				}
			}
		};
		const retryNow = () => {
			if (retryTimer === undefined) return;
			clearTimeout(retryTimer);
			retryTimer = undefined;
			void loadConfig();
		};
		window.addEventListener("online", retryNow);
		void loadConfig();
		return () => {
			cancelled = true;
			clearTimeout(retryTimer);
			window.removeEventListener("online", retryNow);
		};
	}, [hubUrl, hubSecure, profileId]);

	useEffect(() => {
		if (!openIdAuthConfig) return;
		const seenUrls = new Set<string>();

		const normalizeTo = (target: string, source: string) => {
			try {
				const targetUrl = new URL(target);
				const sourceUrl = new URL(source);
				targetUrl.search = sourceUrl.search;
				targetUrl.hash = sourceUrl.hash;
				return targetUrl.toString();
			} catch {
				return source;
			}
		};

		const isUniversalAuthCallback = (rawUrl: string): boolean => {
			try {
				const parsed = new URL(rawUrl);
				if (!(parsed.protocol === "https:" || parsed.protocol === "http:")) {
					return false;
				}

				const host = parsed.hostname.toLowerCase();
				if (
					host !== "app.flow-like.com" &&
					host !== "flow-like.com" &&
					host !== "localhost" &&
					host !== "127.0.0.1"
				) {
					return false;
				}

				const path = parsed.pathname.replace(/^\/+|\/+$/g, "");
				return path === "callback" || path === "desktop/callback";
			} catch {
				return false;
			}
		};

		const isUniversalLogoutCallback = (rawUrl: string): boolean => {
			try {
				const parsed = new URL(rawUrl);
				if (!(parsed.protocol === "https:" || parsed.protocol === "http:")) {
					return false;
				}

				const host = parsed.hostname.toLowerCase();
				if (
					host !== "app.flow-like.com" &&
					host !== "flow-like.com" &&
					host !== "localhost" &&
					host !== "127.0.0.1"
				) {
					return false;
				}

				const path = parsed.pathname.replace(/^\/+|\/+$/g, "");
				return path === "logout" || path === "desktop/logout";
			} catch {
				return false;
			}
		};

		const closeOidcFlowWindows = async () => {
			try {
				const { getAllWindows } = await import("@tauri-apps/api/window");
				const windows = await getAllWindows();
				for (const window of windows) {
					if (window.label === "oidcFlow") {
						window.close();
					}
				}
			} catch {
				// Window API not available on mobile — no-op since mobile uses system browser
			}
		};

		const handleIncomingOidcUrl = async (rawUrl: string) => {
			if (!rawUrl || seenUrls.has(rawUrl)) return;
			seenUrls.add(rawUrl);

			try {
				if (
					isTauri() &&
					new URL(rawUrl).searchParams.get("state")?.startsWith("fl-native-")
				)
					return;
				const isDeepLink = rawUrl.startsWith("flow-like://");
				const signinUrl =
					isDeepLink || isUniversalAuthCallback(rawUrl)
						? normalizeTo(openIdAuthConfig.redirect_uri, rawUrl)
						: rawUrl;
				const logoutUrl =
					openIdAuthConfig.post_logout_redirect_uri &&
					(isDeepLink || isUniversalLogoutCallback(rawUrl))
						? normalizeTo(openIdAuthConfig.post_logout_redirect_uri, rawUrl)
						: rawUrl;

				console.log("[OIDC] Processing callback", { isDeepLink });

				if (signinUrl.startsWith(openIdAuthConfig.redirect_uri)) {
					await userManager?.signinRedirectCallback(signinUrl);
					emitAuthChanged();
					await closeOidcFlowWindows();
				}

				if (
					openIdAuthConfig.post_logout_redirect_uri &&
					logoutUrl.startsWith(openIdAuthConfig.post_logout_redirect_uri)
				) {
					emitAuthChanged();
					await closeOidcFlowWindows();
				}

				if (signinUrl.includes("/login?id_token_hint=")) {
					await closeOidcFlowWindows();
				}
			} catch {
				seenUrls.delete(rawUrl);
				console.error("Failed to process OIDC callback URL");
			}
		};

		const processStartupDeepLinks = async () => {
			try {
				const startupUrls = await getCurrent();
				if (!startupUrls || startupUrls.length === 0) {
					return;
				}

				for (const startupUrl of startupUrls) {
					await handleIncomingOidcUrl(startupUrl);
				}
			} catch (error) {
				console.warn("Failed to process startup deep links for OIDC:", error);
			}
		};

		async function debugListener(event: Event) {
			const url = (event as CustomEvent<{ url?: string }>).detail?.url;
			if (!url) return;
			console.log("Debug OIDC URL received");
			await handleIncomingOidcUrl(url);
		}

		window.addEventListener("debug-oidc", debugListener);

		const unlistenNative = listen<{ profileId: string }>(
			"native-auth-changed",
			async (event) => {
				if (event.payload.profileId !== profileId) return;
				const user = await userManager?.getUser(true);
				if (!user) userManager?.events.unload();
				emitAuthChanged();
			},
		);
		const unlisten = listen<{ url: string }>("oidc/url", async (event) => {
			await handleIncomingOidcUrl(event.payload.url);
		});

		void processStartupDeepLinks();

		return () => {
			unlisten.then((unsub) => unsub());
			unlistenNative.then((unsub) => unsub());
			window.removeEventListener("debug-oidc", debugListener);
		};
	}, [userManager, openIdAuthConfig, profileId]);

	return (
		<UserManagerContext.Provider value={userManager ?? null}>
			{openIdAuthConfig ? (
				<AuthProvider
					key={providerKey}
					{...openIdAuthConfig}
					automaticSilentRenew={true}
					userStore={openIdAuthConfig.userStore}
				>
					<AuthContextBridge
						providerKey={providerKey}
						scope={scope}
						hub={authConfig?.hub ?? currentHub}
						active={matchesHub}
						onChange={setLifted}
					/>
				</AuthProvider>
			) : (
				<AuthProvider key={providerKey}>
					<AuthContextBridge
						providerKey={providerKey}
						scope={scope}
						hub={currentHub}
						active={false}
						onChange={setLifted}
					/>
				</AuthProvider>
			)}
			<AuthContext.Provider value={visibleAuth}>
				{lifted?.scope === scope && (
					<Fragment key={scope}>
						{authConfig && lifted.providerKey === providerKey && (
							<AuthInner hub={authConfig.hub} />
						)}
						{children}
					</Fragment>
				)}
			</AuthContext.Provider>
			<SignInLinkDialog url={signInUrl} onClose={clearSignInUrl} />
		</UserManagerContext.Provider>
	);
}

function AuthInner({ hub }: Readonly<{ hub: string }>) {
	const auth = useAuth();
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const invalidateInfinite = useInvalidateInfiniteInvoke();
	const userManager = useContext(UserManagerContext);

	// auth.events belongs to the AuthProvider's internal UserManager (captured
	// via useState on mount). userManager from context may be a newer instance
	// created after a profile refetch. We must fire userLoaded on the
	// AuthProvider's instance so react-oidc-context picks up the change.
	const authEventsRef = useRef(auth?.events);
	useEffect(() => {
		authEventsRef.current = auth?.events;
	});

	useEffect(() => {
		const onAuthChanged = async () => {
			if (!userManager) return;
			try {
				const user = await userManager.getUser();
				if (user && !user.expired) {
					console.log(
						"[AuthInner] fl-auth-changed: reloading user into context",
					);
					const events = authEventsRef.current;
					if (events) {
						await events.load(user);
					}
				}
			} catch (err) {
				console.warn("[AuthInner] Failed to reload user on auth change:", err);
			}
		};

		window.addEventListener(AUTH_CHANGED_EVENT, onAuthChanged);
		return () => window.removeEventListener(AUTH_CHANGED_EVENT, onAuthChanged);
	}, [userManager]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: The backend gets the auth context when the session changes, not on every new auth context identity.
	useEffect(() => {
		if (!auth) return;

		if (backend instanceof TauriBackend) {
			console.log("Pushing auth context to backend");
			backend.pushAuthContext(auth, hub);
		}

		if (!auth.isAuthenticated) return;

		if (!auth.user?.id_token) {
			console.warn("User is authenticated but no ID token found.");
			return;
		}

		if (!auth.user?.access_token) {
			console.warn("User is authenticated but no access token found.");
		}
	}, [
		auth?.isAuthenticated,
		auth?.isLoading,
		auth?.user?.access_token,
		auth?.user?.id_token,
		backend,
		hub,
	]);

	useEffect(() => {
		if (!userManager || auth.isLoading) return;
		let cancelled = false;
		const recover = async () => {
			try {
				const existing = await userManager.getUser();
				if (!existing?.expired || cancelled) return;
				await renewHubAuth(auth);
			} catch {
				// Renewal preserves outages and starts sign-in only for an explicit provider refusal.
			}
		};
		const onVisible = () => {
			if (document.visibilityState === "visible") void recover();
		};
		void recover();
		window.addEventListener("online", recover);
		document.addEventListener("visibilitychange", onVisible);
		return () => {
			cancelled = true;
			window.removeEventListener("online", recover);
			document.removeEventListener("visibilitychange", onVisible);
		};
	}, [userManager, auth]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: The signed-in subject is a refetch trigger; another account must reload hub-backed queries.
	useEffect(() => {
		if (!(backend instanceof TauriBackend)) return;

		// Hub-backed queries can only be served with an access token; refetching
		// them signed out just replays failures. Local state stays in sync either
		// way. An in-flight refetch that a newer invalidation supersedes rejects
		// with CancelledError, so every invalidation must be settled, not awaited
		// bare.
		const signedIn = Boolean(auth?.isAuthenticated && auth.user?.access_token);

		(async () => {
			if (signedIn) {
				// Ensure user record exists in DB before profile operations
				await Promise.allSettled([invalidate(backend.userState.getInfo, [])]);
			}

			await Promise.allSettled([
				...(signedIn
					? [
							invalidateInfinite(backend.teamState.getInvites, []),
							invalidate(backend.registryState.listMyInvitations, []),
						]
					: []),
				invalidate(backend.userState.getNotifications, []),
				invalidateInfinite(backend.userState.listNotifications, [
					false,
					undefined,
				]),
				invalidate(backend.userState.getProfile, []),
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.userState.getProfiles, []),
				invalidate(backend.appState.getApps, []),
			]);
		})();
	}, [
		backend,
		auth?.isAuthenticated,
		auth?.user?.access_token,
		auth?.user?.profile?.sub,
		invalidate,
		invalidateInfinite,
	]);

	return (
		<ProfileSyncer
			hub={hub}
			auth={{
				isAuthenticated: auth.isAuthenticated,
				accessToken: auth.user?.access_token,
			}}
		/>
	);
}
