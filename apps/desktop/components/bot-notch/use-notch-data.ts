"use client";

import { createId } from "@paralleldrive/cuid2";
import { emitTo, listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { notchCommands } from "../../lib/bot-notch/commands";
import {
	BRIDGE_EVENTS,
	type HostTurn,
	NOTCH_EVENTS,
	type NotchCommand,
	type NotchCommandEnvelope,
	type NotchCommandResult,
	type NotchLayout,
	type NotchSettings,
	type NotchSnapshot,
	type PerchFrame,
} from "../../lib/bot-notch/protocol";

const COMMAND_TIMEOUT_MS = 30_000;
const TURN_MEMORY_MS = 10 * 60_000;
const EMPTY_SNAPSHOT: NotchSnapshot = { ready: false, bots: [] };

/** Calls `handler` with each payload of a Tauri event while mounted. */
export function useTauriEvent<T>(event: string, handler: (payload: T) => void) {
	const latest = useRef(handler);
	latest.current = handler;
	useEffect(() => {
		const stop = listen<T>(event, (message) => latest.current(message.payload));
		return () => {
			void stop.then((unlisten) => unlisten());
		};
	}, [event]);
}

export function useNotchLayout(): NotchLayout | null {
	const [layout, setLayout] = useState<NotchLayout | null>(null);
	useEffect(() => {
		notchCommands
			.layout()
			.then(setLayout)
			.catch((error) => console.warn("[BotNotch] No display layout:", error));
	}, []);
	useTauriEvent<NotchLayout>(NOTCH_EVENTS.layout, setLayout);
	return layout;
}

export function useNotchSettings(): NotchSettings | null {
	const [settings, setSettings] = useState<NotchSettings | null>(null);
	useEffect(() => {
		notchCommands
			.settings()
			.then(setSettings)
			.catch((error) => console.warn("[BotNotch] No settings:", error));
	}, []);
	useTauriEvent<NotchSettings>(NOTCH_EVENTS.settings, setSettings);
	return settings;
}

export function useHostId(): string | null {
	const [hostId, setHostId] = useState<string | null>(null);
	useEffect(() => {
		notchCommands
			.hostId()
			.then(setHostId)
			.catch((error) => console.warn("[BotNotch] No bot host id:", error));
	}, []);
	return hostId;
}

/** Asks the main window to act; resolves with what it answered. */
export type NotchBridgeCall = (command: NotchCommand) => Promise<unknown>;

export function useNotchBridge(): {
	snapshot: NotchSnapshot;
	call: NotchBridgeCall;
} {
	const [snapshot, setSnapshot] = useState(EMPTY_SNAPSHOT);
	const ready = useRef(false);
	const pending = useRef(
		new Map<
			string,
			{ resolve: (value: unknown) => void; reject: (error: Error) => void }
		>(),
	);
	useTauriEvent<NotchSnapshot>(BRIDGE_EVENTS.snapshot, (next) => {
		ready.current = next.ready;
		setSnapshot(next);
	});
	useTauriEvent<NotchCommandResult>(BRIDGE_EVENTS.result, (result) => {
		const waiter = pending.current.get(result.id);
		if (!waiter) return;
		pending.current.delete(result.id);
		if (result.ok) waiter.resolve(result.data);
		else waiter.reject(new Error(result.error));
	});

	const call = useCallback<NotchBridgeCall>((command) => {
		const id = createId();
		return new Promise((resolve, reject) => {
			const timer = window.setTimeout(() => {
				pending.current.delete(id);
				reject(
					new Error(
						`The main window did not answer the notch's "${command.kind}" request`,
					),
				);
			}, COMMAND_TIMEOUT_MS);
			pending.current.set(id, {
				resolve: (value) => {
					window.clearTimeout(timer);
					resolve(value);
				},
				reject: (error) => {
					window.clearTimeout(timer);
					reject(error);
				},
			});
			const envelope: NotchCommandEnvelope = { id, command };
			emitTo("main", BRIDGE_EVENTS.command, envelope).catch((error) => {
				pending.current.delete(id);
				window.clearTimeout(timer);
				reject(error instanceof Error ? error : new Error(String(error)));
			});
		});
	}, []);

	useEffect(() => {
		const ask = () => {
			if (!ready.current)
				void call({ kind: "snapshot" }).catch(() => undefined);
		};
		ask();
		const retry = window.setInterval(ask, 4_000);
		return () => window.clearInterval(retry);
	}, [call]);

	return { snapshot, call };
}

function recentEnough(turn: HostTurn, now: number): boolean {
	if (!turn.finished_at) return true;
	const finished = Date.parse(turn.finished_at);
	return !Number.isFinite(finished) || now - finished < TURN_MEMORY_MS;
}

/** The bot host's turns on this computer, live. */
export function useHostTurns(): ReadonlyMap<string, HostTurn> {
	const [turns, setTurns] = useState<ReadonlyMap<string, HostTurn>>(
		() => new Map(),
	);
	const upsert = useCallback((incoming: readonly HostTurn[]) => {
		setTurns((current) => {
			const now = Date.now();
			const next = new Map(current);
			for (const turn of incoming) next.set(turn.id, turn);
			for (const [id, turn] of next)
				if (!recentEnough(turn, now)) next.delete(id);
			return next;
		});
	}, []);
	useEffect(() => {
		notchCommands
			.hostTurns()
			.then((list) => upsert(Array.isArray(list) ? list : []))
			.catch(() => undefined);
	}, [upsert]);
	useTauriEvent<HostTurn>(NOTCH_EVENTS.turn, (turn) => upsert([turn]));
	return turns;
}

export function usePerchFrames(): ReadonlyMap<string, PerchFrame> {
	const [frames, setFrames] = useState<ReadonlyMap<string, PerchFrame>>(
		() => new Map(),
	);
	useTauriEvent<{ frames: PerchFrame[] }>(NOTCH_EVENTS.frames, (payload) =>
		setFrames(new Map(payload.frames.map((frame) => [frame.botId, frame]))),
	);
	return frames;
}
