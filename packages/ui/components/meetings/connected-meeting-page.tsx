"use client";

import { useTranslation } from "@flow-like/locales";
import { useSearchParams } from "next/navigation";
import { useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useHub } from "../../hooks/use-hub";
import {
	meetingInviteFromHash,
	meetingJoinUrl,
} from "../../lib/meetings/helpers";
import { meetingPageUrl } from "../../lib/meetings/transport";
import { openExternalUrl } from "../../lib/open-external";
import { isTauri } from "../../lib/platform";
import { useBackend, useSignedIn } from "../../state/backend-state";
import { Button } from "../ui/button";
import { MeetingPage } from "./meeting-page";

export function ConnectedMeetingPage() {
	const { t } = useTranslation("bots");
	const client = useBackend().meetingClient;
	const auth = useAuth();
	const signedIn = useSignedIn();
	const meetingId = useSearchParams().get("id") ?? "";
	const [invitation, setInvitation] = useState<{
		id: string;
		token?: string;
	}>();
	useEffect(() => {
		const read = () =>
			setInvitation({
				id: meetingId,
				token: meetingInviteFromHash(window.location.hash),
			});
		read();
		window.addEventListener("hashchange", read);
		return () => window.removeEventListener("hashchange", read);
	}, [meetingId]);
	if (!/^[A-Za-z0-9_-]{1,80}$/.test(meetingId)) {
		return (
			<p role="alert" className="p-6">
				{t(
					"crew.meetings.invalidLink",
					"This meeting link is incomplete. Ask the host for a new link.",
				)}
			</p>
		);
	}
	if (!client) {
		return (
			<p role="alert" className="p-6">
				{t(
					"crew.meetings.unsupportedClient",
					"Meetings are unavailable in this app.",
				)}
			</p>
		);
	}
	if (
		invitation?.id !== meetingId ||
		auth.isLoading ||
		(auth.isAuthenticated && !signedIn)
	) {
		return (
			<output className="p-6">
				{t("crew.meetings.loading", "Loading meeting…")}
			</output>
		);
	}
	const me =
		signedIn && auth.isAuthenticated ? auth.user?.profile.sub : undefined;
	const displayName =
		auth.user?.profile.name ?? auth.user?.profile.preferred_username;
	return (
		<div className="min-h-0 flex-1 overflow-auto">
			{isTauri() && !navigator.mediaDevices?.getDisplayMedia && (
				<MeetingBrowserLink
					meetingId={meetingId}
					inviteToken={invitation.token}
				/>
			)}
			<MeetingPage
				key={`${meetingId}:${me ?? "guest"}`}
				meetingId={meetingId}
				inviteToken={invitation.token}
				client={client}
				me={me}
				onSignIn={() =>
					auth.signinRedirect({
						url_state:
							window.location.pathname +
							window.location.search +
							window.location.hash,
					})
				}
				displayName={typeof displayName === "string" ? displayName : undefined}
			/>
		</div>
	);
}

function MeetingBrowserLink({
	meetingId,
	inviteToken,
}: { meetingId: string; inviteToken?: string }) {
	const { t } = useTranslation("bots");
	const { hub } = useHub();
	const [error, setError] = useState<string>();
	const base = meetingPageUrl(hub?.web);
	if (!base) return null;
	return (
		<aside className="flex flex-wrap items-center justify-center gap-3 border-b bg-muted/40 px-4 py-3 text-sm">
			<span>
				{t(
					"crew.meetings.browserScreenSharing",
					"For screen sharing, open this meeting in your browser.",
				)}
			</span>
			<Button
				variant="outline"
				size="sm"
				onClick={() =>
					void openExternalUrl(
						meetingJoinUrl(base, meetingId, inviteToken),
					).catch(() =>
						setError(
							t(
								"crew.meetings.browserOpenFailed",
								"The browser could not be opened. Copy the meeting link from the room and open it in your browser.",
							),
						),
					)
				}
			>
				{t("crew.meetings.openBrowser", "Open meeting in browser")}
			</Button>
			{error && <p role="alert">{error}</p>}
		</aside>
	);
}
