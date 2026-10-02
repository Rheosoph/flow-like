"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo } from "react";
import { isActivityFinished } from "../../../../lib/device-management/workspace/activity";
import type { DeviceErrorCode } from "../../../../lib/device-management/workspace/errors";
import type {
	ActivityItem,
	ActivityKind,
	DeviceWorkspace,
} from "../../../../lib/device-management/workspace/types";
import { errorCopy } from "../copy/error-copy";
import type { DevicesT } from "../primitives/area-context";
import type { ResultTone } from "../primitives/inline-result";
import {
	useDeviceWorkspace,
	useManagerValue,
} from "./device-workspace-provider";

export interface ActivityFilter {
	deviceId?: string;
	serviceId?: string;
	projectId?: string;
	kind?: ActivityKind;
}

export interface ActivityView {
	/** Tray order: in progress · no reply received · finished. */
	items: ActivityItem[];
	inProgress: ActivityItem[];
	noReply: ActivityItem[];
	finished: ActivityItem[];
	dismiss(itemId: string): void;
	/** "Check result": re-read what the device or the hub says about the open items. */
	checkAgain(deviceId?: string): Promise<void>;
}

const matches = (item: ActivityItem, filter: ActivityFilter) =>
	(!filter.deviceId || item.target.deviceId === filter.deviceId) &&
	(!filter.serviceId || item.target.serviceId === filter.serviceId) &&
	(!filter.projectId || item.target.projectId === filter.projectId) &&
	(!filter.kind || item.kind === filter.kind);

/** The activity tray (SPEC §3.6): tracked operations of this account on this computer. */
export function useActivity(filter: ActivityFilter = {}): ActivityView {
	const { activity } = useDeviceWorkspace();
	const subscribe = useCallback(
		(listener: () => void) => activity.subscribe(listener),
		[activity],
	);
	const read = useCallback(() => activity.list(), [activity]);
	const all = useManagerValue(subscribe, read);
	const { deviceId, serviceId, projectId, kind } = filter;
	return useMemo(() => {
		const items = all.filter((item) =>
			matches(item, { deviceId, serviceId, projectId, kind }),
		);
		return {
			items,
			inProgress: items.filter(
				(item) => !isActivityFinished(item) && item.state !== "unknown",
			),
			noReply: items.filter((item) => item.state === "unknown"),
			finished: items.filter(
				(item) => item.state === "done" || item.state === "failed",
			),
			dismiss: (itemId) => activity.dismiss(itemId),
			checkAgain: (device) => activity.resume(device),
		};
	}, [all, activity, deviceId, serviceId, projectId, kind]);
}

/* Inline results (R9): the outcome next to the control that started it, until dismissed. */

export type InlineResultState =
	| "running"
	| "done"
	| "failed"
	| "rejected"
	| "unknown";

export interface InlineResultEntry {
	id: string;
	/** Which control group shows it, e.g. `service:<device>/<service>`. */
	scopeKey: string;
	/** Verb + object in the user's language at the time ("Stop support-bot"). */
	label: string;
	state: InlineResultState;
	deviceId?: string;
	/** Tray item mirroring this result. */
	activityId?: string;
	error?: {
		code: DeviceErrorCode;
		/** The device's own sentence (device-authored, shown quoted and untranslated). */
		reason?: string;
		/** A hub or local failure sentence, for details. */
		detail?: string;
	};
	/** Epoch milliseconds. */
	at: number;
}

interface InlineResults {
	list(): InlineResultEntry[];
	put(entry: InlineResultEntry): void;
	dismiss(id: string): void;
	clear(scopeKey: string): void;
	subscribe(listener: () => void): () => void;
}

const MAX_RESULTS = 50;
const stores = new WeakMap<DeviceWorkspace, InlineResults>();

function createInlineResults(): InlineResults {
	let entries: InlineResultEntry[] = [];
	const listeners = new Set<() => void>();
	const set = (next: InlineResultEntry[]) => {
		entries = next.slice(-MAX_RESULTS);
		for (const listener of [...listeners]) listener();
	};
	return {
		list: () => entries,
		put(entry) {
			const known = entries.some((row) => row.id === entry.id);
			set(
				known
					? entries.map((row) => (row.id === entry.id ? entry : row))
					: [...entries, entry],
			);
		},
		dismiss: (id) => set(entries.filter((row) => row.id !== id)),
		clear: (scopeKey) =>
			set(entries.filter((row) => row.scopeKey !== scopeKey)),
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
	};
}

/** Results live with the workspace, so they survive tab and route changes inside the area. */
export function inlineResultsOf(workspace: DeviceWorkspace): InlineResults {
	let store = stores.get(workspace);
	if (!store) {
		store = createInlineResults();
		stores.set(workspace, store);
	}
	return store;
}

const RESULT_TONE: Record<InlineResultState, ResultTone> = {
	running: "info",
	done: "good",
	failed: "critical",
	rejected: "critical",
	unknown: "unknown",
};

type ResultCopy = (t: DevicesT, entry: InlineResultEntry) => string;

/** A device's own sentence is quoted as it came; everything else is translated from its code. */
const rejectedText: ResultCopy = (t, { label, error }) =>
	error?.reason
		? t(
				"devices:action.result.rejectedReason",
				"{{label}} was refused by the device: “{{reason}}”",
				{ label, reason: error.reason },
			)
		: t("devices:action.result.rejected", "{{label}} didn't run. {{why}}", {
				label,
				why: errorCopy(t, error?.code ?? "rejected"),
			});

const failedText: ResultCopy = (t, { label, error }) =>
	t("devices:action.result.failed", "{{label}} didn't run. {{why}}", {
		label,
		why: error?.detail ?? errorCopy(t, error?.code ?? "connection_failed"),
	});

const RESULT_TEXT: Record<InlineResultState, ResultCopy> = {
	running: (t, { label }) =>
		t("devices:action.result.running", "{{label}}: in progress…", { label }),
	done: (t, { label }) =>
		t("devices:action.result.done", "{{label}}: done.", { label }),
	unknown: (t, { label }) =>
		t(
			"devices:action.result.unknown",
			"{{label}}: no reply received. It may or may not have run on the device.",
			{ label },
		),
	rejected: rejectedText,
	failed: failedText,
};

export interface InlineResultView extends InlineResultEntry {
	tone: ResultTone;
	/** The sentence to render inside `InlineResult`. */
	text: string;
	/** The tray item behind it, while the tray still holds it. */
	activity?: ActivityItem;
	dismiss(): void;
}

function mirrored(
	entry: InlineResultEntry,
	item: ActivityItem | undefined,
): InlineResultEntry {
	if (!item || entry.state === "rejected" || entry.state === "failed")
		return entry;
	if (item.state === "unknown") return { ...entry, state: "unknown" };
	if (item.state === "failed") return { ...entry, state: "failed" };
	if (item.state === "done") return { ...entry, state: "done" };
	return entry.state === "done" ? { ...entry, state: "running" } : entry;
}

/**
 * The results of one control group, newest last, mirroring their tray items
 * (a tracked operation that finishes later updates the sentence in place).
 */
export function useInlineResults(scopeKey: string): InlineResultView[] {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const store = inlineResultsOf(workspace);
	const read = useCallback(() => store.list(), [store]);
	const entries = useManagerValue(store.subscribe, read);
	const { items } = useActivity();
	return useMemo(
		() =>
			entries
				.filter((entry) => entry.scopeKey === scopeKey)
				.map((entry) => {
					const activity = items.find((item) => item.id === entry.activityId);
					const current = mirrored(entry, activity);
					return {
						...current,
						tone: RESULT_TONE[current.state],
						text: RESULT_TEXT[current.state](t, current),
						...(activity ? { activity } : {}),
						dismiss: () => store.dismiss(entry.id),
					};
				}),
		[entries, items, scopeKey, store, t],
	);
}
