"use client";

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { AuthContextProps } from "react-oidc-context";
import {
	isSessionRejected,
	reportUnauthorized,
	subscribeSessionRejection,
} from "../lib/auth-session";
import { currentRelativeUrl, saveReturnUrl } from "../lib/return-url";

type RecoveryState = "ready" | "redirecting" | "sign-in-required";
const serverSnapshot = () => false;

export function useSessionRecovery(
	auth: AuthContextProps,
	isPublicPath: boolean,
): RecoveryState {
	const accessToken = auth.user?.access_token;
	const sessionRejected = useSyncExternalStore(
		subscribeSessionRejection,
		() => isSessionRejected(auth, accessToken),
		serverSnapshot,
	);
	const latestAuth = useRef(auth);
	const attempted = useRef<
		| {
				signinRedirect: AuthContextProps["signinRedirect"];
				accessToken: string | undefined;
		  }
		| undefined
	>(undefined);
	const [recovery, setRecovery] = useState<{
		accessToken: string | undefined;
		state: RecoveryState;
	}>();
	const sessionInvalid =
		(sessionRejected && Boolean(auth.isAuthenticated || accessToken)) ||
		auth.user?.expired === true ||
		(auth.isAuthenticated && !accessToken);

	useEffect(() => {
		latestAuth.current = auth;
	}, [auth]);

	useEffect(() => {
		if (
			recovery &&
			accessToken &&
			recovery.accessToken !== accessToken &&
			auth.isAuthenticated &&
			!sessionInvalid
		) {
			attempted.current = undefined;
			setRecovery(undefined);
		}
	}, [recovery, accessToken, auth.isAuthenticated, sessionInvalid]);

	useEffect(() => {
		const rejectExpiredSession = () => {
			const current = latestAuth.current;
			if (current.user?.expired) {
				reportUnauthorized(current, current.user.access_token);
			}
		};
		// Automatic renewal runs before expiry. If it cannot renew, stop treating
		// the stored user as a working session when the token expires.
		auth.events.addAccessTokenExpired(rejectExpiredSession);
		auth.events.addSilentRenewError(rejectExpiredSession);
		return () => {
			auth.events.removeAccessTokenExpired(rejectExpiredSession);
			auth.events.removeSilentRenewError(rejectExpiredSession);
		};
	}, [auth.events]);

	useEffect(() => {
		if (
			!sessionInvalid ||
			isPublicPath ||
			auth.isLoading ||
			auth.activeNavigator ||
			(recovery && (!accessToken || recovery.accessToken === accessToken))
		) {
			return;
		}
		if (
			attempted.current?.signinRedirect === auth.signinRedirect &&
			attempted.current?.accessToken === accessToken
		) {
			return;
		}
		attempted.current = { signinRedirect: auth.signinRedirect, accessToken };
		setRecovery({ accessToken, state: "redirecting" });
		const returnUrl = currentRelativeUrl();
		if (returnUrl) saveReturnUrl(returnUrl);

		void (async () => {
			try {
				await auth.removeUser();
			} catch {
				console.warn("[Auth] Could not clear the rejected session");
			}
			const currentToken = latestAuth.current.user?.access_token;
			if (currentToken && currentToken !== accessToken) return;
			try {
				await auth.signinRedirect({ url_state: returnUrl });
			} catch {
				const currentToken = latestAuth.current.user?.access_token;
				if (currentToken && currentToken !== accessToken) return;
				setRecovery({ accessToken, state: "sign-in-required" });
			}
		})();
	}, [auth, accessToken, sessionInvalid, isPublicPath, recovery]);

	if (isPublicPath) return "ready";
	if (recovery && (!accessToken || recovery.accessToken === accessToken)) {
		// react-oidc-context reports navigator failures through auth.error.
		if (auth.error?.source === "signinRedirect" && !auth.activeNavigator) {
			return "sign-in-required";
		}
		return recovery.state;
	}
	return sessionInvalid ? "redirecting" : "ready";
}
