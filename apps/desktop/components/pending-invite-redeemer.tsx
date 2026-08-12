"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect } from "react";
import { useAuth } from "react-oidc-context";
import { getPendingInvite } from "../lib/pending-invite";

/**
 * Brings a signed-in user back to /join when an invite is still pending —
 * covering every flow that navigates away mid-join (onboarding redirects,
 * ProfileSyncer's hard reload, app restarts after a cold-start deep link).
 * Skips /onboarding so it never fights the profile setup flow.
 */
export function PendingInviteRedeemer() {
	const auth = useAuth();
	const pathname = usePathname();
	const router = useRouter();

	useEffect(() => {
		if (!auth.isAuthenticated) return;
		if (pathname?.startsWith("/join") || pathname?.startsWith("/onboarding"))
			return;
		const pending = getPendingInvite();
		if (!pending) return;
		router.push(
			`/join?appId=${encodeURIComponent(pending.appId)}&token=${encodeURIComponent(pending.token)}`,
		);
	}, [auth.isAuthenticated, pathname, router]);

	return null;
}
