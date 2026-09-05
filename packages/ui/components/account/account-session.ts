interface SessionUser {
	access_token: string;
	id_token?: string;
	expired?: boolean;
}

interface SessionAuth {
	isAuthenticated: boolean;
	user?: SessionUser | null;
	signinSilent(): Promise<SessionUser | null>;
}

interface DecodedToken {
	payload: { exp?: number; [key: string]: unknown };
	toString(): string;
}

/** Resolve the current OIDC session for every Amplify request, including renewal. */
export function createAccountTokenProvider<T extends DecodedToken>(
	getAuth: () => SessionAuth | Promise<SessionAuth>,
	decode: (token: string) => T,
) {
	let renewing: Promise<SessionUser> | undefined;
	return {
		async getTokens(options?: { forceRefresh?: boolean }) {
			const auth = await getAuth();
			let user = auth.user;
			if (!auth.isAuthenticated || !user?.access_token || !user.id_token) {
				return null;
			}
			const expires = decode(user.access_token).payload.exp;
			if (
				options?.forceRefresh ||
				user.expired ||
				(expires !== undefined && expires <= Date.now() / 1000 + 30)
			) {
				renewing ??= auth
					.signinSilent()
					.then((renewed) => {
						if (!renewed?.access_token || !renewed.id_token) {
							const error = new Error("Sign in again to update your account.");
							error.name = "AccountSessionExpired";
							throw error;
						}
						return renewed;
					})
					.finally(() => {
						renewing = undefined;
					});
				user = await renewing;
			}
			if (!user.id_token) return null;
			return {
				accessToken: decode(user.access_token),
				idToken: decode(user.id_token),
			};
		},
	};
}
