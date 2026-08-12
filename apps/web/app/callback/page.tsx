"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { consumeReturnUrl, sanitizeReturnUrl } from "../../lib/return-url";

const AUTH_CHANNEL = "flow-like-auth";
const CALLBACK_TIMEOUT_MS = 8000;

export default function CallbackPage() {
	const auth = useAuth();
	const router = useRouter();
	const [stuck, setStuck] = useState(false);
	const redirectedRef = useRef(false);

	// Redirect on successful authentication
	useEffect(() => {
		if (auth.isAuthenticated && !redirectedRef.current) {
			redirectedRef.current = true;

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
			router.push(returnUrl || "/");
		}
	}, [auth.isAuthenticated, auth.user?.url_state, router]);

	// Detect stuck state: not loading, not authenticated, no error
	useEffect(() => {
		if (auth.isLoading || auth.isAuthenticated || auth.error) return;

		const timeout = setTimeout(() => {
			setStuck(true);
		}, CALLBACK_TIMEOUT_MS);

		return () => clearTimeout(timeout);
	}, [auth.isLoading, auth.isAuthenticated, auth.error]);

	if (auth.isLoading) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div className="text-center">
					<div className="mb-4 text-lg">Signing you in...</div>
					<div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent mx-auto" />
				</div>
			</div>
		);
	}

	if (auth.error || stuck) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div className="text-center space-y-4">
					<div className="mb-4 text-lg text-red-500">
						{auth.error ? "Authentication Error" : "Authentication timed out"}
					</div>
					<div className="text-sm text-muted-foreground">
						{auth.error?.message ||
							"The sign-in process did not complete. This can happen on mobile browsers."}
					</div>
					<div className="flex gap-3 justify-center">
						<button
							type="button"
							onClick={() => auth.signinRedirect()}
							className="px-4 py-2 bg-primary text-primary-foreground rounded"
						>
							Try Again
						</button>
						<button
							type="button"
							onClick={() => router.push("/")}
							className="px-4 py-2 bg-secondary text-secondary-foreground rounded"
						>
							Return Home
						</button>
					</div>
				</div>
			</div>
		);
	}

	return (
		<div className="flex h-screen items-center justify-center">
			<div className="text-center">
				<div className="mb-4 text-lg">Processing authentication...</div>
				<div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent mx-auto" />
			</div>
		</div>
	);
}
