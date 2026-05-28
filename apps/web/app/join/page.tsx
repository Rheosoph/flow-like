"use client";

import { LoadingScreen, useBackend } from "@flow-like/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";

const MAX_RETRIES = 6;
const BASE_DELAY = 800;

export default function JoinPage() {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("appId");
	const token = searchParams.get("token");
	const hasAttempted = useRef(false);
	const [attempt, setAttempt] = useState(0);

	const addToProfile = useCallback(
		async (appId: string) => {
			try {
				const profile = await backend.userState.getSettingsProfile();
				await backend.userState.updateProfileApp(
					profile,
					{ app_id: appId, favorite: false, pinned: false },
					"Upsert",
				);
			} catch (error) {
				console.error("Failed to add app to profile:", error);
			}
		},
		[backend],
	);

	const joinApp = useCallback(async () => {
		if (!appId || !token) {
			toast.error("Invalid invite link — missing app ID or token.");
			router.push("/");
			return;
		}

		for (let i = 0; i <= MAX_RETRIES; i++) {
			setAttempt(i);
			try {
				await backend.teamState.joinInviteLink(appId, token);
				await addToProfile(appId);
				toast.success("Successfully joined the app!");
				router.push(`/use?id=${appId}`);
				return;
			} catch (error) {
				if (i === MAX_RETRIES) {
					console.error("Failed to join after retries:", error);
					toast.error(
						"Failed to join. The invite link may be expired or invalid.",
					);
					router.push("/");
					return;
				}
				const delay = BASE_DELAY * 2 ** i;
				await new Promise((r) => setTimeout(r, delay));
			}
		}
	}, [backend, appId, token, addToProfile, router]);

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
			message={attempt > 0 ? "Setting up your account..." : undefined}
		/>
	);
}
