"use client";
import {
	useBackend,
	useInvalidateInfiniteInvoke,
	useInvalidateInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import type { IProfile } from "@flow-like/flow-like-ui";
import { listen } from "@tauri-apps/api/event";
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
	UserManager,
	type UserManagerSettings,
	WebStorageStateStore,
} from "oidc-client-ts";
import { createContext, useContext, useEffect, useRef, useState } from "react";
import { AuthProvider, useAuth } from "react-oidc-context";
import { get } from "../lib/api";
import { ProfileSyncer, TauriBackend } from "./tauri-provider";

const AUTH_CHANGED_EVENT = "fl-auth-changed";

function emitAuthChanged() {
	window.dispatchEvent(new CustomEvent(AUTH_CHANGED_EVENT));
}

const UserManagerContext = createContext<UserManager | null>(null);

export class OIDCTokenProvider implements TokenProvider {
	constructor(private readonly userManager: UserManager) {}
	async getTokens(options?: {
		forceRefresh?: boolean;
	}): Promise<AuthTokens | null> {
		console.warn("Getting tokens from OIDCTokenProvider...");
		const user = await this.userManager.getUser();
		if (!user?.access_token || !user?.id_token) {
			return null;
		}

		const accessToken = decodeJWT(user.access_token);
		const idToken = decodeJWT(user.id_token);

		return {
			accessToken: accessToken,
			idToken: idToken,
		};
	}
}

class TauriWindow implements IWindow {
	private abort: ((reason: Error) => void) | undefined;
	close() {
		return;
	}
	async navigate(params: NavigateParams): Promise<never> {
		openUrl(params.url);

		const promise = new Promise((resolve, reject) => {
			this.abort = reject;
		});

		return promise as Promise<never>;
	}
}

class TauriRedirectNavigator implements INavigator {
	async prepare(params: unknown): Promise<IWindow> {
		return new TauriWindow();
	}

	async callback(url: string, params?: unknown): Promise<void> {
		return;
	}
}

export function DesktopAuthProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [openIdAuthConfig, setOpenIdAuthConfig] =
		useState<UserManagerSettings>();
	const [userManager, setUserManager] = useState<UserManager>();
	const backend = useBackend();
	const currentProfile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const hubUrl = currentProfile.data?.hub ?? "api.flow-like.com";
	const hubSecure = currentProfile.data?.secure ?? true;

	useEffect(() => {
		const effectiveProfile = {
			hub: hubUrl,
			secure: hubSecure,
			bits: [],
			created: new Date().toISOString(),
			updated: new Date().toISOString(),
			name: "default",
		} as IProfile;

		(async () => {
			try {
				const response = await get<any>(effectiveProfile, "auth/openid");
				if (response) {
					if (process.env.NEXT_PUBLIC_REDIRECT_URL)
						response.redirect_uri = process.env.NEXT_PUBLIC_REDIRECT_URL;
					if (process.env.NEXT_PUBLIC_REDIRECT_LOGOUT_URL)
						response.post_logout_redirect_uri =
							process.env.NEXT_PUBLIC_REDIRECT_LOGOUT_URL;
					const store = new WebStorageStateStore({
						store: localStorage,
					});
					response.userStore = store;
					response.automaticSilentRenew = true;
					const navigator = new TauriRedirectNavigator();
					const userManagerInstance = new UserManager(response, navigator);
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
					setUserManager(userManagerInstance);
					setOpenIdAuthConfig(response);
				} else {
					console.warn("OpenID response was falsy, not configuring auth");
				}
			} catch (error) {
				console.error("Failed to fetch OpenID config:", error);
			}
		})();
	}, [hubUrl, hubSecure]);

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

				console.log("[OIDC] Processing callback URL:", {
					rawUrl,
					signinUrl,
					logoutUrl,
				});

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
			} catch (error) {
				seenUrls.delete(rawUrl);
				console.error("Failed to process OIDC callback URL:", rawUrl, error);
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
			console.log("Debug OIDC URL:", url);
			await handleIncomingOidcUrl(url);
		}

		window.addEventListener("debug-oidc", debugListener);

		const unlisten = listen<{ url: string }>("oidc/url", async (event) => {
			await handleIncomingOidcUrl(event.payload.url);
		});

		void processStartupDeepLinks();

		return () => {
			unlisten.then((unsub) => unsub());
			window.removeEventListener("debug-oidc", debugListener);
		};
	}, [userManager, openIdAuthConfig]);

	if (!openIdAuthConfig)
		return <AuthProvider key="loading-auth-config">{children}</AuthProvider>;

	return (
		<UserManagerContext.Provider value={userManager ?? null}>
			<AuthProvider
				key={openIdAuthConfig.client_id}
				{...openIdAuthConfig}
				automaticSilentRenew={true}
				userStore={
					new WebStorageStateStore({
						store: localStorage,
					})
				}
			>
				<AuthInner>{children}</AuthInner>
			</AuthProvider>
		</UserManagerContext.Provider>
	);
}

function AuthInner({ children }: Readonly<{ children: React.ReactNode }>) {
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
	useEffect(() => {
		if (!auth) return;

		if (backend instanceof TauriBackend) {
			console.log("Pushing auth context to backend:", auth);
			backend.pushAuthContext(auth);
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
	]);

	useEffect(() => {
		if (!auth) return;

		(async () => {
			try {
				const existingUser = auth.user;

				if (existingUser && !existingUser.expired) {
					return;
				}

				if (existingUser?.expired) {
					try {
						const user = await auth?.signinSilent();
						if (!user) {
							console.warn(
								"Silent login returned no user, attempting redirect login.",
							);
							await auth?.signinRedirect();
						}
					} catch (silentError) {
						console.warn(
							"Silent login failed, attempting normal login:",
							silentError,
						);

						try {
							await auth?.signinRedirect();
						} catch (redirectError) {
							console.error(
								"Both silent and redirect login failed:",
								redirectError,
							);
						}
					}
				}
			} catch (error) {
				console.error("Login process failed:", error);
			}
		})();
	}, [auth.user?.profile?.sub]);

	useEffect(() => {
		if (!(backend instanceof TauriBackend)) return;

		(async () => {
			// Ensure user record exists in DB before profile operations
			await invalidate(backend.userState.getInfo, []);

			void Promise.allSettled([
				invalidate(backend.userState.getNotifications, []),
				invalidateInfinite(backend.userState.listNotifications, [false]),
				invalidateInfinite(backend.teamState.getInvites, []),
				invalidate(backend.userState.getProfile, []),
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.userState.getProfiles, []),
				invalidate(backend.appState.getApps, []),
			]);
		})();
	}, [
		backend,
		auth?.isAuthenticated,
		auth?.user?.profile?.sub,
		invalidate,
		invalidateInfinite,
	]);

	return (
		<>
			<ProfileSyncer
				auth={{
					isAuthenticated: auth.isAuthenticated,
					accessToken: auth.user?.access_token,
				}}
			/>
			{children}
		</>
	);
}
