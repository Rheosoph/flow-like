"use client";

import { useEffect, useRef, useState } from "react";
import { meetingError } from "../lib/meetings/helpers";
import type { MeetingClient } from "../lib/meetings/types";
import { conversationRealtimeRoom } from "../lib/realtime/conversation-room";
import {
	type RealtimeSession,
	createRealtimeSession,
} from "../lib/realtime/webrtc";
import type { ConversationRoom } from "./use-conversation-room";

/** Meeting credentials only authorize this meeting's separate peer room. */
export function useMeetingRoom({
	client,
	meetingId,
	token,
	participantId,
	active,
	roomEpoch,
	signalingServers,
}: {
	client: MeetingClient;
	meetingId: string;
	token: string;
	participantId: string;
	active: boolean;
	roomEpoch: number;
	signalingServers?: string[];
}) {
	const [result, setResult] = useState<{
		key: string;
		room: ConversationRoom;
	}>();
	const [error, setError] = useState<string>();
	const key = `${meetingId}:${token}:${participantId}`;
	const signalingKey = JSON.stringify(signalingServers ?? null);
	const refreshRef = useRef<() => Promise<void>>(async () => {});
	useEffect(() => {
		if (!active) {
			setResult(undefined);
			return;
		}
		let disposed = false;
		let rotating = false;
		let refreshAgain = false;
		let session: RealtimeSession | undefined;
		let roomKey = "";
		let refreshTimer: ReturnType<typeof setTimeout> | undefined;
		let expiryTimer: ReturnType<typeof setTimeout> | undefined;
		const teardown = (preserveView = false) => {
			session?.dispose();
			session = undefined;
			roomKey = "";
			if (!disposed && !preserveView) setResult(undefined);
		};
		const refresh = async () => {
			if (disposed) return;
			if (rotating) {
				refreshAgain = true;
				return;
			}
			rotating = true;
			clearTimeout(refreshTimer);
			try {
				const access = await client.realtime(meetingId, token);
				if (disposed) return;
				const nextKey = `${access.room_id}:${access.key_id}`;
				if (session && roomKey !== nextKey) teardown(true);
				if (!session) {
					const next = await createRealtimeSession({
						room: conversationRealtimeRoom(access.room_id),
						access,
						sub: participantId,
						signalingServers:
							access.signaling_servers ?? JSON.parse(signalingKey) ?? undefined,
						onAuthFailure: () => {
							teardown();
							void refresh();
						},
					});
					if (disposed) {
						next.dispose();
						return;
					}
					session = next;
					roomKey = nextKey;
					next.awareness.setLocalStateField("sub", participantId);
					setResult({
						key,
						room: { awareness: next.awareness, provider: next.provider },
					});
				} else {
					session.refreshAccess(access);
				}
				setError(undefined);
				clearTimeout(expiryTimer);
				const expiresIn = Math.max(0, access.expires_in * 1000);
				expiryTimer = setTimeout(() => teardown(), expiresIn);
				refreshTimer = setTimeout(
					() => void refresh(),
					Math.max(1000, Math.min(30_000, expiresIn - 5000)),
				);
			} catch (cause) {
				if (!disposed) {
					teardown();
					setError(meetingError(cause));
					refreshTimer = setTimeout(() => void refresh(), 3000);
				}
			} finally {
				rotating = false;
				if (refreshAgain && !disposed) {
					refreshAgain = false;
					void refresh();
				}
			}
		};
		refreshRef.current = refresh;
		void refresh();
		return () => {
			disposed = true;
			clearTimeout(refreshTimer);
			clearTimeout(expiryTimer);
			teardown();
			refreshRef.current = async () => {};
		};
	}, [active, client, key, meetingId, token, participantId, signalingKey]);
	useEffect(() => {
		if (roomEpoch >= 0) void refreshRef.current();
	}, [roomEpoch]);
	return {
		room: active && result?.key === key ? result.room : undefined,
		error,
	};
}
