"use client";

import { useCallback, useEffect, useState } from "react";
import { useBackend } from "../state/backend-state";
import type { IBotTurn } from "../state/backend-state/bot-state";

/** The turn this bot works on right now without a chat window, with the way to end it; `waiting` is a turn asleep until its owner answers an approval. */
export function useRunningBotTurn(botId?: string | null) {
	const host = useBackend().botState?.turns;
	const [turns, setTurns] = useState<ReadonlyMap<string, IBotTurn>>(new Map());
	const [cancelling, setCancelling] = useState<string | null>(null);

	useEffect(() => {
		if (!host) return;
		let active = true;
		let stop: (() => void) | undefined;
		const seen = new Set<string>();
		const put = (turn: IBotTurn) =>
			setTurns((current) => new Map(current).set(turn.id, turn));
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
		};
	}, [host]);

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
