"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleDot,
	CirclePause,
	RefreshCw,
	SlidersHorizontal,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { PersonChip } from "../primitives/person-chip";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { useGate, useInlineResults } from "../workspace";
import type { ReadersEdit, ReadersMode } from "./history-readers-sheet";
import { LiveDataState, livePhase } from "./live-state";
import { TABLE_RESET } from "./observe-data";
import { RulesExpiredNotice, rulesExpiredAt } from "./renew-rules";
import type { HistoryPauseReason } from "./timeline-model";
import {
	DEVICE_SCOPE,
	HISTORY_KINDS,
	type HistoryKind,
	type HistoryRead,
	type HistoryStream,
	type Recording,
	historyResultKey,
	interimDigest,
	policyAppliedOf,
	recordingOf,
} from "./use-history";
import {
	type ObserveTarget,
	type PersonNames,
	gateLine,
	usePeople,
} from "./use-observe-target";

const SERVICE_ROWS = 8;
const READER_CHIPS = 3;

interface Row {
	key: string;
	scope: string;
	kinds: readonly HistoryKind[];
	streams: HistoryStream[];
	recording: Recording;
}

/** One row per readers list; a scope with neither list is one "Logs and metrics" row. */
export function historyRows(
	streams: readonly HistoryStream[],
	scopes: readonly string[],
	now: number,
	appliedDigest: string | null | undefined,
): Row[] {
	return scopes.flatMap((scope): Row[] => {
		const own = HISTORY_KINDS.flatMap((kind) => {
			const stream = streams.find(
				(row) => row.scope === scope && row.kind === kind,
			);
			return stream ? [stream] : [];
		});
		if (!own.length) return [];
		if (own.every((stream) => !stream.roster))
			return [
				{
					key: scope,
					scope,
					kinds: own.map((stream) => stream.kind),
					streams: own,
					recording: { state: "none" },
				},
			];
		return own.map((stream) => ({
			key: `${scope}|${stream.kind}`,
			scope,
			kinds: [stream.kind],
			streams: [stream],
			recording: recordingOf(stream, now, appliedDigest),
		}));
	});
}

/** "Paused: readers list expired": the short cause inside the chip. */
export function pausedLabel(
	t: DevicesT,
	reason: HistoryPauseReason | null,
): string {
	if (reason === "roster_expired")
		return enumLabel(t, "archiveRoster", "expired");
	if (reason === "rules_changed") return enumLabel(t, "archiveRoster", "stale");
	const labels: Record<
		Exclude<HistoryPauseReason, "roster_expired" | "rules_changed">,
		string
	> = {
		rules_expired: t(
			"devices:observe.history.paused.rulesExpired",
			"Paused: access rules expired",
		),
		quota_reached: t(
			"devices:observe.history.paused.quotaReached",
			"Paused: history storage is full",
		),
		tier_without_history: t(
			"devices:observe.history.paused.tierWithoutHistory",
			"Paused: your plan doesn't store history",
		),
		outbox_full: t(
			"devices:observe.history.paused.outboxFull",
			"Paused: uploads aren't getting through",
		),
	};
	return reason
		? labels[reason]
		: t("devices:observe.history.paused.unknown", "Paused");
}

export function RecordingChip({
	recording,
}: Readonly<{ recording: Recording }>) {
	const { t } = useTranslation("devices");
	if (recording.state === "none")
		return (
			<StatusChip tone="outline">
				{enumLabel(t, "archiveRoster", "none")}
			</StatusChip>
		);
	if (recording.state === "recording")
		return (
			<StatusChip tone="good" icon={CircleDot}>
				{enumLabel(t, "archiveRoster", "current")}
			</StatusChip>
		);
	return (
		<StatusChip tone="warning" icon={CirclePause}>
			{pausedLabel(t, recording.reason)}
		</StatusChip>
	);
}

function Readers({
	row,
	target,
	people,
}: Readonly<{ row: Row; target: ObserveTarget; people: PersonNames }>) {
	const { t } = useTranslation("devices");
	if (row.recording.state === "none")
		return <span className="text-muted-foreground">–</span>;
	if (
		row.recording.state === "paused" &&
		row.recording.reason === "roster_expired"
	)
		return (
			<span className="text-muted-foreground">
				{t("devices:observe.history.noneExpired", "none: the list expired")}
			</span>
		);
	const readers = row.streams[0]?.roster?.recipients ?? [];
	const users = [...new Set(readers.map((reader) => reader.user_id))];
	const shown = users.slice(0, READER_CHIPS);
	if (!users.length) return <span className="text-muted-foreground">–</span>;
	return (
		<span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
			{shown.map((user) => (
				<PersonChip
					key={user}
					you={user === target.me}
					name={
						people(user) ??
						(user === target.me
							? t("devices:observe.history.you", "You")
							: t(
									"devices:observe.history.someone",
									"Someone with shared access",
								))
					}
				/>
			))}
			{users.length > shown.length ? (
				<span className="text-xs text-muted-foreground">
					{t("devices:observe.history.moreReaders", "+{{count, number}} more", {
						count: users.length - shown.length,
					})}
				</span>
			) : null}
		</span>
	);
}

const MODE_OF: Record<Recording["state"], ReadersMode> = {
	none: "setup",
	recording: "change",
	paused: "resume",
};

export interface HistorySettingsProps {
	target: ObserveTarget;
	history: HistoryRead;
	/** Scopes to list, in order: the device first, then its services (or one service). */
	scopes: readonly string[];
	/** Services whose history isn't listed here (the list is capped). */
	hiddenServices?: number;
	onEdit(edit: ReadersEdit): void;
}

/** SPEC §5.2 "History settings" (owner): who can read what is retained, and whether it records. */
export function HistorySettings({
	target,
	history,
	scopes,
	hiddenServices = 0,
	onEdit,
}: Readonly<HistorySettingsProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [all, setAll] = useState(false);
	const gate = useGate("approve_history_readers", target.deviceId, {
		extra: { policyApplied: policyAppliedOf(target) },
	});
	const results = useInlineResults(historyResultKey(target.deviceId));
	const service = target.serviceId !== null;
	const now = Math.floor(time.nowS / 30) * 30;
	const rows = history.streams
		? historyRows(history.streams, scopes, now, interimDigest(target))
		: [];
	const people = usePeople(
		rows.flatMap((row) =>
			(row.streams[0]?.roster?.recipients ?? []).map(
				(reader) => reader.user_id,
			),
		),
	);
	const shown = all ? rows : rows.slice(0, SERVICE_ROWS + 2);
	const labels = {
		scope: t("devices:observe.history.col.scope", "Applies to"),
		kind: t("devices:observe.history.col.kind", "Kind"),
		recording: t("devices:observe.history.col.recording", "Recording"),
		readers: t("devices:observe.history.col.readers", "Readers"),
		expires: t("devices:observe.history.col.expires", "Expires"),
		action: t("devices:observe.history.col.action", "Action"),
	};
	const actions: Record<ReadersMode, string> = {
		setup: t("devices:observe.history.setUp", "Set up…"),
		change: t("devices:observe.history.changeReaders", "Change readers…"),
		resume: t("devices:observe.history.resume", "Resume recording…"),
	};
	// One reason blocks every row alike, so it is said once above the table and the buttons point to it (R7).
	const reasonId = useId();
	const expiredAt = rulesExpiredAt(target, now);
	const refused = expiredAt === undefined ? gateLine(t, gate, time) : null;
	const blockedBy = expiredAt === undefined ? refused?.kind : "policy";
	const projectOf = (scope: string) =>
		target.services?.find((row) => row.serviceId === scope)?.projectId ?? null;

	const expires = (recording: Recording): ReactNode => {
		if (recording.state === "none")
			return <span className="text-muted-foreground">–</span>;
		return recording.until <= time.nowS
			? t("devices:observe.history.expired", "expired {{at}}", {
					at: time.at(recording.until),
				})
			: time.at(recording.until);
	};

	const phase = livePhase(target);
	let body: ReactNode;
	if (rows.length)
		body = (
			<>
				{expiredAt === undefined ? null : (
					<div id={reasonId} className="border-b border-hairline px-4 py-3">
						<RulesExpiredNotice target={target} expiredAt={expiredAt} />
					</div>
				)}
				{refused ? (
					<div className="border-b border-hairline px-4 py-2.5">
						<GateInline
							kind={refused.kind}
							id={reasonId}
							className="max-w-none"
						>
							{refused.reason}
						</GateInline>
					</div>
				) : null}
				<DvTable
					className={TABLE_RESET}
					label={
						service
							? t(
									"devices:observe.history.tableService",
									"History for {{service}}",
									{
										service: target.serviceId ?? "",
									},
								)
							: t("devices:observe.history.table", "History settings")
					}
					cols={
						service
							? ["22%", "24%", "20%", "16%", "18%"]
							: ["17%", "13%", "22%", "20%", "12%", "16%"]
					}
					head={
						<tr>
							{service ? null : <Th>{labels.scope}</Th>}
							<Th>{labels.kind}</Th>
							<Th>{labels.recording}</Th>
							<Th>{labels.readers}</Th>
							<Th>{labels.expires}</Th>
							<Th>{labels.action}</Th>
						</tr>
					}
				>
					{shown.map((row) => {
						const mode = MODE_OF[row.recording.state];
						return (
							<Tr key={row.key} data-history-row={row.key}>
								{service ? null : (
									<Td label={labels.scope}>
										{row.scope === DEVICE_SCOPE ? (
											t("devices:observe.history.wholeDevice", "Whole device")
										) : (
											<span className="font-mono">{row.scope}</span>
										)}
									</Td>
								)}
								<Td label={labels.kind}>
									{row.kinds.length > 1
										? t("devices:observe.history.bothKinds", "Logs and metrics")
										: enumLabel(t, "archiveKind", row.kinds[0] ?? "logs")}
								</Td>
								<Td label={labels.recording}>
									<RecordingChip recording={row.recording} />
									{row.recording.state === "paused" &&
									row.recording.since !== undefined ? (
										<CellSub>
											{t("devices:observe.history.since", "since {{at}}", {
												at: time.at(row.recording.since),
											})}
										</CellSub>
									) : null}
								</Td>
								<Td label={labels.readers}>
									<Readers row={row} target={target} people={people} />
								</Td>
								<Td label={labels.expires}>{expires(row.recording)}</Td>
								<Td label={labels.action} kind="act">
									<DvButton
										size="sm"
										{...(blockedBy
											? {
													"aria-disabled": true,
													"aria-describedby": reasonId,
													"data-gated": blockedBy,
												}
											: {})}
										onClick={() => {
											if (blockedBy) return;
											onEdit({
												scope: row.scope,
												kinds: row.kinds,
												projectId: projectOf(row.scope),
												mode,
												streams: row.streams,
											});
										}}
									>
										{actions[mode]}
									</DvButton>
								</Td>
							</Tr>
						);
					})}
				</DvTable>
				{rows.length > shown.length ? (
					<div className="border-t border-hairline px-4 py-2">
						<DvButton size="sm" variant="ghost" onClick={() => setAll(true)}>
							{t(
								"devices:observe.history.showAll",
								"Show {{count, number}} more",
								{
									count: rows.length - shown.length,
								},
							)}
						</DvButton>
					</div>
				) : null}
				{results.length ? (
					<div className="flex flex-col gap-2 border-t border-hairline px-4 py-3">
						{results.map((result) => (
							<InlineResult
								key={result.id}
								tone={result.tone}
								onDismiss={result.dismiss}
							>
								{result.text}
							</InlineResult>
						))}
					</div>
				) : null}
			</>
		);
	else if (phase !== "open")
		body = <LiveDataState target={target} what="history" />;
	else if (history.failed)
		body = (
			<StateView
				kind="error"
				title={t(
					"devices:observe.history.failed",
					"The history settings of {{device}} couldn't be read",
					{ device: target.name },
				)}
				actions={
					<DvButton
						size="sm"
						icon={RefreshCw}
						busy={history.reading}
						onClick={() => void history.refresh()}
					>
						{t("devices:observe.history.retry", "Try again")}
					</DvButton>
				}
			/>
		);
	else
		body = (
			<StateView
				kind="loading"
				title={t(
					"devices:observe.history.reading",
					"Reading history settings…",
				)}
			/>
		);

	return (
		<Block
			id="observe-history-settings"
			icon={SlidersHorizontal}
			title={
				service
					? t(
							"devices:observe.history.titleService",
							"History for this service",
						)
					: t("devices:observe.history.title", "History settings")
			}
			stamp={
				history.readAt === undefined ? (
					<FreshnessStamp source="live" age="notloaded" />
				) : (
					<FreshnessStamp
						source="live"
						age="live"
						observedAt={history.readAt}
						cadenceSec={60}
					/>
				)
			}
			flush={rows.length > 0}
			foot={
				<span>
					{service
						? t(
								"devices:observe.history.footService",
								"Retained history for {{service}} is encrypted for the readers you pick. Device-wide history is on {{device}}'s page.",
								{ service: target.serviceId ?? "", device: target.name },
							)
						: t(
								"devices:observe.history.foot",
								"Readers can decrypt the retained records. Changing the list replaces it and applies to future records only.",
							)}
					{hiddenServices > 0
						? ` ${t("devices:observe.history.hiddenServices", {
								count: hiddenServices,
								defaultValue_one:
									"History of {{count, number}} more service is set on its own page.",
								defaultValue_other:
									"History of {{count, number}} more services is set on their own pages.",
							})}`
						: null}
				</span>
			}
		>
			{body}
		</Block>
	);
}
