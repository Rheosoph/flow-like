"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import type {
	CallLane,
	DeviceWorkspace,
	LiveInspection,
	LiveSessionManager,
	LiveState,
	LiveStreams,
	StreamSpec,
	StreamState,
	UnlockStep,
} from "../../../../lib/device-management/workspace/types";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "./device-workspace-provider";

const IDLE: LiveState = { kind: "idle" };
const NO_STEPS: UnlockStep[] = [];

type StepsReader = { steps?(deviceId: string): UnlockStep[] };
type StreamExtras = {
	refresh?(deviceId: string, spec: StreamSpec): Promise<void>;
};

const sameSteps = (previous: UnlockStep[], next: UnlockStep[]) =>
	previous === next || JSON.stringify(previous) === JSON.stringify(next);

function useLiveSubscribe(live: LiveSessionManager) {
	return useCallback(
		(listener: () => void) => live.subscribe(listener),
		[live],
	);
}

export interface LiveSessionView {
	state: LiveState;
	/** Progress of the latest connection attempt (unlock sheet, diagnostics). */
	steps: UnlockStep[];
	inspection: LiveInspection | undefined;
	/** "Connect live": keeps a session while the device stays unlocked. Needs unlocked keys. */
	connect(): Promise<void>;
	/** "Try again" after a failed session: a failed session refuses every call until it is closed. */
	retry(): Promise<void>;
	close(): void;
	/** Re-reads the services. Never rejects: read `inspection.error` and `steps`. */
	refresh(): Promise<void>;
}

/**
 * The live session of one device. `demand` holds a session while the caller
 * is mounted (views that need live data); without it the hook only observes.
 */
export function useLiveSession(
	deviceId: string | undefined,
	options: { demand?: boolean } = {},
): LiveSessionView {
	const workspace = useDeviceWorkspace();
	const { live, keys } = workspace;
	const demand = options.demand === true;
	const subscribe = useLiveSubscribe(live);

	const readState = useCallback(() => {
		const state = deviceId ? live.state(deviceId) : IDLE;
		return state.kind === "idle" ? IDLE : state;
	}, [live, deviceId]);
	const readSteps = useCallback(
		() => (deviceId && (live as StepsReader).steps?.(deviceId)) || NO_STEPS,
		[live, deviceId],
	);
	const readInspection = useCallback(
		() => (deviceId ? live.inspection(deviceId) : undefined),
		[live, deviceId],
	);
	const state = useManagerValue(subscribe, readState);
	const steps = useManagerValue(subscribe, readSteps, sameSteps);
	const inspection = useManagerValue(subscribe, readInspection);

	useEffect(() => {
		if (!demand || !deviceId) return;
		return live.acquire(deviceId, "view");
	}, [live, deviceId, demand]);

	return useMemo(
		() => ({
			state,
			steps,
			inspection,
			connect: async () => {
				if (!deviceId) return;
				workspace.touch(deviceId);
				await keys.unlock(deviceId, "", { connectLive: true });
			},
			retry: async () => {
				if (!deviceId) return;
				workspace.touch(deviceId);
				live.close(deviceId);
				await live.refreshInspection(deviceId);
			},
			close: () => {
				if (deviceId) live.close(deviceId);
			},
			refresh: async () => {
				if (deviceId) await live.refreshInspection(deviceId);
			},
		}),
		[state, steps, inspection, workspace, keys, live, deviceId],
	);
}

/** The live inspection as last read; `undefined` before the first read and after a lock. */
export function useLiveInspection(
	deviceId: string | undefined,
): LiveInspection | undefined {
	const { live } = useDeviceWorkspace();
	const subscribe = useLiveSubscribe(live);
	const read = useCallback(
		() => (deviceId ? live.inspection(deviceId) : undefined),
		[live, deviceId],
	);
	return useManagerValue(subscribe, read);
}

/** A call bound to the device, not to a connection; user and operation calls count as use for the idle lock. */
export function deviceCall(
	workspace: DeviceWorkspace,
	deviceId: string,
	lane: CallLane = "user",
): ManagementCall {
	const call = workspace.live.call(deviceId, { lane });
	return (command, operationId) => {
		if (lane !== "poll") workspace.touch(deviceId);
		return call(command, operationId);
	};
}

export function useDeviceCall(
	deviceId: string,
	lane: CallLane = "user",
): ManagementCall {
	const workspace = useDeviceWorkspace();
	return useMemo(
		() => deviceCall(workspace, deviceId, lane),
		[workspace, deviceId, lane],
	);
}

export interface LiveStreamView<T> extends StreamState<T> {
	/** False until the stream reported once. */
	started: boolean;
	loadOlder(): Promise<void>;
	refresh(): Promise<void>;
}

const NOT_STARTED: StreamState<never> = {
	freshness: { src: "live", age: "notloaded" },
	gaps: [],
};

function specKey(deviceId: string | undefined, spec: StreamSpec | null) {
	return deviceId && spec ? `${deviceId}|${JSON.stringify(spec)}` : "";
}

/**
 * One live stream (metrics, logs, messages, offline queues, certificates,
 * shared metrics). Subscribing is the demand: the device is polled only while
 * a view is mounted. `null` as spec idles.
 */
export function useLiveStream<T>(
	deviceId: string | undefined,
	spec: StreamSpec | null,
): LiveStreamView<T> {
	const { streams } = useDeviceWorkspace();
	const key = specKey(deviceId, spec);
	const [entry, setEntry] = useState<{ key: string; state: StreamState<T> }>();

	// biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for the device and the spec's content
	useEffect(() => {
		if (!deviceId || !spec) return;
		return streams.subscribe<T>(deviceId, spec, (state) =>
			setEntry({ key, state }),
		);
	}, [streams, key]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for the device and the spec's content
	return useMemo(() => {
		const state = entry?.key === key ? entry.state : undefined;
		return {
			...(state ?? (NOT_STARTED as StreamState<T>)),
			started: state !== undefined,
			loadOlder: async () => {
				if (deviceId && (spec?.kind === "logs" || spec?.kind === "messages"))
					await (streams as LiveStreams).loadOlder(deviceId, spec);
			},
			refresh: async () => {
				if (deviceId && spec)
					await (streams as StreamExtras).refresh?.(deviceId, spec);
			},
		};
	}, [streams, entry, key]);
}
