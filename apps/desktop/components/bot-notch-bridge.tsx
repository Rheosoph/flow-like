"use client";

import {
	approvalOf,
	plainBody,
	rawApprovalOf,
} from "@flow-like/flow-like-ui/components/bots-tab/approval-entry";
import { useHiddenBots } from "@flow-like/flow-like-ui/components/bots-tab/use-bot-visibility";
import {
	botAutoApproves,
	botKeys,
	useAnswerBotApproval,
	useBotLabels,
	useBots,
	useSetBotAutoApprove,
} from "@flow-like/flow-like-ui/hooks/use-bots";
import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { useBackend } from "@flow-like/flow-like-ui/state/backend-state";
import type { IBotTurn } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import {
	botConversation,
	sendToHost,
} from "@flow-like/flow-like-ui/state/global-chat/bot-history";
import { useQueries } from "@tanstack/react-query";
import { emitTo, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useMemo, useRef } from "react";
import { pokeBotHost } from "../lib/bot-host-poke";
import { notchBotsOf, notchCandidates } from "../lib/bot-notch/model";
import {
	BRIDGE_EVENTS,
	NOTCH_WINDOW_LABEL,
	type NotchCommand,
	type NotchCommandEnvelope,
	type NotchCommandResult,
	type NotchEntry,
	type NotchSendResult,
	type NotchSnapshot,
} from "../lib/bot-notch/protocol";
import { nativeBotUrl } from "../lib/native-bots";

const ENTRY_SCAN = 40;

function isMainWindow(): boolean {
	try {
		return getCurrentWebviewWindow().label === "main";
	} catch {
		return false;
	}
}

/** Feeds the bot notch from the main window, which owns the account, the inbox and the bots. */
export function BotNotchBridge() {
	const main = useMemo(isMainWindow, []);
	return main ? <Bridge /> : null;
}

function errorCodeOf(body: Record<string, unknown> | null): string | null {
	const error = body?.error;
	return isRecord(error) && typeof error.code === "string" ? error.code : null;
}

/** The candidates that hold the auto-approval grant, from the same queries the bot panel reads. */
function useAutoApproving(botIds: readonly string[]): ReadonlySet<string> {
	const state = useBackend().botState;
	const grants = useQueries({
		queries: botIds.map((botId) => ({
			queryKey: botKeys.toolGrants(botId),
			queryFn: () => state?.listToolGrants?.(botId) ?? [],
			enabled: Boolean(state?.listToolGrants),
		})),
	});
	const key = grants
		.map((query, index) => (botAutoApproves(query.data) ? botIds[index] : ""))
		.join("\u0000");
	return useMemo(() => new Set(key.split("\u0000").filter(Boolean)), [key]);
}

function Bridge() {
	const backend = useBackend();
	const bots = useBots();
	const hidden = useHiddenBots();
	const botLabels = useBotLabels();
	const answer = useAnswerBotApproval();
	const autoApprove = useSetBotAutoApprove();
	const router = useClientRouter();

	const candidates = useMemo(
		() => notchCandidates(bots.data ?? [], hidden.keys).map((bot) => bot.id),
		[bots.data, hidden.keys],
	);
	const autoApproving = useAutoApproving(candidates);

	const snapshot = useMemo<NotchSnapshot>(
		() => ({
			ready: bots.isSuccess,
			bots: notchBotsOf(
				bots.data ?? [],
				hidden.keys,
				(bot) => botLabels(bot).name || bot.id,
				autoApproving,
			),
		}),
		[bots.isSuccess, bots.data, hidden.keys, botLabels, autoApproving],
	);

	const latest = useRef(snapshot);
	latest.current = snapshot;

	useEffect(() => {
		void emitTo(NOTCH_WINDOW_LABEL, BRIDGE_EVENTS.snapshot, snapshot).catch(
			() => undefined,
		);
	}, [snapshot]);

	const run = useRef<(command: NotchCommand) => Promise<unknown>>(
		async () => undefined,
	);
	run.current = async (command) => {
		const inbox = backend.inboxState;
		switch (command.kind) {
			case "snapshot":
				await emitTo(
					NOTCH_WINDOW_LABEL,
					BRIDGE_EVENTS.snapshot,
					latest.current,
				);
				return null;
			case "send": {
				if (!inbox) throw new Error("The inbox is not ready yet");
				const conversationId = await botConversation(inbox, command.botId);
				await sendToHost(
					inbox,
					{ botId: command.botId, conversationId },
					{
						id: command.messageId,
						text: command.text,
						pointedAt: command.pointedAt ?? undefined,
					},
				);
				pokeBotHost();
				return { conversationId } satisfies NotchSendResult;
			}
			case "entry": {
				if (!inbox) throw new Error("The inbox is not ready yet");
				const page = await inbox.listEntries(command.conversationId, {
					limit: ENTRY_SCAN,
				});
				const entry = page.entries.find(
					(candidate) => candidate.message_id === command.messageId,
				);
				const body = entry ? plainBody(entry) : null;
				return {
					text: typeof body?.text === "string" ? body.text : null,
					approval: entry ? (approvalOf(entry) ?? null) : null,
					card: entry ? (rawApprovalOf(entry) ?? null) : null,
					errorCode: errorCodeOf(body),
				} satisfies NotchEntry;
			}
			case "autoApprove":
				await autoApprove.mutateAsync({
					botId: command.botId,
					enabled: command.enabled,
				});
				return null;
			case "answer":
				await answer.mutateAsync({
					botId: command.botId,
					turnId: command.turnId,
					approvalId: command.approvalId,
					conversationId: command.conversationId,
					card: command.approval,
					decision: command.decision,
				});
				return null;
			case "open": {
				const window = getCurrentWebviewWindow();
				await window.show();
				await window.unminimize();
				await window.setFocus();
				router.push(
					nativeBotUrl({
						botId: command.botId ?? undefined,
						conversationId: command.conversationId ?? undefined,
					}),
				);
				return null;
			}
			case "stop": {
				const turns = backend.botState?.turns;
				if (!turns) throw new Error("This desktop runs no bot turns");
				const turn: IBotTurn = {
					id: command.turn.id,
					bot_id: command.turn.bot_id,
					conversation_id: command.turn.conversation_id,
					status: command.turn.status,
				};
				await turns.cancel(turn);
				return null;
			}
		}
	};

	useEffect(() => {
		const stop = listen<NotchCommandEnvelope>(
			BRIDGE_EVENTS.command,
			(event) => {
				const { id, command } = event.payload;
				void run
					.current(command)
					.then(
						(data): NotchCommandResult => ({ id, ok: true, data }),
						(error: unknown): NotchCommandResult => ({
							id,
							ok: false,
							error: error instanceof Error ? error.message : String(error),
						}),
					)
					.then((result) =>
						emitTo(NOTCH_WINDOW_LABEL, BRIDGE_EVENTS.result, result),
					)
					.catch((error) =>
						console.warn("[BotNotch] A command result was lost:", error),
					);
			},
		);
		return () => {
			void stop.then((unlisten) => unlisten());
		};
	}, []);

	return null;
}
