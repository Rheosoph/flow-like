"use client";

import { Button } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useState } from "react";
import { storePendingOAuthCallback } from "../../../lib/oauth-callback-storage";

function ThirdpartyCallbackContent() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const [error, setError] = useState<string | null>(null);
	const [processing, setProcessing] = useState(true);
	const [showReturnHint, setShowReturnHint] = useState(false);

	useEffect(() => {
		const handleCallback = async () => {
			try {
				const code = searchParams.get("code");
				const state = searchParams.get("state");
				const errorParam = searchParams.get("error");
				const errorDescription = searchParams.get("error_description");

				// Also check for implicit flow tokens in hash (handled by Next.js differently)
				const accessToken = searchParams.get("access_token");
				const idToken = searchParams.get("id_token");
				const callbackPayload = {
					url: window.location.href,
					code,
					state,
					access_token: accessToken,
					id_token: idToken,
					token_type: searchParams.get("token_type"),
					expires_in: searchParams.get("expires_in"),
					scope: searchParams.get("scope"),
				};

				if (errorParam) {
					setError(`Authorization failed: ${errorDescription || errorParam}`);
					setProcessing(false);
					return;
				}

				if (!state) {
					setError("Missing state parameter in OAuth callback");
					setProcessing(false);
					return;
				}

				// Dispatch a custom event with the OAuth callback data
				// This will be picked up by the OAuth callback handler
				storePendingOAuthCallback(callbackPayload);

				const callbackEvent = new CustomEvent("thirdparty-oauth-callback", {
					detail: callbackPayload,
				});
				window.dispatchEvent(callbackEvent);

				setTimeout(() => {
					setProcessing(false);
					setShowReturnHint(true);
				}, 300);
			} catch (err) {
				setError(
					`Failed to process callback: ${err instanceof Error ? err.message : String(err)}`,
				);
				setProcessing(false);
			}
		};

		handleCallback();
	}, [searchParams, router]);

	if (error) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div className="text-center">
					<div className="mb-4 text-lg text-destructive">
						Authentication Error
					</div>
					<div className="text-sm text-muted-foreground">{error}</div>
					<Button
						type="button"
						onClick={() => router.push("/flow")}
						className="mt-4"
					>
						Return to Flow
					</Button>
				</div>
			</div>
		);
	}

	if (processing) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div className="text-center">
					<div className="mb-4 text-lg">Processing authentication...</div>
					<div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent mx-auto" />
				</div>
			</div>
		);
	}

	if (showReturnHint) {
		return (
			<div className="flex h-screen items-center justify-center">
				<div className="max-w-md text-center">
					<div className="mb-4 text-lg">Connection request received</div>
					<div className="text-sm text-muted-foreground">
						You can return to the previous tab while Flow-Like finishes the
						authorization.
					</div>
					<Button
						type="button"
						onClick={() => router.push("/")}
						className="mt-4"
					>
						Open Flow-Like
					</Button>
				</div>
			</div>
		);
	}

	return null;
}

export default function ThirdpartyCallbackPage() {
	return (
		<Suspense
			fallback={
				<div className="flex h-screen items-center justify-center">
					<div className="text-center">
						<div className="mb-4 text-lg">Loading...</div>
						<div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent mx-auto" />
					</div>
				</div>
			}
		>
			<ThirdpartyCallbackContent />
		</Suspense>
	);
}
