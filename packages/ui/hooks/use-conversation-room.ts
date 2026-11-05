"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { decodeJwtExpiryMs } from "../lib/realtime/authenticated-websocket";
import { conversationRealtimeRoom } from "../lib/realtime/conversation-room";
import type { IRealtimeAccess } from "../lib/realtime/types";
import { createRealtimeSession } from "../lib/realtime/webrtc";
import { useBackend } from "../state/backend-state";
import type { IConversation } from "../state/backend-state/inbox-state";
import { useHub } from "./use-hub";

export interface RoomAwareness {
	clientID: number;
	getStates(): Map<number, unknown>;
	getLocalState(): Record<string, unknown> | null;
	setLocalStateField(field: string, value: unknown): void;
	on(event: "change", listener: () => void): void;
	off(event: "change", listener: () => void): void;
}

/** The slice of y-webrtc's `WebrtcProvider` that calls use. */
export interface RoomProvider {
	room?: {
		peerId: string;
		webrtcConns: Map<string, { peer?: unknown }>;
	} | null;
	on(event: "peers", listener: (change: unknown) => void): void;
	off(event: "peers", listener: (change: unknown) => void): void;
}

export interface ConversationRoom {
	awareness: RoomAwareness;
	provider: RoomProvider;
}

const TOKEN_ROTATE_MARGIN_MS = 5 * 60 * 1000;
const ICE_ROTATE_MARGIN_MS = 2 * 60 * 1000;
const ACCESS_RETRY_MS = 15 * 1000;
const MIN_ROTATE_DELAY_MS = 60 * 1000;

function iceExpiryMs(access: IRealtimeAccess): number | null {
	return typeof access.ice_expires_at === "number" &&
		Number.isFinite(access.ice_expires_at) &&
		access.ice_expires_at > 0
		? access.ice_expires_at * 1000
		: null;
}

/**
 * The live peer room of an account conversation, or undefined while there is none (local
 * conversations, a missing hub or no other member). Token and ICE rotation happen inside; a membership
 * change rotates the room key and rebuilds the session, so a returned room stays valid until
 * the hook returns another one.
 */
export function useConversationRoom(
	conversation: IConversation,
	me: string | null | undefined,
): ConversationRoom | undefined {
	const inbox = useBackend().inboxState;
	const canFetchAccess = inbox?.getConversationRealtimeAccess !== undefined;
	const signaling = useHub().hub?.signaling ?? undefined;
	const hasSignaling = (signaling?.length ?? 0) > 0;
	const signalingRef = useRef(signaling);
	signalingRef.current = signaling;

	const people = useMemo(
		() =>
			conversation.members
				.filter((member) => member.kind === "user")
				.map((member) => member.principal_id),
		[conversation.members],
	);
	const memberKey = useMemo(
		() =>
			conversation.members
				.map((m) => m.principal_id)
				.sort()
				.join(","),
		[conversation.members],
	);

	const connect =
		!conversation.local &&
		canFetchAccess &&
		!!me &&
		(people.length >= 2 ||
			conversation.members.some((member) => member.kind === "bot")) &&
		people.includes(me) &&
		hasSignaling;

	const [room, setRoom] = useState<ConversationRoom>();
	const conversationId = conversation.id;

	// biome-ignore lint/correctness/useExhaustiveDependencies: a membership change rotates the room key, so the session is rebuilt
	useEffect(() => {
		const fetchAccess = inbox?.getConversationRealtimeAccess?.bind(inbox);
		if (!connect || !fetchAccess) return;
		let disposed = false;
		let rotating = false;
		let session: Awaited<ReturnType<typeof createRealtimeSession>> | null =
			null;
		let tokenExpiresAt: number | null = null;
		let iceExpiresAt: number | null = null;
		let rotateTimer: ReturnType<typeof setTimeout> | null = null;
		let retryTimer: ReturnType<typeof setTimeout> | null = null;
		let retryDelay = 2000;

		const clearTimers = () => {
			if (rotateTimer !== null) clearTimeout(rotateTimer);
			if (retryTimer !== null) clearTimeout(retryTimer);
			rotateTimer = null;
			retryTimer = null;
		};

		const scheduleRotation = () => {
			if (rotateTimer !== null) clearTimeout(rotateTimer);
			const times: number[] = [];
			if (tokenExpiresAt !== null)
				times.push(tokenExpiresAt - TOKEN_ROTATE_MARGIN_MS);
			if (iceExpiresAt !== null)
				times.push(iceExpiresAt - ICE_ROTATE_MARGIN_MS);
			if (times.length === 0) return;
			rotateTimer = setTimeout(
				() => {
					rotateTimer = null;
					void rotate(true);
				},
				Math.max(Math.min(...times) - Date.now(), MIN_ROTATE_DELAY_MS),
			);
		};

		const track = (access: IRealtimeAccess) => {
			tokenExpiresAt = decodeJwtExpiryMs(access.jwt);
			iceExpiresAt = iceExpiryMs(access);
			scheduleRotation();
		};

		let keyId: string | null = null;

		const rotate = async (onlyIfDue: boolean) => {
			if (disposed || rotating || !session) return;
			const now = Date.now();
			const due =
				tokenExpiresAt === null ||
				tokenExpiresAt - now <= TOKEN_ROTATE_MARGIN_MS ||
				(iceExpiresAt !== null && iceExpiresAt - now <= ICE_ROTATE_MARGIN_MS);
			if (onlyIfDue && !due) {
				scheduleRotation();
				return;
			}
			rotating = true;
			try {
				const access = await fetchAccess(conversationId);
				if (disposed || !session) return;
				if (keyId !== null && access.key_id !== keyId) {
					teardown();
					await start(access);
					return;
				}
				session.refreshAccess(access);
				track(access);
			} catch (cause) {
				console.warn("Conversation realtime refresh failed", cause);
				if (!disposed) {
					if (rotateTimer !== null) clearTimeout(rotateTimer);
					rotateTimer = setTimeout(() => {
						rotateTimer = null;
						void rotate(true);
					}, ACCESS_RETRY_MS);
				}
			} finally {
				rotating = false;
			}
		};

		const teardown = () => {
			try {
				session?.dispose();
			} catch {}
			session = null;
			keyId = null;
			setRoom(undefined);
		};

		const start = async (prefetched?: IRealtimeAccess) => {
			try {
				const access = prefetched ?? (await fetchAccess(conversationId));
				const next = await createRealtimeSession({
					room: conversationRealtimeRoom(conversationId),
					access,
					sub: me ?? undefined,
					signalingServers: signalingRef.current,
					onAuthFailure: () => {
						void rotate(false);
					},
				});
				if (disposed) {
					next.dispose();
					return;
				}
				session = next;
				keyId = access.key_id ?? null;
				retryDelay = 2000;
				track(access);
				next.awareness.setLocalStateField("sub", me);
				setRoom({ awareness: next.awareness, provider: next.provider });
			} catch (cause) {
				console.warn("Conversation realtime setup failed", cause);
				if (!disposed && retryTimer === null) {
					retryTimer = setTimeout(() => {
						retryTimer = null;
						void start();
					}, retryDelay);
					retryDelay = Math.min(retryDelay * 2, 60_000);
				}
			}
		};

		void start();
		return () => {
			disposed = true;
			clearTimers();
			teardown();
		};
	}, [connect, inbox, conversationId, me, memberKey]);

	return room;
}
