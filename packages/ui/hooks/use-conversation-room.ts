"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { decodeJwtExpiryMs } from "../lib/realtime/authenticated-websocket";
import {
	conversationRealtimeRoom,
	createLocalConversationRoom,
} from "../lib/realtime/conversation-room";
import type { IRealtimeAccess } from "../lib/realtime/types";
import {
	createRealtimeSession,
	realtimeIceExpiryMs,
} from "../lib/realtime/webrtc";
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
const KEY_ROLL_SPREAD_MS = 5 * 1000;
const KEY_ROLL_WINDOW_MS = 10 * 60 * 1000;
const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * A room key carries the UTC day it was issued on, and from midnight the hub hands out the
 * next day's. People on two keys cannot reach each other, so everyone fetches the new key
 * as the day turns. Null for a key id that names no day.
 */
export function roomKeyRollsAt(keyId: string): number | null {
	const day = /^(\d{4})-(\d{2})-(\d{2})\./.exec(keyId);
	if (!day) return null;
	const start = Date.UTC(Number(day[1]), Number(day[2]) - 1, Number(day[3]));
	return Number.isFinite(start) ? start + DAY_MS : null;
}

/** What in a room's access runs out, in epoch milliseconds. */
export interface RoomAccessClock {
	tokenExpiresAt: number | null;
	iceExpiresAt: number | null;
	keyRollsAt: number | null;
}

export function roomAccessClock(access: IRealtimeAccess): RoomAccessClock {
	return {
		tokenExpiresAt: decodeJwtExpiryMs(access.jwt),
		iceExpiresAt: realtimeIceExpiryMs(access),
		keyRollsAt: roomKeyRollsAt(access.key_id ?? ""),
	};
}

// A clock far from the hub's must not keep asking for a key that is not due there.
const keyRollPending = (clock: RoomAccessClock, now: number) =>
	clock.keyRollsAt !== null && now < clock.keyRollsAt + KEY_ROLL_WINDOW_MS;

/** Whether the access should be fetched again now. */
export function roomAccessDue(clock: RoomAccessClock, now: number): boolean {
	return (
		clock.tokenExpiresAt === null ||
		clock.tokenExpiresAt - now <= TOKEN_ROTATE_MARGIN_MS ||
		(clock.iceExpiresAt !== null &&
			clock.iceExpiresAt - now <= ICE_ROTATE_MARGIN_MS) ||
		(clock.keyRollsAt !== null &&
			now >= clock.keyRollsAt &&
			keyRollPending(clock, now))
	);
}

/**
 * When to fetch the access again: before the token or the relay credentials lapse and as
 * the key's day turns, never sooner than a minute from now. `keyRollSpread` keeps people
 * from all asking in the same instant. Null when nothing in the access runs out.
 */
export function roomAccessRefreshAt(
	clock: RoomAccessClock,
	now: number,
	keyRollSpread = 0,
): number | null {
	const times: number[] = [];
	if (clock.tokenExpiresAt !== null)
		times.push(clock.tokenExpiresAt - TOKEN_ROTATE_MARGIN_MS);
	if (clock.iceExpiresAt !== null)
		times.push(clock.iceExpiresAt - ICE_ROTATE_MARGIN_MS);
	if (clock.keyRollsAt !== null && keyRollPending(clock, now))
		times.push(clock.keyRollsAt + keyRollSpread);
	return times.length === 0
		? null
		: Math.max(Math.min(...times), now + MIN_ROTATE_DELAY_MS);
}

/**
 * The live peer room of an account conversation, or a device-only room for local bots.
 * Token and ICE rotation happen inside. A new room key (the day turned, or a refresh met
 * a changed membership) rebuilds the session and swaps the room in place, so a call in it
 * carries over; a returned room stays valid until the hook returns another one.
 */
export function useConversationRoom(
	conversation: IConversation,
	me: string | null | undefined,
	enabled = true,
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
		enabled &&
		!conversation.local &&
		canFetchAccess &&
		!!me &&
		(people.length >= 2 ||
			conversation.members.some((member) => member.kind === "bot")) &&
		people.includes(me) &&
		hasSignaling;
	const localEnabled =
		enabled &&
		Boolean(conversation.local) &&
		!!me &&
		people.length === 1 &&
		people.includes(me) &&
		conversation.members.some((member) => member.kind === "bot");
	// biome-ignore lint/correctness/useExhaustiveDependencies: local call state belongs to this conversation and its current members
	const localRoom = useMemo(
		() => (localEnabled && me ? createLocalConversationRoom(me) : undefined),
		[localEnabled, me, conversation.id, memberKey],
	);

	const [room, setRoom] = useState<ConversationRoom>();
	const conversationId = conversation.id;
	const refetchRef = useRef<() => void>(() => {});
	const openedFor = useRef(memberKey);

	// biome-ignore lint/correctness/useExhaustiveDependencies: a membership change fetches the access again in place instead of restarting the session
	useEffect(() => {
		const fetchAccess = inbox?.getConversationRealtimeAccess?.bind(inbox);
		if (!connect || !fetchAccess) return;
		openedFor.current = memberKey;
		let disposed = false;
		let rotating = false;
		let stale = false;
		let session: Awaited<ReturnType<typeof createRealtimeSession>> | null =
			null;
		let clock: RoomAccessClock = {
			tokenExpiresAt: null,
			iceExpiresAt: null,
			keyRollsAt: null,
		};
		let rotateTimer: ReturnType<typeof setTimeout> | null = null;
		let retryTimer: ReturnType<typeof setTimeout> | null = null;
		let retryDelay = 2000;
		const keyRollSpread = Math.random() * KEY_ROLL_SPREAD_MS;

		const clearTimers = () => {
			if (rotateTimer !== null) clearTimeout(rotateTimer);
			if (retryTimer !== null) clearTimeout(retryTimer);
			rotateTimer = null;
			retryTimer = null;
		};

		const scheduleRotation = () => {
			if (rotateTimer !== null) clearTimeout(rotateTimer);
			const now = Date.now();
			const at = roomAccessRefreshAt(clock, now, keyRollSpread);
			if (at === null) return;
			rotateTimer = setTimeout(() => {
				rotateTimer = null;
				void rotate(true);
			}, at - now);
		};

		const track = (access: IRealtimeAccess) => {
			clock = roomAccessClock(access);
			scheduleRotation();
		};

		let keyId: string | null = null;

		const rotate = async (onlyIfDue: boolean) => {
			if (disposed) return;
			if (rotating || !session) {
				// Asked while a fetch is in flight: answered once that one is done.
				if (!onlyIfDue) stale = true;
				return;
			}
			if (onlyIfDue && !roomAccessDue(clock, Date.now())) {
				scheduleRotation();
				return;
			}
			rotating = true;
			stale = false;
			try {
				const access = await fetchAccess(conversationId);
				if (disposed || !session) return;
				if (keyId !== null && access.key_id !== keyId) {
					// Consumers keep the old room until the new one stands, so a call carries over.
					release();
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
						void rotate(onlyIfDue);
					}, ACCESS_RETRY_MS);
				}
			} finally {
				rotating = false;
				refetchIfStale();
			}
		};

		const refetchIfStale = () => {
			if (stale && session && !rotating) void rotate(false);
		};

		const release = () => {
			try {
				session?.dispose();
			} catch {}
			session = null;
			keyId = null;
		};

		const teardown = () => {
			release();
			setRoom(undefined);
		};

		const start = async (prefetched?: IRealtimeAccess) => {
			try {
				if (!prefetched) stale = false;
				const access = prefetched ?? (await fetchAccess(conversationId));
				if (disposed) return;
				const next = await createRealtimeSession({
					room: conversationRealtimeRoom(conversationId),
					access,
					sub: me ?? undefined,
					signalingServers: signalingRef.current,
					onAuthFailure: () => {
						void rotate(false);
					},
				});
				try {
					// y-webrtc opens its room once the key is derived; a call reads the room's peer id.
					await (next.provider as { key?: PromiseLike<unknown> }).key;
				} catch (cause) {
					next.dispose();
					throw cause;
				}
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
				refetchIfStale();
			} catch (cause) {
				console.warn("Conversation realtime setup failed", cause);
				if (disposed) return;
				// A rebuild that failed would leave consumers on a closed room.
				setRoom(undefined);
				if (retryTimer === null) {
					retryTimer = setTimeout(() => {
						retryTimer = null;
						void start();
					}, retryDelay);
					retryDelay = Math.min(retryDelay * 2, 60_000);
				}
			}
		};

		refetchRef.current = () => void rotate(false);
		void start();
		return () => {
			disposed = true;
			refetchRef.current = () => {};
			clearTimers();
			teardown();
		};
	}, [connect, inbox, conversationId, me]);

	// Someone joining or leaving changes the room key. The access is fetched again and the
	// room swapped in place, so the change does not end a call for the people in it.
	useEffect(() => {
		if (openedFor.current === memberKey) return;
		openedFor.current = memberKey;
		refetchRef.current();
	}, [memberKey]);

	return localRoom ?? (connect ? room : undefined);
}
