"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { presence } from "../../../../lib/device-management/model/presence";
import type {
	KeySessionSnapshot,
	LocalSummary,
	Preflight,
} from "../../../../lib/device-management/workspace/types";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "./device-workspace-provider";
import { useAttentionState } from "./use-attention";

/** Key snapshots also change when this computer's vault list does (setup, import, delete), which the key manager does not announce. */
function useKeySubscribe() {
	const { keys, local } = useDeviceWorkspace();
	return useCallback(
		(listener: () => void) => {
			const stopKeys = keys.subscribe(listener);
			const stopLocal = local.subscribe(listener);
			return () => {
				stopKeys();
				stopLocal();
			};
		},
		[keys, local],
	);
}

/** The key session of one device: state, idle lock time, last error (IA §6.4.1). */
export function useKeySession(deviceId: string): KeySessionSnapshot {
	const { keys } = useDeviceWorkspace();
	const subscribe = useKeySubscribe();
	const read = useCallback(() => keys.snapshot(deviceId), [keys, deviceId]);
	return useManagerValue(subscribe, read);
}

export interface KeyChip {
	unlockedCount: number;
	/** Every device with keys on this computer, plus open sessions. */
	sessions: KeySessionSnapshot[];
	lock(deviceId: string): void;
	lockAll(): void;
	setKeepUnlocked(deviceId: string, keep: boolean): void;
}

/** Top-bar "N unlocked" and its popover (SPEC §3.2.7). */
export function useKeyChip(): KeyChip {
	const { keys } = useDeviceWorkspace();
	const subscribe = useKeySubscribe();
	const read = useCallback(() => keys.list(), [keys]);
	const sessions = useManagerValue(subscribe, read);
	return useMemo(
		() => ({
			unlockedCount: sessions.filter((row) => row.state === "unlocked").length,
			sessions,
			lock: (deviceId) => keys.lock(deviceId),
			lockAll: () => keys.lockAll(),
			setKeepUnlocked: (deviceId, keep) => keys.setKeepUnlocked(deviceId, keep),
		}),
		[sessions, keys],
	);
}

export interface AccessChangePassword {
	/** True: every sharing and reader change asks for the device password again. */
	ask: boolean;
	setAsk(ask: boolean): void;
}

/** IA §6.4.1 "Ask for my password again for access changes" (per user, this computer; off by default). */
export function useAccessChangePassword(): AccessChangePassword {
	const { keys } = useDeviceWorkspace();
	const subscribe = useCallback(
		(listener: () => void) => keys.subscribe(listener),
		[keys],
	);
	const read = useCallback(() => keys.askPasswordForAccessChanges(), [keys]);
	const ask = useManagerValue(subscribe, read);
	return useMemo(
		() => ({
			ask,
			setAsk: (next) => keys.setAskPasswordForAccessChanges(next),
		}),
		[ask, keys],
	);
}

/** What this computer holds (P4): vaults, backups, storage persistence, authorities. */
export function useLocalSummary(): LocalSummary {
	const { local } = useDeviceWorkspace();
	const subscribe = useCallback(
		(listener: () => void) => local.subscribe(listener),
		[local],
	);
	const read = useCallback(() => local.summary(), [local]);
	return useManagerValue(subscribe, read);
}

export interface PreflightRead {
	preflight: Preflight | undefined;
	loading: boolean;
	error?: unknown;
	refresh(): Promise<void>;
}

/** D1–D9 before the password; re-run when the keys, this computer's storage or the hub row change. */
export function usePreflight(deviceId: string | undefined): PreflightRead {
	const { keys } = useDeviceWorkspace();
	const { input } = useAttentionState();
	const [state, setState] = useState<{
		deviceId?: string;
		preflight?: Preflight;
		error?: unknown;
		loading: boolean;
	}>({ loading: false });
	const run = useRef(0);

	const keyState = input.keys.find((row) => row.deviceId === deviceId)?.state;
	const row = input.devices.find((entry) => entry.device_id === deviceId);
	const checkIn = row ? presence(row, input.now).kind : undefined;
	const hubState = input.hub.state;
	const local = input.local;

	const refresh = useCallback(async () => {
		if (!deviceId) return;
		const current = ++run.current;
		setState((previous) => ({
			...(previous.deviceId === deviceId ? previous : {}),
			deviceId,
			loading: true,
		}));
		try {
			const preflight = await keys.preflight(deviceId);
			if (run.current === current)
				setState({ deviceId, preflight, loading: false });
		} catch (error) {
			if (run.current === current)
				setState((previous) => ({ ...previous, error, loading: false }));
		}
	}, [keys, deviceId]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: the listed snapshots are what the checks read
	useEffect(() => {
		void refresh();
		return () => {
			run.current++;
		};
	}, [refresh, keyState, hubState, local, row?.status, checkIn]);

	return useMemo(
		() => ({
			preflight: state.deviceId === deviceId ? state.preflight : undefined,
			loading: state.loading,
			...(state.error === undefined ? {} : { error: state.error }),
			refresh,
		}),
		[state, deviceId, refresh],
	);
}
