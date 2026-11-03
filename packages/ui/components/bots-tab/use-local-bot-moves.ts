"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type UseQueryResult,
	useQueries,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	botKeys,
	useDeleteBotSecret,
	useSetBotApp,
	useUpdateBot,
} from "../../hooks/use-bots";
import { inboxKeys } from "../../hooks/use-inbox";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import type {
	BotMoveStep,
	IBot,
	IBotMoveIssue,
	IBotMoveReport,
	ILocalBotHost,
} from "../../state/backend-state/bot-state";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { useBotModelOptions } from "./editor/steps/use-bot-models";
import {
	type MoveFix,
	clientMoveIssues,
	mergeMoveIssues,
	moveFixRequest,
} from "./move-checks";

export const moveCheckKeys = {
	all: ["bots-move-check"] as const,
	bot: (botId: string) => [...moveCheckKeys.all, botId] as const,
};

export type MoveOutcome =
	| { status: "moving"; bot: IBot }
	| { status: "moved"; bot: IBot; warnings: number }
	| { status: "failed"; bot: IBot; cause: unknown };

/** One bot in the review: what its check found and what happened to it. */
export interface BotMoveReview {
	/** The id the bot had on this computer; checks and outcomes stay under it after the move. */
	id: string;
	/** The bot on this computer, or the account bot once it moved. */
	bot: IBot;
	report?: IBotMoveReport;
	issues: IBotMoveIssue[];
	checking: boolean;
	checkFailed: boolean;
	outcome?: MoveOutcome;
	/** Nothing blocks it and it hasn't moved yet. */
	ready: boolean;
	recheck: () => void;
}

interface MoveRow {
	id: string;
	bot: IBot;
}

/**
 * Rows in the order the bots first showed up. A bot that moved leaves the local list but keeps
 * its row, so the review can say it moved; a bot that vanished otherwise drops out.
 */
function useStableRows(
	live: readonly IBot[],
	outcomes: Readonly<Record<string, MoveOutcome>>,
): MoveRow[] {
	const [order, setOrder] = useState(() => live.map((bot) => bot.id));
	const added = live.filter((bot) => !order.includes(bot.id));
	if (added.length > 0) setOrder([...order, ...added.map((bot) => bot.id)]);
	const byId = new Map(live.map((bot) => [bot.id, bot]));
	return order.flatMap((id) => {
		const bot = byId.get(id) ?? outcomes[id]?.bot;
		return bot ? [{ id, bot }] : [];
	});
}

/**
 * The global chat keeps talking to a moved bot under the id its account gave it. A chat that
 * can't move is reported; the move itself already happened.
 */
export async function followMovedBot(fromBotId: string, toBotId: string) {
	if (fromBotId === toBotId) return;
	const chat = useGlobalChatStore.getState();
	if (!("rebindBotChat" in chat) || typeof chat.rebindBotChat !== "function")
		return;
	try {
		await chat.rebindBotChat(fromBotId, toBotId);
	} catch (cause) {
		console.warn(
			`The chat couldn't follow bot ${fromBotId} to its new id ${toBotId}`,
			cause,
		);
	}
}

/** The server's check of every listed bot plus the client's own: models and look. */
export function useMoveReviews(
	host: ILocalBotHost,
	bots: readonly IBot[],
	outcomes: Readonly<Record<string, MoveOutcome>>,
): BotMoveReview[] {
	const rows = useStableRows(bots, outcomes);
	const models = useBotModelOptions();
	const modelIds = useMemo(
		() => new Set(models.options.map((option) => option.id)),
		[models.options],
	);
	const checks: UseQueryResult<IBotMoveReport>[] = useQueries({
		queries: rows.map(({ id }) => {
			const status = outcomes[id]?.status;
			return {
				queryKey: moveCheckKeys.bot(id),
				queryFn: () => host.checkMove(id),
				enabled: status === undefined || status === "failed",
				staleTime: Number.POSITIVE_INFINITY,
				gcTime: 0,
				retry: false,
			};
		}),
	});
	return rows.map(({ id, bot }, index) => {
		const check = checks[index];
		const outcome = outcomes[id];
		const report = check?.data;
		const checking = Boolean(check?.isPending) || models.isLoading;
		const issues = mergeMoveIssues(
			report?.issues ?? [],
			models.isLoading ? [] : clientMoveIssues(bot, modelIds),
		);
		const settled = outcome?.status === "moving" || outcome?.status === "moved";
		return {
			id,
			bot,
			report,
			issues,
			checking,
			checkFailed: Boolean(check?.isError),
			outcome,
			ready:
				!settled &&
				!checking &&
				Boolean(check?.isSuccess) &&
				issues.length === 0,
			recheck: () => void check?.refetch(),
		};
	});
}

/** Applies one fix from the review, then checks the bot again. */
export function useMoveFix(onEditBot: (botId: string) => void) {
	const { t } = useTranslation("bots");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const { mutateAsync: updateBot } = useUpdateBot();
	const { mutateAsync: setBotApp } = useSetBotApp();
	const { mutateAsync: deleteSecret } = useDeleteBotSecret();
	const settings = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profileId = settings.data?.hub_profile.id ?? null;
	const [fixing, setFixing] = useState<string | null>(null);

	const apply = useCallback(
		async (bot: IBot, fix: MoveFix) => {
			if (fix.kind === "open_settings") {
				onEditBot(bot.id);
				return;
			}
			setFixing(bot.id);
			try {
				if (fix.kind === "detach_app") {
					await setBotApp({ botId: bot.id, appId: fix.appId, attached: false });
				} else if (fix.kind === "remove_secret") {
					await deleteSecret({ botId: bot.id, name: fix.name });
				} else {
					const request = moveFixRequest(fix, bot, profileId);
					if (request) await updateBot({ botId: bot.id, request });
				}
				await queryClient.invalidateQueries({
					queryKey: moveCheckKeys.bot(bot.id),
				});
			} catch (cause) {
				console.warn("Fixing a bot before its move failed", cause);
				toast.error(
					t("move.fix.failed", "That fix didn't go through. Try again."),
				);
			} finally {
				setFixing(null);
			}
		},
		[deleteSecret, onEditBot, profileId, queryClient, setBotApp, t, updateBot],
	);

	return { apply, fixing, profileId };
}

/** The step each moving bot is at, as the desktop tells it, by the bot's id on this computer. */
function useMoveSteps(
	host: ILocalBotHost,
): Readonly<Record<string, BotMoveStep>> {
	const [steps, setSteps] = useState<Record<string, BotMoveStep>>({});
	useEffect(() => {
		if (!host.onProgress) return;
		let active = true;
		let stop: (() => void) | undefined;
		host
			.onProgress(({ bot_id, step }) =>
				setSteps((current) => ({ ...current, [bot_id]: step })),
			)
			.then((unlisten) => {
				if (active) stop = unlisten;
				else unlisten();
			})
			.catch((cause) => console.warn("Listening to the move failed", cause));
		return () => {
			active = false;
			stop?.();
		};
	}, [host]);
	return steps;
}

/**
 * Moves the ready bots one at a time. A failure keeps that bot here with its cause and checks
 * it again; closing the review stops before the next bot. A moved bot's chat follows it to the
 * id its account gave it.
 */
export function useMoveRun(host: ILocalBotHost, onMoved: (bot: IBot) => void) {
	const queryClient = useQueryClient();
	const [outcomes, setOutcomes] = useState<Record<string, MoveOutcome>>({});
	const steps = useMoveSteps(host);
	const [progress, setProgress] = useState<{
		done: number;
		total: number;
	} | null>(null);
	const running = useRef(false);
	const closed = useRef(false);
	useEffect(() => {
		closed.current = false;
		return () => {
			closed.current = true;
		};
	}, []);

	const record = useCallback(
		(botId: string, outcome: MoveOutcome) =>
			setOutcomes((current) => ({ ...current, [botId]: outcome })),
		[],
	);

	const move = useCallback(
		async (queue: readonly IBot[]) => {
			if (running.current || queue.length === 0) return;
			running.current = true;
			setProgress({ done: 0, total: queue.length });
			const moved: IBot[] = [];
			const failed: string[] = [];
			for (const bot of queue) {
				if (closed.current) break;
				record(bot.id, { status: "moving", bot });
				const result = await host.move(bot.id).catch((cause: unknown) => {
					failed.push(bot.id);
					record(bot.id, { status: "failed", bot, cause });
					return undefined;
				});
				if (result) {
					const next = result.bot;
					moved.push(next);
					record(bot.id, {
						status: "moved",
						bot: next,
						warnings: result.warnings ?? 0,
					});
					await followMovedBot(bot.id, next.id);
				}
				setProgress((current) =>
					current ? { ...current, done: current.done + 1 } : current,
				);
			}
			running.current = false;
			setProgress(null);
			void queryClient.invalidateQueries({ queryKey: botKeys.all });
			void queryClient.invalidateQueries({ queryKey: inboxKeys.all });
			for (const botId of failed) {
				void queryClient.invalidateQueries({
					queryKey: moveCheckKeys.bot(botId),
				});
			}
			if (moved[0] && !closed.current) onMoved(moved[0]);
		},
		[host, onMoved, queryClient, record],
	);

	return { outcomes, progress, steps, move };
}
