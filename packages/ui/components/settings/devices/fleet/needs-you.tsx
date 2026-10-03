"use client";

import { useTranslation } from "@flow-like/locales";
import { TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
	type AttentionCounts,
	countAttention,
} from "../../../../lib/device-management/model/attention";
import {
	buildServiceViews,
	subjectDevice,
} from "../../../../lib/device-management/model/device-view";
import type {
	AttentionInput,
	AttentionItem,
} from "../../../../lib/device-management/model/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import {
	type AttentionDone,
	type AttentionEntry,
	AttentionList,
} from "../primitives/attention-list";
import { Block } from "../primitives/block";
import {
	FreshnessStamp,
	MixedSourcesStamp,
	type StampSpec,
	baseSource,
} from "../primitives/freshness-stamp";
import { useDevicesRoute } from "../routing/use-devices-route";
import { stampOf, useAttentionEntries } from "../shell/attention-popover";
import { useAttention, useAttentionState, useDeviceRows } from "../workspace";

/** R11: seven items, then "Show all N". */
export const NEEDS_YOU_CAP = 7;

export const NEEDS_YOU_ID = "devices-needs-you";

interface Seen {
	sentence: AttentionEntry["sentence"];
	conditionKey?: string;
	/** The device whose status was readable here when the item was last listed. */
	readFrom?: string;
}

interface Resolved extends Seen {
	id: string;
	/** Unix seconds this computer noticed the item was gone. */
	doneAt: number;
}

interface SessionMemory {
	/** Counted items the list showed last, by id. */
	seen: Map<string, Seen>;
	done: Resolved[];
}

/** What was resolved since the area opened; lives as long as the workspace (one account on one hub). */
const SESSIONS = new WeakMap<DeviceWorkspace, SessionMemory>();

function sessionOf(workspace: DeviceWorkspace) {
	let memory = SESSIONS.get(workspace);
	if (!memory) {
		memory = { seen: new Map(), done: [] };
		SESSIONS.set(workspace, memory);
	}
	return memory;
}

/** Whether this computer can read a device's status right now (keys open, a status to read). */
const statusReadable = (input: AttentionInput, deviceId: string) =>
	Array.isArray(buildServiceViews(deviceId, input));

interface Listed {
	entries: readonly AttentionEntry[];
	/** The model items behind `entries`. */
	items: readonly AttentionItem[];
	input: AttentionInput;
}

/** The counted items of the list by id (Info never counts, so it is never "done"). */
function countedById({ entries, items, input }: Listed) {
	const devices = new Map<string, string | undefined>();
	for (const item of items) devices.set(item.id, subjectDevice(item.subject));
	const open = new Map<string, Seen>();
	for (const entry of entries) {
		if (entry.severity === "info") continue;
		const deviceId = devices.get(entry.id);
		const seen: Seen = {
			sentence: entry.sentence,
			conditionKey: entry.conditionKey,
		};
		if (deviceId && statusReadable(input, deviceId)) seen.readFrom = deviceId;
		open.set(entry.id, seen);
	}
	return open;
}

interface Tracking {
	open: Map<string, Seen>;
	/** Snooze end per item id, unix seconds. */
	snoozed: Readonly<Record<string, number>>;
	nowS: number;
	readable(deviceId: string): boolean;
}

/**
 * Items that left the list since the last look, unless the person snoozed
 * them. An item of a device that was readable and no longer is (locked, keys
 * gone, access ended) only stopped being visible: nothing says it was resolved.
 */
function newlyResolved(memory: SessionMemory, tracking: Tracking) {
	const { open, snoozed, nowS, readable } = tracking;
	const resolved: Resolved[] = [];
	for (const [id, last] of memory.seen) {
		if (open.has(id)) continue;
		const snoozedUntil = snoozed[id];
		if (snoozedUntil !== undefined && snoozedUntil > nowS) continue;
		if (last.readFrom && !readable(last.readFrom)) continue;
		resolved.push({
			id,
			sentence: last.sentence,
			conditionKey: last.conditionKey,
			doneAt: nowS,
		});
	}
	return resolved;
}

/**
 * Brings the session's "done" rows up to date with the list. A condition that
 * comes back takes its done row away again. Returns whether anything changed.
 */
function trackResolved(memory: SessionMemory, tracking: Tracking) {
	const resolved = newlyResolved(memory, tracking);
	const kept: Resolved[] = [];
	for (const row of memory.done) if (!tracking.open.has(row.id)) kept.push(row);
	const changed = resolved.length > 0 || kept.length !== memory.done.length;
	memory.done = [...resolved, ...kept];
	memory.seen = tracking.open;
	return changed;
}

interface ResolvedSentenceProps {
	row: Resolved;
}

function ResolvedSentence({ row }: ResolvedSentenceProps) {
	const { t } = useTranslation("devices");
	return (
		<>
			<span className="font-medium text-foreground">
				{t("fleet.needsYou.resolved", "Resolved.")}
			</span>{" "}
			{row.sentence}
		</>
	);
}

function doneRow(row: Resolved, onDismiss: (id: string) => void) {
	const done: AttentionDone = {
		id: row.id,
		sentence: <ResolvedSentence row={row} />,
		doneAt: row.doneAt,
		conditionKey: row.conditionKey,
		onDismiss() {
			onDismiss(row.id);
		},
	};
	return done;
}

function useDoneInSession(
	items: readonly AttentionItem[],
	entries: readonly AttentionEntry[],
) {
	const { workspace, input, rows } = useAttentionState();
	const [, setVersion] = useState(0);
	const memory = sessionOf(workspace);
	// A list that hasn't loaded says nothing about what was resolved.
	const known = rows.data !== undefined;
	useEffect(() => {
		if (!known) return;
		const changed = trackResolved(memory, {
			open: countedById({ entries, items, input }),
			snoozed: workspace.attention.snoozed,
			nowS: input.now,
			readable: (deviceId) => statusReadable(input, deviceId),
		});
		if (changed) setVersion(bump);
	}, [memory, entries, items, input, known, workspace]);
	const dismiss = (id: string) => {
		const kept: Resolved[] = [];
		for (const row of memory.done) if (row.id !== id) kept.push(row);
		memory.done = kept;
		setVersion(bump);
	};
	const done: AttentionDone[] = [];
	for (const row of memory.done) done.push(doneRow(row, dismiss));
	return done;
}

const bump = (version: number) => version + 1;

/** The list's "Nothing needs you right now." line in the area's text size, not the app's paragraph size. */
const ALL_CLEAR_TEXT = "[&>p]:text-ui";

function summaryOf(t: DevicesT, counts: AttentionCounts) {
	if (counts.total === 0) return undefined;
	return [
		t("devices:fleet.needsYou.critical", "{{count, number}} critical", {
			count: counts.critical,
		}),
		t("devices:fleet.needsYou.warnings", {
			count: counts.warning,
			defaultValue_one: "{{count, number}} warning",
			defaultValue_other: "{{count, number}} warnings",
		}),
		t("devices:fleet.needsYou.notices", {
			count: counts.notice,
			defaultValue_one: "{{count, number}} notice",
			defaultValue_other: "{{count, number}} notices",
		}),
	].join(" · ");
}

const stampOfEntry = (entry: AttentionEntry) => entry.stamp;

interface HeadStampProps {
	/** The stamp most items share, or null when they differ. */
	base: StampSpec | null;
}

/** R5: the block head states the shared source once; a Hub source shows when the list was read. */
function HeadStamp({ base }: HeadStampProps) {
	const rows = useDeviceRows();
	if (!base) return <MixedSourcesStamp />;
	if (base.source === "hub")
		return <FreshnessStamp {...stampOf(rows.freshness)} />;
	return <FreshnessStamp {...base} />;
}

export interface NeedsYouProps {
	/** `focus=attention` in the URL: scroll the block into view once. */
	focus?: boolean;
}

/** SPEC §5.1 Needs you: every open item in tiers, seven at first, with what was done this session. */
export function NeedsYou({ focus = false }: NeedsYouProps) {
	const { t } = useTranslation("devices");
	const { navigate, clearParam } = useDevicesRoute();
	const items = useAttention();
	const entries = useAttentionEntries(items, { onNavigate: navigate });
	const done = useDoneInSession(items, entries);
	const [all, setAll] = useState(false);
	const counts = useMemo(() => countAttention(items), [items]);
	const base = useMemo(() => baseSource(entries.map(stampOfEntry)), [entries]);
	const block = useRef<HTMLDivElement>(null);
	const scrollToBlock = () => {
		block.current?.scrollIntoView?.({ block: "start" });
	};
	useEffect(() => {
		if (!focus) return;
		block.current?.scrollIntoView?.({ block: "start" });
		clearParam("focus");
	}, [focus, clearParam]);
	const showAll = () => setAll(true);
	const showFewer = () => {
		setAll(false);
		scrollToBlock();
	};

	return (
		<div ref={block} className="min-w-0 scroll-mt-4">
			<Block
				id={NEEDS_YOU_ID}
				icon={TriangleAlert}
				title={t("fleet.needsYou.title", "Needs you")}
				count={counts.total}
				summary={summaryOf(t, counts)}
				stamp={<HeadStamp base={base} />}
				flush
			>
				<AttentionList
					items={entries}
					cap={all ? undefined : NEEDS_YOU_CAP}
					done={done}
					base={base}
					onShowAll={showAll}
					expanded={all && entries.length > NEEDS_YOU_CAP}
					onShowFewer={showFewer}
					className={ALL_CLEAR_TEXT}
				/>
			</Block>
		</div>
	);
}
