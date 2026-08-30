"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
	PING_FIELD,
	PING_TTL_MS,
	type PingEmoji,
	SUMMON_FIELD,
	SUMMON_TTL_MS,
	sanitizePing,
	sanitizeSummon,
} from "../lib/realtime/presence-signals";
import {
	type DragPublisher,
	EMPTY_PEER_DRAGS,
	EMPTY_PEER_PINGS,
	type PeerDrag,
	type PeerPing,
	type PeerPresenceEvent,
	type PeerRun,
	type PeerSummon,
	type SignalStore,
	createDragPublisher,
	createPresenceSignalsObserver,
	createSignalStore,
} from "../lib/realtime/presence-signals-store";

export type {
	PeerDrag,
	PeerPing,
	PeerPresenceEvent,
	PeerRun,
	PeerSummon,
	SignalStore,
} from "../lib/realtime/presence-signals-store";

export interface UseRealtimeSignalsOptions {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	/** The local user's sub; their other windows are peers labelled "You". */
	sub?: string;
	layerPath: string | undefined;
	/** A peer asked everyone to jump to a viewport (once per summon, first-sight). */
	onSummon?: (summon: PeerSummon) => void;
	/** A peer's run finished (once per run, first-sight). */
	onPeerRun?: (run: PeerRun) => void;
	/** A user's first session appeared / last session left, debounced. */
	onPeerPresenceEvent?: (event: PeerPresenceEvent) => void;
}

export interface RealtimeSignals {
	/** Other sessions' live drags — consumed by FlowDragGhostsLayer. */
	dragStore: SignalStore<PeerDrag[]>;
	/** Live pings, own included — consumed by FlowPingsLayer. */
	pingStore: SignalStore<PeerPing[]>;
	/** Throttled to 20 Hz; call on every drag tick with the live positions. */
	broadcastDrag: (nodes: { id: string; x: number; y: number }[]) => void;
	/** Drop/cancel: withdraws the drag from the wire immediately. */
	endDrag: () => void;
	sendPing: (x: number, y: number, emoji?: PingEmoji) => void;
	summonPeers: (viewport: { x: number; y: number; zoom: number }) => void;
}

/**
 * Transient canvas signals over the board awareness: drag ghosts, pings,
 * summons, run outcomes and join/leave. Callbacks are read through refs, so a
 * parent re-render never re-subscribes; only a new awareness or sub does.
 */
export function useRealtimeSignals({
	awareness,
	sub,
	layerPath,
	onSummon,
	onPeerRun,
	onPeerPresenceEvent,
}: UseRealtimeSignalsOptions): RealtimeSignals {
	const [dragStore] = useState(() =>
		createSignalStore<PeerDrag[]>(EMPTY_PEER_DRAGS),
	);
	const [pingStore] = useState(() =>
		createSignalStore<PeerPing[]>(EMPTY_PEER_PINGS),
	);
	const callbacksRef = useRef({ onSummon, onPeerRun, onPeerPresenceEvent });
	callbacksRef.current = { onSummon, onPeerRun, onPeerPresenceEvent };
	const layerPathRef = useRef(layerPath);
	layerPathRef.current = layerPath;
	const awarenessRef = useRef(awareness);
	awarenessRef.current = awareness;
	const dragPublisherRef = useRef<DragPublisher | null>(null);
	const pingSeqRef = useRef(0);
	const summonSeqRef = useRef(0);

	useEffect(() => {
		if (!awareness) {
			dragStore.set(EMPTY_PEER_DRAGS);
			pingStore.set(EMPTY_PEER_PINGS);
			return;
		}
		const observer = createPresenceSignalsObserver(awareness, {
			selfSub: sub,
			dragStore,
			pingStore,
			onSummon: (summon) => callbacksRef.current.onSummon?.(summon),
			onPeerRun: (run) => callbacksRef.current.onPeerRun?.(run),
			onPeerPresenceEvent: (event) =>
				callbacksRef.current.onPeerPresenceEvent?.(event),
		});
		return () => {
			observer.dispose();
			dragStore.set(EMPTY_PEER_DRAGS);
			pingStore.set(EMPTY_PEER_PINGS);
		};
	}, [awareness, sub, dragStore, pingStore]);

	// The publisher withdraws the drag on unmount and whenever the awareness
	// is swapped, so a reconnect never leaves a ghost behind.
	useEffect(() => {
		if (!awareness) return;
		const publisher = createDragPublisher({ awareness });
		dragPublisherRef.current = publisher;
		return () => {
			publisher.dispose();
			if (dragPublisherRef.current === publisher)
				dragPublisherRef.current = null;
		};
	}, [awareness]);

	const broadcastDrag = useCallback(
		(nodes: { id: string; x: number; y: number }[]) => {
			dragPublisherRef.current?.publish(nodes);
		},
		[],
	);

	const endDrag = useCallback(() => {
		dragPublisherRef.current?.clear();
	}, []);

	// Pings and summons are moments, not state: withdraw them after their TTL
	// so a late joiner's first sight of us carries neither.
	const pingClearRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const summonClearRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	useEffect(
		() => () => {
			if (pingClearRef.current) clearTimeout(pingClearRef.current);
			if (summonClearRef.current) clearTimeout(summonClearRef.current);
		},
		[],
	);
	const sendPing = useCallback((x: number, y: number, emoji?: PingEmoji) => {
		const live = awarenessRef.current;
		if (!live) return;
		const payload = sanitizePing({
			x,
			y,
			layerPath: layerPathRef.current ?? "root",
			emoji,
			seq: ++pingSeqRef.current,
			ts: Date.now(),
		});
		if (!payload) return;
		live.setLocalStateField(PING_FIELD, payload);
		if (pingClearRef.current) clearTimeout(pingClearRef.current);
		pingClearRef.current = setTimeout(() => {
			pingClearRef.current = null;
			awarenessRef.current?.setLocalStateField(PING_FIELD, undefined);
		}, PING_TTL_MS);
	}, []);

	const summonPeers = useCallback(
		(viewport: { x: number; y: number; zoom: number }) => {
			const live = awarenessRef.current;
			if (!live) return;
			const payload = sanitizeSummon({
				x: viewport.x,
				y: viewport.y,
				zoom: viewport.zoom,
				layerPath: layerPathRef.current ?? "root",
				seq: ++summonSeqRef.current,
				ts: Date.now(),
			});
			if (!payload) return;
			live.setLocalStateField(SUMMON_FIELD, payload);
			if (summonClearRef.current) clearTimeout(summonClearRef.current);
			summonClearRef.current = setTimeout(() => {
				summonClearRef.current = null;
				awarenessRef.current?.setLocalStateField(SUMMON_FIELD, undefined);
			}, SUMMON_TTL_MS);
		},
		[],
	);

	return {
		dragStore,
		pingStore,
		broadcastDrag,
		endDrag,
		sendPing,
		summonPeers,
	};
}
