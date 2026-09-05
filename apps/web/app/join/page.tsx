"use client";

import {
	LoadingScreen,
	addAppToProfile,
	useBackend,
} from "@flow-like/flow-like-ui";
import {
	attemptJoinWithRetry,
	joinFailureMessage,
} from "@flow-like/flow-like-ui/lib/join-invite";
import { useTranslation } from "@flow-like/locales";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";

export default function JoinPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("appId");
	const token = searchParams.get("token");
	const hasAttempted = useRef(false);
	const [attempt, setAttempt] = useState(0);

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

	useEffect(() => {
		if (hasAttempted.current) return;
		if (!auth.isAuthenticated || auth.isLoading) return;
		if (!appId || !token) return;

		hasAttempted.current = true;
		joinApp();
	}, [auth.isAuthenticated, auth.isLoading, appId, token, joinApp]);

	return (
		<LoadingScreen
			progress={Math.min(30 + attempt * 10, 95)}
			message={
				attempt > 0
					? t("settingUpYourAccount", "Setting up your account...")
					: undefined
			}
		/>
	);
}
