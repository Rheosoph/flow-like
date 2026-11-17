"use client";

import type { BotApprovalDecision } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import type {
	IInboxApproval,
	IInboxDesktopTarget,
} from "@flow-like/flow-like-ui/state/backend-state/inbox-state";
import { createId } from "@paralleldrive/cuid2";
import { useCallback, useEffect, useRef, useState } from "react";
import { previewText } from "../../lib/bot-notch/model";
import type {
	HostTurn,
	NotchEntry,
	NotchSendResult,
} from "../../lib/bot-notch/protocol";
import type { NotchBridgeCall } from "./use-notch-data";

export interface NotchTask {
	botId: string;
	messageId: string;
	conversationId: string | null;
	text: string;
}

export type NotchCard =
	| {
			kind: "approval";
			turn: HostTurn;
			approval: IInboxApproval;
			card: IInboxApproval;
	  }
	| {
			kind: "reply";
			conversationId: string | null;
			text: string;
			failed: boolean;
	  };

const REPLY_SHOWN_MS = 45_000;

function withEntry<K, V>(map: ReadonlyMap<K, V>, key: K, value: V) {
	const next = new Map(map);
	next.set(key, value);
	return next;
}

function withoutEntry<K, V>(map: ReadonlyMap<K, V>, key: K) {
	if (!map.has(key)) return map;
	const next = new Map(map);
	next.delete(key);
	return next;
}

/**
 * The tasks the notch handed its bots and the cards they bring back: the approval a waiting turn
 * sleeps on (any turn of a shown bot) and the reply to a task the notch sent.
 */
export function useNotchTasks(
	call: NotchBridgeCall,
	turns: ReadonlyMap<string, HostTurn>,
	shown: ReadonlySet<string>,
	messages: { sendFailed: (error: string) => string; failed: string },
) {
	const [tasks, setTasks] = useState<ReadonlyMap<string, NotchTask>>(
		() => new Map(),
	);
	const [cards, setCards] = useState<ReadonlyMap<string, NotchCard>>(
		() => new Map(),
	);
	const read = useRef(new Set<string>());
	const timers = useRef(new Map<string, number>());

	const dismiss = useCallback((botId: string) => {
		window.clearTimeout(timers.current.get(botId));
		timers.current.delete(botId);
		setCards((current) => withoutEntry(current, botId));
	}, []);

	const showReply = useCallback(
		(botId: string, card: Extract<NotchCard, { kind: "reply" }>) => {
			setCards((current) => withEntry(current, botId, card));
			window.clearTimeout(timers.current.get(botId));
			if (!card.failed)
				timers.current.set(
					botId,
					window.setTimeout(() => dismiss(botId), REPLY_SHOWN_MS),
				);
		},
		[dismiss],
	);

	useEffect(() => {
		for (const turn of turns.values()) {
			if (!shown.has(turn.bot_id)) continue;
			const task = tasks.get(turn.bot_id);
			const ours = task?.messageId === turn.trigger_message_id;
			const replyId = turn.reply_message_id;
			if (turn.status === "waiting" && replyId) {
				const key = `${turn.id}:${replyId}`;
				if (read.current.has(key)) continue;
				read.current.add(key);
				void call({
					kind: "entry",
					conversationId: turn.conversation_id,
					messageId: replyId,
				})
					.then((data) => {
						const { approval, card } = data as NotchEntry;
						if (approval)
							setCards((current) =>
								withEntry(current, turn.bot_id, {
									kind: "approval",
									turn,
									approval,
									card: card ?? approval,
								}),
							);
					})
					.catch((error) =>
						console.warn("[BotNotch] The approval card was not read:", error),
					);
				continue;
			}
			if (!ours || (turn.status !== "done" && turn.status !== "failed"))
				continue;
			const key = `${turn.id}:end`;
			if (read.current.has(key)) continue;
			read.current.add(key);
			const failed = turn.status === "failed";
			if (!replyId) {
				showReply(turn.bot_id, {
					kind: "reply",
					conversationId: turn.conversation_id,
					text: messages.failed,
					failed,
				});
				continue;
			}
			void call({
				kind: "entry",
				conversationId: turn.conversation_id,
				messageId: replyId,
			})
				.then((data) => {
					const entry = data as NotchEntry;
					showReply(turn.bot_id, {
						kind: "reply",
						conversationId: turn.conversation_id,
						text: entry.text ? previewText(entry.text) : messages.failed,
						failed: failed || entry.errorCode !== null,
					});
				})
				.catch((error) =>
					console.warn("[BotNotch] The reply was not read:", error),
				);
		}
		setCards((current) => {
			let next = current;
			for (const [botId, card] of current)
				if (
					card.kind === "approval" &&
					turns.get(card.turn.id)?.status !== "waiting"
				)
					next = withoutEntry(next, botId);
			return next;
		});
	}, [turns, tasks, shown, call, showReply, messages.failed]);

	useEffect(
		() => () => {
			for (const timer of timers.current.values()) window.clearTimeout(timer);
		},
		[],
	);

	const send = useCallback(
		async (
			botId: string,
			text: string,
			pointedAt: IInboxDesktopTarget | null,
		) => {
			const messageId = createId();
			dismiss(botId);
			setTasks((current) =>
				withEntry(current, botId, {
					botId,
					messageId,
					conversationId: null,
					text,
				}),
			);
			try {
				const result = (await call({
					kind: "send",
					botId,
					messageId,
					text,
					pointedAt,
				})) as NotchSendResult;
				setTasks((current) => {
					const task = current.get(botId);
					return task?.messageId === messageId
						? withEntry(current, botId, {
								...task,
								conversationId: result.conversationId,
							})
						: current;
				});
			} catch (error) {
				const reason = error instanceof Error ? error.message : String(error);
				setTasks((current) => withoutEntry(current, botId));
				showReply(botId, {
					kind: "reply",
					conversationId: null,
					text: messages.sendFailed(reason),
					failed: true,
				});
			}
		},
		[call, dismiss, showReply, messages],
	);

	const answer = useCallback(
		async (botId: string, decision: BotApprovalDecision) => {
			const card = cards.get(botId);
			if (card?.kind !== "approval") return;
			dismiss(botId);
			await call({
				kind: "answer",
				botId,
				turnId: card.turn.id,
				approvalId: card.approval.id,
				conversationId: card.turn.conversation_id,
				approval: card.card,
				decision,
			}).catch((error) => {
				setCards((current) => withEntry(current, botId, card));
				console.warn("[BotNotch] The approval answer failed:", error);
			});
		},
		[cards, call, dismiss],
	);

	return { tasks, cards, send, answer, dismiss };
}
