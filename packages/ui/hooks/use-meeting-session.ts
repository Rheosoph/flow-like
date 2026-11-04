"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { meetingError, meetingPhase } from "../lib/meetings/helpers";
import type {
	MeetingClient,
	MeetingCredentials,
	MeetingSession,
} from "../lib/meetings/types";

export const MEETING_POLL_MS = 3000;
export const MEETING_SESSION_LEASE_MS = 12_000;

export function meetingSessionActive(
	snapshot: MeetingSession,
	receivedAt: number,
	now: number,
): boolean {
	const elapsed = now - receivedAt;
	const serverNow = Date.parse(snapshot.server_time) + elapsed;
	return (
		Number.isFinite(serverNow) &&
		elapsed >= 0 &&
		snapshot.me.status === "admitted" &&
		elapsed < MEETING_SESSION_LEASE_MS &&
		meetingPhase(snapshot.meeting, serverNow) === "open"
	);
}

export function useMeetingSession(
	client: MeetingClient,
	meetingId: string,
	credentials?: MeetingCredentials,
) {
	const token = credentials?.session_token;
	const key = `${meetingId}:${token ?? ""}`;
	const [snapshot, setSnapshot] = useState<{
		key: string;
		value: MeetingSession;
		receivedAt: number;
	}>();
	const [failure, setFailure] = useState<{
		key: string;
		message: string;
		terminal: boolean;
	}>();
	const [now, setNow] = useState(Date.now);
	const refreshRef = useRef<() => Promise<void>>(async () => {});
	useEffect(() => {
		if (!token) return;
		let disposed = false;
		let loading = false;
		const refresh = async () => {
			if (disposed || loading) return;
			loading = true;
			try {
				const value = await client.session(meetingId, token);
				if (disposed) return;
				const receivedAt = Date.now();
				setSnapshot({ key, value, receivedAt });
				setNow(receivedAt);
				setFailure(undefined);
			} catch (cause) {
				if (!disposed) {
					const status = (cause as { status?: number } | null)?.status;
					setFailure({
						key,
						message: meetingError(cause),
						terminal:
							status !== undefined && [401, 403, 404, 410].includes(status),
					});
				}
			} finally {
				loading = false;
			}
		};
		refreshRef.current = refresh;
		void refresh();
		const poll = setInterval(() => void refresh(), MEETING_POLL_MS);
		const clock = setInterval(() => setNow(Date.now()), 1000);
		return () => {
			disposed = true;
			clearInterval(poll);
			clearInterval(clock);
			refreshRef.current = async () => {};
		};
	}, [client, meetingId, token, key]);
	const current = snapshot?.key === key ? snapshot : undefined;
	const error = failure?.key === key ? failure : undefined;
	return {
		snapshot: current?.value,
		active:
			!!current &&
			!error?.terminal &&
			meetingSessionActive(current.value, current.receivedAt, now),
		error: error?.message,
		terminal: error?.terminal ?? false,
		refresh: useCallback(() => refreshRef.current(), []),
	};
}
