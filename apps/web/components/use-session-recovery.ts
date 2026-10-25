"use client";

import { requiresInteractiveSignIn } from "@flow-like/flow-like-ui/lib/auth/oidc-renewal";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { AuthContextProps } from "react-oidc-context";
import {
	isSessionRejected,
	subscribeSessionRejection,
} from "../lib/auth-session";
import { currentRelativeUrl, saveReturnUrl } from "../lib/return-url";
import { renewWebSession } from "../lib/session-request";

type RecoveryState =
	| "ready"
	| "renewing"
	| "redirecting"
	| "sign-in-required"
	| "retry-required";
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
	const [, setExpiryRevision] = useState(0);
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
	const pendingState =
		auth.user?.expired && accessToken && !sessionRejected
			? "renewing"
			: "redirecting";

	useEffect(() => {
		latestAuth.current = auth;
	}, [auth]);

	useEffect(() => {
		if (recovery?.state !== "retry-required") return;
		const retry = () => {
			attempted.current = undefined;
			setRecovery(undefined);
		};
		const visible = () => {
			if (document.visibilityState === "visible") retry();
		};
		window.addEventListener("online", retry);
		window.addEventListener("flow-like:retry-session", retry);
		document.addEventListener("visibilitychange", visible);
		return () => {
			window.removeEventListener("online", retry);
			window.removeEventListener("flow-like:retry-session", retry);
			document.removeEventListener("visibilitychange", visible);
		};
	}, [recovery?.state]);

	useEffect(() => {
		if (recovery?.state === "renewing" && !auth.user) {
			attempted.current = undefined;
			setRecovery(undefined);
			return;
		}
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
	}, [recovery, accessToken, auth.isAuthenticated, auth.user, sessionInvalid]);

	useEffect(() => {
		const checkExpiredSession = () => {
			const current = latestAuth.current;
			if (current.user?.expired) {
				setExpiryRevision((revision) => revision + 1);
			}
		};
		// Expiry can recover silently without rejecting the current session.
		auth.events.addAccessTokenExpired(checkExpiredSession);
		auth.events.addSilentRenewError(checkExpiredSession);
		return () => {
			auth.events.removeAccessTokenExpired(checkExpiredSession);
			auth.events.removeSilentRenewError(checkExpiredSession);
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
		setRecovery({ accessToken, state: pendingState });
		const returnUrl = currentRelativeUrl();
		if (returnUrl) saveReturnUrl(returnUrl);
		const isCurrentSession = () => {
			const current = latestAuth.current;
			return (
				Boolean(current.user) === Boolean(auth.user) &&
				current.signinRedirect === auth.signinRedirect &&
				current.user?.access_token === accessToken &&
				current.user?.profile.iss === auth.user?.profile.iss &&
				current.user?.profile.sub === auth.user?.profile.sub
			);
		};

		void (async () => {
			try {
				await renewWebSession(auth, accessToken);
				if (isCurrentSession()) setRecovery(undefined);
				return;
			} catch (error) {
				if (!isCurrentSession()) return;
				if (!requiresInteractiveSignIn(error)) {
					setRecovery({ accessToken, state: "retry-required" });
					return;
				}
			}
			setRecovery({ accessToken, state: "redirecting" });
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
	}, [auth, accessToken, sessionInvalid, isPublicPath, recovery, pendingState]);

	if (isPublicPath) return "ready";
	if (recovery?.state === "renewing" && !auth.user) return "ready";
	if (recovery && (!accessToken || recovery.accessToken === accessToken)) {
		if (recovery.state === "renewing" && sessionRejected) return "redirecting";
		// react-oidc-context reports navigator failures through auth.error.
		if (auth.error?.source === "signinRedirect" && !auth.activeNavigator) {
			return "sign-in-required";
		}
		return recovery.state;
	}
	return sessionInvalid ? pendingState : "ready";
}
