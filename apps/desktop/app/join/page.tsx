"use client";

import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";

import { Button, addAppToProfile, useBackend } from "@flow-like/flow-like-ui";
import { BOTS_ROUTE } from "@flow-like/flow-like-ui/components/bots-tab/dock-selection";
import { useJoinRoomLink } from "@flow-like/flow-like-ui/hooks/use-join-room";
import {
	attemptJoinWithRetry,
	joinFailureMessage,
} from "@flow-like/flow-like-ui/lib/join-invite";
import { useTranslation } from "@flow-like/locales";
import { Loader2, LogIn } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	clearPendingInvite,
	setPendingInvite,
	setPendingRoomInvite,
} from "../../lib/pending-invite";

export default function JoinPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const auth = useAuth();
	const router = useClientRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("appId");
	const token = searchParams.get("token");
	const room = appId ? null : searchParams.get("room");
	const validInvite = Boolean(room || (appId && token));
	const joinRoomLink = useJoinRoomLink();
	const hasAttempted = useRef(false);
	const [attempt, setAttempt] = useState(0);
	const [isRedirecting, setIsRedirecting] = useState(false);

	// Persist immediately: sign-in leaves the app and onboarding may navigate
	// away — PendingInviteRedeemer brings the user back here afterwards.
	useEffect(() => {
		if (appId && token) setPendingInvite(appId, token);
		else if (room) setPendingRoomInvite(room);
	}, [appId, token, room]);

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

		console.error("Failed to join:", result.kind);
		toast.error(joinFailureMessage(result.kind ?? "retry-exhausted"));
		router.push("/");
	}, [backend, appId, token, router]);

	const joinRoom = useCallback(
		async (roomToken: string) => {
			const conversationId = await joinRoomLink(roomToken, setAttempt);
			clearPendingInvite();
			// Scrub the token from the address bar and history before leaving.
			window.history.replaceState(null, "", "/");
			router.push(
				conversationId
					? `${BOTS_ROUTE}?conversation=${encodeURIComponent(conversationId)}`
					: "/",
			);
		},
		[joinRoomLink, router],
	);

	useEffect(() => {
		if (hasAttempted.current) return;
		if (!(room || (appId && token))) return;
		if (auth.isLoading || !auth.isAuthenticated) return;

		hasAttempted.current = true;
		if (room) void joinRoom(room);
		else joinApp();
	}, [
		auth.isAuthenticated,
		auth.isLoading,
		appId,
		token,
		room,
		joinApp,
		joinRoom,
	]);

	const handleSignIn = useCallback(async () => {
		setIsRedirecting(true);
		try {
			await auth.signinRedirect();
		} catch (error) {
			console.error("Sign-in redirect failed:", error);
			setIsRedirecting(false);
		}
	}, [auth]);

	if (!auth.isLoading && !auth.isAuthenticated && validInvite) {
		return (
			<main className="flex h-full w-full flex-1 items-center justify-center p-6">
				<div className="w-full max-w-md space-y-4 rounded-xl border bg-card p-8 text-center shadow-floating">
					<h2 className="text-xl font-semibold">
						{t("youaposveBeenInvited", "You've been invited")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t(
							"signInToAcceptThisInviteYouaposllBeBroughtRightBackHereAfterwards",
							"Sign in to accept this invite. You'll be brought right back here afterwards.",
						)}
					</p>
					<Button
						className="w-full"
						onClick={handleSignIn}
						disabled={isRedirecting}
					>
						<LogIn className="mr-2 h-4 w-4" />
						{isRedirecting
							? t("openingSignin", "Opening sign-in...")
							: t("signInToContinue", "Sign In to Continue")}
					</Button>
				</div>
			</main>
		);
	}

	return (
		<main className="flex h-full w-full flex-1 items-center justify-center p-6">
			<div className="w-full max-w-md space-y-4 rounded-xl border bg-card p-8 text-center shadow-floating">
				<h2 className="text-xl font-semibold">
					{t("youaposveBeenInvited", "You've been invited")}
				</h2>
				{validInvite ? (
					<output className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
						<Loader2
							aria-hidden="true"
							className="size-4 animate-spin motion-reduce:animate-none"
						/>
						{auth.isLoading
							? t("loading", "Loading…")
							: attempt > 0
								? t("settingUpYourAccount", "Setting up your account...")
								: t("acceptingInvitation", "Accepting invitation…")}
					</output>
				) : (
					<>
						<p role="alert" className="text-sm text-muted-foreground">
							{t(
								"incompleteInviteLink",
								"This invite link is incomplete. Open the full link from your invitation.",
							)}
						</p>
						<Button variant="outline" onClick={() => router.push("/")}>
							{t("returnHome", "Return Home")}
						</Button>
					</>
				)}
			</div>
		</main>
	);
}
