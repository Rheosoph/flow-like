"use client";

import { LoadingScreen, useBackend } from "@flow-like/flow-like-ui";
import type { IProfile } from "@flow-like/flow-like-ui";
import { Amplify } from "aws-amplify";
import {
	type AuthTokens,
	type TokenProvider,
	decodeJWT,
} from "aws-amplify/auth";
import { usePathname } from "next/navigation";
import {
	UserManager,
	type UserManagerSettings,
	WebStorageStateStore,
} from "oidc-client-ts";
import { useEffect, useState } from "react";
import { AuthProvider, useAuth } from "react-oidc-context";
import { get } from "../lib/api";
import { currentRelativeUrl, saveReturnUrl } from "../lib/return-url";
import { SignInRequired } from "./sign-in-required";
import { WebBackend } from "./web-provider";

const PUBLIC_PATHS = [
	"/callback",
	"/thirdparty/callback",
	"/store",
	"/store/explore",
];

const DEFAULT_PROFILE: IProfile = {
	name: "default",
	bits: [],
	created: new Date().toISOString(),
	updated: new Date().toISOString(),
	hub: process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com",
};

export class OIDCTokenProvider implements TokenProvider {
	constructor(private readonly userManager: UserManager) {}
	async getTokens(options?: {
		forceRefresh?: boolean;
	}): Promise<AuthTokens | null> {
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

export function WebAuthProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const [openIdAuthConfig, setOpenIdAuthConfig] =
		useState<UserManagerSettings>();
	const [userManager, setUserManager] = useState<UserManager>();
	const [loadingProgress, setLoadingProgress] = useState(10);

	useEffect(() => {
		(async () => {
			setLoadingProgress(30);
			const response = await get<any>(DEFAULT_PROFILE, "auth/openid");
			if (response) {
				setLoadingProgress(60);
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
				const userManagerInstance = new UserManager(response);
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
				setLoadingProgress(90);
				setUserManager(userManagerInstance);
				setOpenIdAuthConfig(response);
			}
		})();
	}, []);

	if (!openIdAuthConfig) {
		return <LoadingScreen progress={loadingProgress} />;
	}

	return (
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
	);
}

const AUTH_CHANNEL = "flow-like-auth";

function AuthInner({ children }: Readonly<{ children: React.ReactNode }>) {
	const auth = useAuth();
	const backend = useBackend();
	const pathname = usePathname();
	const [profileLoaded, setProfileLoaded] = useState(false);
	const [authPushed, setAuthPushed] = useState(false);

	const isPublicPath =
		pathname === "/debug/markdown" ||
		pathname === "/debug/upgrade" ||
		PUBLIC_PATHS.some((path) => pathname?.startsWith(path));

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
			console.warn("User is authenticated but no ID token found.");
			return;
		}

		setAuthPushed(true);
	}, [
		auth?.isAuthenticated,
		auth?.isLoading,
		auth?.user?.id_token,
		auth?.activeNavigator,
		backend,
	]);

	// Ensure user exists in DB, then fetch and push profile
	useEffect(() => {
		if (
			!authPushed ||
			!auth?.isAuthenticated ||
			!auth?.user?.access_token ||
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
				const profile = await backend.userState.getProfile();
				if (profile && backend instanceof WebBackend) {
					backend.pushProfile(profile);
					setProfileLoaded(true);
				}
			} catch (error) {
				console.error("Failed to fetch profile:", error);
				if (backend instanceof WebBackend) {
					backend.pushProfile(DEFAULT_PROFILE);
					setProfileLoaded(true);
				}
			}
		})();

		return () => {
			cancelled = true;
		};
	}, [authPushed, auth?.isAuthenticated, auth?.user?.access_token, backend]);

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
							await auth?.signinRedirect({ url_state: currentRelativeUrl() });
						}
					} catch {
						console.warn("Silent login failed, attempting normal login");

						try {
							await auth?.signinRedirect({ url_state: currentRelativeUrl() });
						} catch {
							console.error("Both silent and redirect login failed");
						}
					}
				}
			} catch {
				console.error("Login process failed");
			}
		})();
	}, [auth.user?.profile?.sub]);

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

	// Show loading state while auth is initializing
	if (auth.isLoading && !isPublicPath) {
		return <LoadingScreen progress={95} />;
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

	return <>{children}</>;
}
