"use client";

import { useHub } from "../../hooks/use-hub";
import { useClientRouter } from "../../lib/client-navigation";
import { meetingPageUrl } from "../../lib/meetings/transport";
import type { MeetingClient } from "../../lib/meetings/types";
import { isTauri } from "../../lib/platform";
import { RoomMeetings } from "../meetings/room-meetings";

export function ConnectedRoomMeetings({
	conversationId,
	client,
	me,
}: {
	conversationId: string;
	client: MeetingClient;
	me: string;
}) {
	const { hub } = useHub();
	const router = useClientRouter();
	const joinBaseUrl = meetingPageUrl(
		hub?.web,
		typeof window !== "undefined" && !isTauri()
			? window.location.origin
			: undefined,
	);
	if (!joinBaseUrl) return null;
	return (
		<RoomMeetings
			conversationId={conversationId}
			client={client}
			me={me}
			joinBaseUrl={joinBaseUrl}
			onJoin={
				isTauri()
					? (id) => router.push(`/meeting?id=${encodeURIComponent(id)}`)
					: undefined
			}
		/>
	);
}
