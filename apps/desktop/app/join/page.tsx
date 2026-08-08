"use client";

import {
	Button,
	LoadingScreen,
	addAppToProfile,
	useBackend,
} from "@flow-like/flow-like-ui";
import {
	attemptJoinWithRetry,
	joinFailureMessage,
} from "@flow-like/flow-like-ui/lib/join-invite";
import { LogIn } from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { clearPendingInvite, setPendingInvite } from "../../lib/pending-invite";

export default function JoinPage() {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("appId");
	const token = searchParams.get("token");
	const hasAttempted = useRef(false);
	const [attempt, setAttempt] = useState(0);
	const [isRedirecting, setIsRedirecting] = useState(false);

	// Persist immediately: sign-in leaves the app and onboarding may navigate
	// away — PendingInviteRedeemer brings the user back here afterwards.
	useEffect(() => {
		if (appId && token) setPendingInvite(appId, token);
	}, [appId, token]);

	const joinApp = useCallback(async () => {
		if (!appId || !token) {
			toast.error("Invalid invite link — missing app ID or token.");
			router.push("/");
			return;
		}

		const result = await attemptJoinWithRetry(async () => {
			await backend.teamState.joinInviteLink(appId, token);
			await addAppToProfile(backend, appId);
		}, setAttempt);

		clearPendingInvite();
		// Scrub the token from the address bar and history before leaving.
		window.history.replaceState(null, "", "/");

		if (result.ok) {
			toast.success("Successfully joined the app!");
			router.push(`/use?id=${appId}`);
			return;
		}

		console.error("Failed to join:", result.error);
		toast.error(joinFailureMessage(result.kind ?? "retry-exhausted"));
		router.push("/");
	}, [backend, appId, token, router]);

	useEffect(() => {
		if (hasAttempted.current) return;
		if (!appId || !token) return;
		if (auth.isLoading || !auth.isAuthenticated) return;

		hasAttempted.current = true;
		joinApp();
	}, [auth.isAuthenticated, auth.isLoading, appId, token, joinApp]);

	const handleSignIn = useCallback(async () => {
		setIsRedirecting(true);
		try {
			await auth.signinRedirect();
		} catch (error) {
			console.error("Sign-in redirect failed:", error);
			setIsRedirecting(false);
		}
	}, [auth]);

	if (!auth.isLoading && !auth.isAuthenticated && appId && token) {
		return (
			<main className="flex h-full w-full flex-1 items-center justify-center p-6">
				<div className="w-full max-w-md space-y-4 rounded-xl border bg-card p-8 text-center shadow-floating">
					<h2 className="text-xl font-semibold">You&apos;ve been invited</h2>
					<p className="text-sm text-muted-foreground">
						Sign in to accept this invite. You&apos;ll be brought right back
						here afterwards.
					</p>
					<Button
						className="w-full"
						onClick={handleSignIn}
						disabled={isRedirecting}
					>
						<LogIn className="mr-2 h-4 w-4" />
						{isRedirecting ? "Opening sign-in..." : "Sign In to Continue"}
					</Button>
				</div>
			</main>
		);
	}

	return (
		<LoadingScreen
			progress={Math.min(30 + attempt * 10, 95)}
			message={attempt > 0 ? "Setting up your account..." : undefined}
		/>
	);
}
