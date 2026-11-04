"use client";

import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type ConversationBotActivity,
	createTypingTracker,
	hasActiveTurn,
	mergeBotActivity,
} from "../lib/realtime/conversation-presence";
import {
	CHAT_TYPING_FIELD,
	TYPING_HEARTBEAT_MS,
	TYPING_TTL_MS,
} from "../lib/realtime/presence-signals";
import { useBackend } from "../state/backend-state";
import type { IBotTurn } from "../state/backend-state/bot-state";
import type {
	ConversationTurnStatus,
	IConversation,
	IConversationTurn,
} from "../state/backend-state/inbox-state";
import type { ConversationRoom } from "./use-conversation-room";
import { inboxKeys } from "./use-inbox";

export type { ConversationBotActivity };
export type { ConversationTurnStatus };

export interface ConversationActivity {
	/** One per bot, with its most advanced status. */
	bots: readonly ConversationBotActivity[];
	/** Person subs, never me, oldest typer first. */
	typing: readonly string[];
	/** Call on every draft change; an empty draft clears. */
	notifyTyping: (draft: string) => void;
	/** Call on send. */
	stopTyping: () => void;
}

const TURN_POLL_MS = 2000;
const NO_TURNS: readonly IConversationTurn[] = [];

function useLiveConversationTurns(conversationId: string): readonly IBotTurn[] {
	const host = useBackend().botState?.turns;
	const [turns, setTurns] = useState<ReadonlyMap<string, IBotTurn>>(new Map());

	useEffect(() => {
		if (!host) return;
		let active = true;
		let stop: (() => void) | undefined;
		const seen = new Set<string>();
		const put = (turn: IBotTurn) => {
			if (turn.conversation_id !== conversationId) return;
			setTurns((current) => new Map(current).set(turn.id, turn));
		};
		host
			.subscribe((turn) => {
				seen.add(turn.id);
				put(turn);
			})
			.then((unlisten) => {
				if (active) stop = unlisten;
				else unlisten();
			})
			.catch((cause) => console.warn("Listening to bot turns failed", cause));
		host
			.list()
			.then((list) => {
				if (!active) return;
				for (const turn of list) if (!seen.has(turn.id)) put(turn);
			})
			.catch((cause) => console.warn("Reading bot turns failed", cause));
		return () => {
			active = false;
			stop?.();
			setTurns(new Map());
		};
	}, [host, conversationId]);

	return useMemo(() => [...turns.values()], [turns]);
}

function useConversationBots(
	conversation: IConversation,
	newestEntryKey: string | undefined,
): readonly ConversationBotActivity[] {
	const inbox = useBackend().inboxState;
	const listTurns = inbox?.listConversationTurns?.bind(inbox);
	const enabled = !conversation.local && listTurns !== undefined;
	const live = useLiveConversationTurns(conversation.id);

	const query = useQuery({
		queryKey: inboxKeys.turns(conversation.id),
		queryFn: () => listTurns?.(conversation.id) ?? Promise.resolve([]),
		enabled,
		refetchInterval: (current) =>
			hasActiveTurn(current.state.data ?? NO_TURNS) ? TURN_POLL_MS : false,
	});

	const refetch = query.refetch;
	const lastKey = useRef(newestEntryKey);
	useEffect(() => {
		if (lastKey.current === newestEntryKey) return;
		lastKey.current = newestEntryKey;
		if (enabled) void refetch();
	}, [newestEntryKey, enabled, refetch]);

	const botIds = useMemo(
		() =>
			new Set(
				conversation.members
					.filter((member) => member.kind === "bot")
					.map((member) => member.principal_id),
			),
		[conversation.members],
	);
	const apiTurns = enabled ? (query.data ?? NO_TURNS) : NO_TURNS;

	return useMemo(
		() => mergeBotActivity(apiTurns, live, botIds, Date.now()),
		[apiTurns, live, botIds],
	);
}

function useTypingPeople(
	conversation: IConversation,
	me: string | null | undefined,
	room: ConversationRoom | undefined,
): Pick<ConversationActivity, "typing" | "notifyTyping" | "stopTyping"> {
	const awareness = room?.awareness;
	const people = useMemo(
		() =>
			conversation.members
				.filter((member) => member.kind === "user")
				.map((member) => member.principal_id),
		[conversation.members],
	);
	const allowed = useMemo(
		() => new Set(people.filter((id) => id !== me)),
		[people, me],
	);
	const [typing, setTyping] = useState<readonly string[]>([]);

	useEffect(() => {
		if (!awareness) setTyping([]);
	}, [awareness]);

	useEffect(() => {
		if (!awareness) return;
		const tracker = createTypingTracker();
		let timer: ReturnType<typeof setTimeout> | null = null;
		const update = () => {
			if (timer !== null) clearTimeout(timer);
			timer = null;
			const now = Date.now();
			const snapshot = tracker.observe(
				awareness.getStates(),
				awareness.clientID,
				allowed,
				now,
			);
			setTyping((current) =>
				current.length === snapshot.typing.length &&
				current.every((sub, index) => sub === snapshot.typing[index])
					? current
					: snapshot.typing,
			);
			if (snapshot.nextExpiryAt !== null) {
				timer = setTimeout(
					update,
					Math.max(snapshot.nextExpiryAt - now, 0) + 1,
				);
			}
		};
		awareness.on("change", update);
		update();
		return () => {
			awareness.off("change", update);
			if (timer !== null) clearTimeout(timer);
			tracker.reset();
		};
	}, [awareness, allowed]);

	const publishedAt = useRef(0);
	const clearTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

	const stopTyping = useCallback(() => {
		if (clearTimer.current !== undefined) {
			clearTimeout(clearTimer.current);
			clearTimer.current = undefined;
		}
		publishedAt.current = 0;
		if (!awareness) return;
		try {
			if (awareness.getLocalState()?.[CHAT_TYPING_FIELD] === undefined) return;
			awareness.setLocalStateField(CHAT_TYPING_FIELD, undefined);
		} catch {}
	}, [awareness]);

	const notifyTyping = useCallback(
		(draft: string) => {
			if (!awareness) return;
			if (draft.trim().length === 0) {
				stopTyping();
				return;
			}
			const now = Date.now();
			if (now - publishedAt.current >= TYPING_HEARTBEAT_MS) {
				publishedAt.current = now;
				awareness.setLocalStateField(CHAT_TYPING_FIELD, { ts: now });
			}
			if (clearTimer.current !== undefined) clearTimeout(clearTimer.current);
			clearTimer.current = setTimeout(stopTyping, TYPING_TTL_MS);
		},
		[awareness, stopTyping],
	);

	useEffect(() => stopTyping, [stopTyping]);

	return { typing, notifyTyping, stopTyping };
}

/** `room` is the conversation's peer room from `useConversationRoom`, shared with the call. */
export function useConversationActivity(
	conversation: IConversation,
	me: string | null | undefined,
	newestEntryKey: string | undefined,
	room: ConversationRoom | undefined,
): ConversationActivity {
	const bots = useConversationBots(conversation, newestEntryKey);
	const { typing, notifyTyping, stopTyping } = useTypingPeople(
		conversation,
		me,
		room,
	);
	return useMemo(
		() => ({ bots, typing, notifyTyping, stopTyping }),
		[bots, typing, notifyTyping, stopTyping],
	);
}
