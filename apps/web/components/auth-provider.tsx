"use client";

import { LoadingScreen, useBackend } from "@flow-like/flow-like-ui";
import type { IProfile } from "@flow-like/flow-like-ui";
import { createAccountTokenProvider } from "@flow-like/flow-like-ui/components/account/account-session";
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { coordinateOidcRenewal } from "@flow-like/flow-like-ui/lib/auth/oidc-renewal";
import { useQueryClient } from "@tanstack/react-query";
import { Amplify } from "aws-amplify";
import {
	type AuthTokens,
	type TokenProvider,
	decodeJWT,
} from "aws-amplify/auth";
import { usePathname } from "next/navigation";
import { UserManager, type UserManagerSettings } from "oidc-client-ts";
import { createContext, useContext, useEffect, useState } from "react";
import { AuthProvider, useAuth } from "react-oidc-context";
import { get } from "../lib/api";
import { getWebOidcSettings } from "../lib/oidc-settings";
import { getPublicApiUrl, getPublicWebConfig } from "../lib/public-config";
import { currentRelativeUrl, saveReturnUrl } from "../lib/return-url";
import { SignInRequired } from "./sign-in-required";
import { useSessionRecovery } from "./use-session-recovery";
import { WebBackend } from "./web-provider";

const PUBLIC_PATHS = [
	"/payments/return",
	"/callback",
	"/thirdparty/callback",
	"/store",
	"/store/explore",
];

const OAuthCallbackReadyContext = createContext(false);

export function useOAuthCallbackReady(): boolean {
	return useContext(OAuthCallbackReadyContext);
}

function defaultProfile(): IProfile {
	return {
		name: "default",
		bits: [],
		created: new Date().toISOString(),
		updated: new Date().toISOString(),
		hub: getPublicApiUrl(),
	};
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

export function WebAuthProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const pathname = usePathname();
	const [openIdAuthConfig, setOpenIdAuthConfig] =
		useState<UserManagerSettings>();
	const [userManager, setUserManager] = useState<UserManager>();
	const [loadingProgress, setLoadingProgress] = useState(10);
	const [configurationError, setConfigurationError] = useState(false);

	useEffect(() => {
		let cancelled = false;
		let manager: UserManager | undefined;
		(async () => {
			try {
				getPublicWebConfig();
			} catch {
				setConfigurationError(true);
				return;
			}
			setLoadingProgress(30);
			const response = await get<any>(defaultProfile(), "auth/openid");
			if (cancelled) return;
			if (response) {
				setLoadingProgress(60);
				const settings = getWebOidcSettings(response);
				const userManagerInstance = new UserManager(settings);
				manager = userManagerInstance;
				coordinateOidcRenewal(userManagerInstance);
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
				setLoadingProgress(90);
				setUserManager(userManagerInstance);
				setOpenIdAuthConfig(settings);
			}
		})();
		return () => {
			cancelled = true;
			manager?.stopSilentRenew();
		};
	}, []);

	if (configurationError) {
		return (
			<p role="alert">
				Web configuration is unavailable. Contact your administrator.
			</p>
		);
	}

	if (!openIdAuthConfig) {
		return <LoadingScreen progress={loadingProgress} />;
	}

	// Third-party state belongs to the workflow connection, not Hub sign-in.
	const skipSigninCallback =
		pathname === "/thirdparty/callback" || pathname === "/thirdparty/callback/";

	return (
		<AuthProvider
			key={openIdAuthConfig.client_id}
			{...openIdAuthConfig}
			userManager={userManager}
			skipSigninCallback={skipSigninCallback}
		>
			<AuthInner>{children}</AuthInner>
		</AuthProvider>
	);
}

const AUTH_CHANNEL = "flow-like-auth";

function AuthInner({ children }: Readonly<{ children: React.ReactNode }>) {
	const auth = useAuth();
	const backend = useBackend();
	const queryClient = useQueryClient();
	const pathname = usePathname();
	const account = JSON.stringify([
		auth.user?.profile.iss,
		auth.user?.profile.sub,
	]);
	const [loadedProfile, setLoadedProfile] = useState<{
		backend: typeof backend;
		account: string;
	} | null>(null);
	const profileLoaded =
		loadedProfile?.backend === backend && loadedProfile.account === account;
	const [authPushed, setAuthPushed] = useState(false);

	const isPublicPath =
		pathname === "/debug/markdown" ||
		pathname === "/debug/upgrade" ||
		PUBLIC_PATHS.some((path) => pathname?.startsWith(path));
	const sessionRecovery = useSessionRecovery(auth, isPublicPath);

	useEffect(() => {
		if (
			!auth.isAuthenticated ||
			(sessionRecovery !== "ready" && sessionRecovery !== "renewing")
		) {
			setLoadedProfile(null);
		}
	}, [auth.isAuthenticated, sessionRecovery]);

	// Listen for auth changes from other tabs
	useEffect(() => {
		let channel: BroadcastChannel | null = null;

		const handleAuthMessage = async (event: MessageEvent) => {
			if (event.data?.type === "AUTH_SUCCESS" && !auth.isAuthenticated) {
				try {
					await auth.signinSilent();
				} catch {
					window.location.reload();
				}
			}
		};

		// Fallback for browsers without BroadcastChannel
		const handleStorageChange = async (event: StorageEvent) => {
			if (event.key?.includes("oidc.") && !auth.isAuthenticated) {
				try {
					await auth.signinSilent();
				} catch {
					window.location.reload();
				}
			}
		};

		try {
			channel = new BroadcastChannel(AUTH_CHANNEL);
			channel.addEventListener("message", handleAuthMessage);
		} catch {
			// BroadcastChannel not supported, use storage events as fallback
			window.addEventListener("storage", handleStorageChange);
		}

		return () => {
			if (channel) {
				channel.removeEventListener("message", handleAuthMessage);
				channel.close();
			} else {
				window.removeEventListener("storage", handleStorageChange);
			}
		};
	}, [auth]);

	// Push auth context to backend (always, so unsigned users can trigger signinRedirect)
	useEffect(() => {
		if (!auth) return;

		if (backend instanceof WebBackend) {
			backend.pushAuthContext(auth);
		}

		if (!auth.isAuthenticated) {
			setAuthPushed(false);
			return;
		}

		if (!auth.user?.id_token) {
			setAuthPushed(false);
			console.warn("User is authenticated but no ID token found.");
			return;
		}

		setAuthPushed(true);
	}, [auth, backend]);

	// Ensure user exists in DB, then fetch and push profile
	useEffect(() => {
		if (
			profileLoaded ||
			!authPushed ||
			!auth?.isAuthenticated ||
			!auth?.user?.access_token ||
			sessionRecovery !== "ready" ||
			!backend
		) {
			return;
		}

		let cancelled = false;

		(async () => {
			const MAX_RETRIES = 5;
			const BASE_DELAY = 500;

			for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
				if (cancelled) return;

				try {
					await backend.userState.getInfo();
					break;
				} catch (error) {
					if (error instanceof ApiResponseError && error.status === 401) return;
					if (attempt === MAX_RETRIES) {
						console.error("Failed to ensure user exists after retries:", error);
					} else {
						const delay = BASE_DELAY * 2 ** attempt;
						await new Promise((r) => setTimeout(r, delay));
					}
				}
			}

			if (cancelled) return;

			try {
				const settingsProfile = await backend.userState.getSettingsProfile();
				if (cancelled) return;
				if (settingsProfile && backend instanceof WebBackend) {
					const profileKey = [backend.userState.getProfile.name || "backendFn"];
					const settingsKey = [
						backend.userState.getSettingsProfile.name || "backendFn",
					];
					const appsKey = [backend.appState.getApps.name || "backendFn"];
					const groupsKey = [backend.appState.getMyGroups.name || "backendFn"];
					// These lists belong to the signed-in session. Keep retries from
					// restoring another session's lists when query observers mount.
					for (const queryKey of [appsKey, groupsKey]) {
						queryClient.setQueryDefaults(queryKey, {
							...queryClient.getQueryDefaults(queryKey),
							persister: undefined,
						});
					}
					// Public callback pages can start these reads before auth is ready.
					// Cancel them so a late signed-out response cannot replace this session.
					await Promise.all(
						[profileKey, settingsKey, appsKey, groupsKey].map((queryKey) =>
							queryClient.cancelQueries({ queryKey }),
						),
					);
					if (cancelled) return;
					backend.pushProfile(settingsProfile.hub_profile);
					queryClient.setQueryData(profileKey, settingsProfile.hub_profile);
					queryClient.setQueryData(settingsKey, settingsProfile);
					// Bypass lazy persistence as well as fresh in-memory data for login.
					// Start before mounting the library; other pages need not wait for apps.
					void Promise.allSettled([
						queryClient.fetchQuery({
							queryKey: appsKey,
							queryFn: () => backend.appState.getApps(),
							staleTime: 0,
							persister: undefined,
						}),
						queryClient.fetchQuery({
							queryKey: groupsKey,
							queryFn: () => backend.appState.getMyGroups(),
							staleTime: 0,
							persister: undefined,
						}),
					]);
					setLoadedProfile({ backend, account });
				}
			} catch (error) {
				if (cancelled) return;
				if (error instanceof ApiResponseError && error.status === 401) return;
				console.error("Failed to fetch profile:", error);
				if (backend instanceof WebBackend) {
					backend.pushProfile(defaultProfile());
					setLoadedProfile({ backend, account });
				}
			}
		})();

		return () => {
			cancelled = true;
		};
	}, [
		account,
		profileLoaded,
		authPushed,
		auth?.isAuthenticated,
		auth?.user?.access_token,
		backend,
		queryClient,
		sessionRecovery,
	]);

	// Preserve the pre-login location (e.g. /join?appId=…&token=…) so the
	// callback can restore it. url_state on signinRedirect is the primary
	// carrier; this stored copy covers logins started outside SignInRequired.
	useEffect(() => {
		if (auth.isLoading || auth.isAuthenticated || isPublicPath) return;
		if (auth.activeNavigator) return;
		const returnUrl = currentRelativeUrl();
		if (returnUrl) saveReturnUrl(returnUrl);
	}, [
		auth.isLoading,
		auth.isAuthenticated,
		auth.activeNavigator,
		isPublicPath,
	]);

	// Silent renewal also sets isLoading. Keep the current account's workspace
	// mounted so token rotation cannot discard editor state or rerun page onLoad.
	const renewingLoadedSession =
		auth.isAuthenticated &&
		profileLoaded &&
		(!auth.activeNavigator || auth.activeNavigator === "signinSilent") &&
		(sessionRecovery === "renewing" ||
			(auth.activeNavigator === "signinSilent" && sessionRecovery === "ready"));
	if (auth.isLoading && !isPublicPath && !renewingLoadedSession) {
		return <LoadingScreen progress={95} />;
	}

	if (sessionRecovery === "sign-in-required") return <SignInRequired />;
	if (sessionRecovery === "retry-required") {
		return (
			<div
				role="alert"
				className="flex min-h-screen flex-col items-center justify-center gap-4 p-6 text-center"
			>
				<p>
					Your session could not be renewed. Check your connection and try
					again.
				</p>
				<button
					type="button"
					className="rounded-md border px-4 py-2"
					onClick={() =>
						window.dispatchEvent(new Event("flow-like:retry-session"))
					}
				>
					Try again
				</button>
			</div>
		);
	}
	if (sessionRecovery === "redirecting") return <LoadingScreen progress={98} />;
	if (sessionRecovery === "renewing" && !renewingLoadedSession) {
		return <LoadingScreen progress={98} />;
	}

	// Show loading state while redirecting to sign-in
	if (!auth.isAuthenticated && auth.activeNavigator && !isPublicPath) {
		return <LoadingScreen progress={98} />;
	}

	// Show sign-in required screen when not authenticated (skip for public paths)
	if (!auth.isAuthenticated && !isPublicPath) {
		return <SignInRequired />;
	}

	// Show loading while profile is being fetched (skip for public paths)
	if (!profileLoaded && !isPublicPath) {
		return <LoadingScreen progress={99} />;
	}

	const oauthCallbackReady =
		!auth.isLoading &&
		(!auth.isAuthenticated ||
			(authPushed && profileLoaded && sessionRecovery === "ready"));

	return (
		<OAuthCallbackReadyContext.Provider value={oauthCallbackReady}>
			{children}
		</OAuthCallbackReadyContext.Provider>
	);
}
