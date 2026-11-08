"use client";

import { useCallback, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useBackend } from "../state/backend-state";
import type { IBotTurn } from "../state/backend-state/bot-state";
import { invalidateBotConfiguration } from "./use-bots";

/** The turn this bot works on right now without a chat window, with the way to end it; `waiting` is a turn asleep until its owner answers an approval. */
export function useRunningBotTurn(botId?: string | null) {
	const host = useBackend().botState?.turns;
	const queryClient = useQueryClient();
	const [turns, setTurns] = useState<ReadonlyMap<string, IBotTurn>>(new Map());
	const [cancelling, setCancelling] = useState<string | null>(null);

	useEffect(() => {
		if (!host) return;
		let active = true;
		let stop: (() => void) | undefined;
		const seen = new Set<string>();
		const finished = new Set<string>();
		const put = (turn: IBotTurn) =>
			setTurns((current) => new Map(current).set(turn.id, turn));
		host
			.subscribe((turn) => {
				if (!active) return;
				if (
					turn.bot_id === botId &&
					["done", "failed", "cancelled"].includes(turn.status) &&
					!finished.has(turn.id)
				) {
					finished.add(turn.id);
					invalidateBotConfiguration(queryClient, turn.bot_id);
				}
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
		};
	}, [host, botId, queryClient]);

	const turn = botId
		? [...turns.values()].find(
				(item) => item.bot_id === botId && item.status === "running",
			)
		: undefined;

	const waiting = botId
		? [...turns.values()].find(
				(item) => item.bot_id === botId && item.status === "waiting",
			)
		: undefined;

	const cancel = useCallback(async () => {
		if (!host || !turn) return;
		setCancelling(turn.id);
		try {
			await host.cancel(turn);
			setTurns((current) =>
				new Map(current).set(turn.id, { ...turn, status: "cancelled" }),
			);
		} catch (cause) {
			console.warn(`Cancelling turn ${turn.id} failed`, cause);
		} finally {
			setCancelling(null);
		}
	}, [host, turn]);

	return {
		turn,
		waiting,
		cancel,
		cancelling: turn !== undefined && cancelling === turn.id,
	};
}
