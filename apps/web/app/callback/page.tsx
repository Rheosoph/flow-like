"use client";

import { isUsePathname } from "@flow-like/flow-like-ui/lib/use-route-url";
import { useTranslation } from "@flow-like/locales";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { type AuthContextProps, useAuth } from "react-oidc-context";
import { hostedReturnPath } from "../../lib/hosted-route";
import {
	consumeReturnUrl,
	sanitizeReturnUrl,
	saveReturnUrl,
} from "../../lib/return-url";

const AUTH_CHANNEL = "flow-like-auth";
const CALLBACK_TIMEOUT_MS = 8000;
const RECOVERY_KEY = "flow-like-signin-callback-recovery";

function isStateError(error: AuthContextProps["error"]): boolean {
	return (
		error?.source === "signinCallback" &&
		[
			"No matching state found in storage",
			"State does not match",
			"No state in response",
		].includes(error.message)
	);
}

function clearRecoveryAttempt(): void {
	try {
		sessionStorage.removeItem(RECOVERY_KEY);
	} catch {}
}

function clearCallbackParams(): void {
	window.history.replaceState(
		window.history.state,
		"",
		window.location.pathname,
	);
}

export default function CallbackPage() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const router = useRouter();
	const [stuck, setStuck] = useState(false);
	const [retrying, setRetrying] = useState(false);
	const redirectedRef = useRef(false);
	const retryInFlight = useRef(false);
	const autoAttempted = useRef(false);

	const restartSignIn = useCallback(async () => {
		if (retryInFlight.current) return;
		retryInFlight.current = true;
		setRetrying(true);
		setStuck(false);
		try {
			// A rejected callback's state is untrusted. Use only the saved local target.
			const returnUrl = consumeReturnUrl();
			if (returnUrl) saveReturnUrl(returnUrl);
			clearCallbackParams();
			// Start a fresh state/PKCE exchange; never reuse the rejected code.
			await auth.signinRedirect({
				url_state: returnUrl ?? undefined,
				redirectMethod: "replace",
			});
		} catch {
			// The retry controls below also cover thrown navigator failures.
		} finally {
			// react-oidc-context can resolve a failed redirect with null.
			retryInFlight.current = false;
			setRetrying(false);
			setStuck(true);
		}
	}, [auth.signinRedirect]);

	useEffect(() => {
		if (
			auth.isLoading ||
			auth.isAuthenticated ||
			auth.activeNavigator ||
			!isStateError(auth.error) ||
			autoAttempted.current
		) {
			return;
		}
		const params = new URLSearchParams(window.location.search);
		if (!params.get("code") || params.has("error")) return;
		autoAttempted.current = true;
		try {
			// Persist before leaving the page so a second failed callback cannot loop.
			if (sessionStorage.getItem(RECOVERY_KEY)) return;
			sessionStorage.setItem(RECOVERY_KEY, "1");
		} catch {
			// Without a persistent guard, recovery must be initiated by the user.
			return;
		}
		void restartSignIn();
	}, [
		auth.isLoading,
		auth.isAuthenticated,
		auth.activeNavigator,
		auth.error,
		restartSignIn,
	]);

	// Redirect on successful authentication
	useEffect(() => {
		if (
			auth.isAuthenticated &&
			!auth.isLoading &&
			!auth.activeNavigator &&
			!auth.error &&
			!redirectedRef.current
		) {
			redirectedRef.current = true;
			clearRecoveryAttempt();
			clearCallbackParams();

			try {
				const channel = new BroadcastChannel(AUTH_CHANNEL);
				channel.postMessage({ type: "AUTH_SUCCESS" });
				channel.close();
			} catch {
				// BroadcastChannel not supported
			}

			// url_state survived the OAuth round trip in the state parameter;
			// the stored copy is the fallback for logins that didn't carry it.
			// Consume unconditionally so no stale key outlives this login.
			const stored = consumeReturnUrl();
			const returnUrl = sanitizeReturnUrl(auth.user?.url_state) ?? stored;
			const hostedPath = hostedReturnPath(returnUrl);
			if (hostedPath) {
				// The static host serves every hosted app route from the exported /a page.
				window.location.replace(hostedPath);
				return;
			}
			if (returnUrl) {
				const destination = new URL(returnUrl, window.location.origin);
				if (isUsePathname(destination.pathname)) {
					window.location.replace(returnUrl);
					return;
				}
			}
			router.replace(returnUrl || "/");
		}
	}, [
		auth.isAuthenticated,
		auth.isLoading,
		auth.activeNavigator,
		auth.error,
		auth.user?.url_state,
		router,
	]);

	// Detect stuck state: not loading, not authenticated, no error
	useEffect(() => {
		if (
			auth.isLoading ||
			auth.isAuthenticated ||
			auth.activeNavigator ||
			auth.error ||
			retrying
		)
			return;

		const timeout = setTimeout(() => {
			setStuck(true);
		}, CALLBACK_TIMEOUT_MS);

		return () => clearTimeout(timeout);
	}, [
		auth.isLoading,
		auth.isAuthenticated,
		auth.activeNavigator,
		auth.error,
		retrying,
	]);

	if (auth.isLoading || auth.activeNavigator || retrying) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div className="text-center">
					<div className="mb-4 text-lg">
						{t("signingYouIn", "Signing you in...")}
					</div>
					<div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent mx-auto" />
				</div>
			</div>
		);
	}

	if (auth.error || stuck) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div role="alert" className="text-center space-y-4">
					<div className="mb-4 text-lg text-red-500">
						{t("unableToSignIn", "Unable to sign in")}
					</div>
					<div className="text-sm text-muted-foreground">
						{isStateError(auth.error) || (stuck && !auth.error)
							? t(
									"loginExpiredPleaseTryAgain",
									"Login expired, please try again.",
								)
							: t(
									"signInCouldNotComplete",
									"We couldn't complete your sign-in. Please try again.",
								)}
					</div>
					<div className="flex gap-3 justify-center">
						<button
							type="button"
							onClick={() => void restartSignIn()}
							className="px-4 py-2 bg-primary text-primary-foreground rounded"
						>
							{t("tryAgain", "Try Again")}
						</button>
						<button
							type="button"
							onClick={() => {
								clearRecoveryAttempt();
								router.replace("/");
							}}
							className="px-4 py-2 bg-secondary text-secondary-foreground rounded"
						>
							{t("returnHome", "Return Home")}
						</button>
					</div>
				</div>
			</div>
		);
	}

	return (
		<div className="flex h-screen items-center justify-center">
			<div className="text-center">
				<div className="mb-4 text-lg">
					{t("processingAuthentication", "Processing authentication...")}
				</div>
				<div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent mx-auto" />
			</div>
		</div>
	);
}
