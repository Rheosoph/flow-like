"use client";

import { LoadingScreen, useBackend } from "@tm9657/flow-like-ui";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";

export default function JoinPage() {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("appId");
	const token = searchParams.get("token");
	const hasAttempted = useRef(false);

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
			console.error("App ID or token is missing in the URL parameters.");
			toast.error("Invalid invite link — missing app ID or token.");
			router.push("/");
			return;
		}

		try {
			await backend.teamState.joinInviteLink(appId, token);
			await addToProfile(appId);
			toast.success("Successfully joined the app!");
			router.push(`/use?id=${appId}`);
		} catch (error) {
			console.error("Failed to join via invite link:", error);
			toast.error("Failed to join. The invite link may be expired or invalid.");
			router.push("/");
		}
	}, [backend, appId, token, addToProfile, router]);

	useEffect(() => {
		if (hasAttempted.current) return;
		if (!auth.isAuthenticated || auth.isLoading) return;
		if (!appId || !token) return;

		hasAttempted.current = true;
		joinApp();
	}, [auth.isAuthenticated, auth.isLoading, appId, token, joinApp]);

	return <LoadingScreen />;
}
