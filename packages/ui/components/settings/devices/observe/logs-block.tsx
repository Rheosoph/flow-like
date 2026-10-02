"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { ScrollText } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateNotice } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import {
	type LogRecord,
	type LogScope,
	LogViewer,
} from "../primitives/log-viewer";
import { StateView } from "../primitives/state-view";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { type LiveStreamView, useGate, useLiveStream } from "../workspace";
import { LiveDataState, livePhase, liveWanted } from "./live-state";
import {
	downloadText,
	fileSlug,
	lastGap,
	logRecords,
	recordsOf,
} from "./observe-data";
import { SmallSelect } from "./small-select";
import { type ObserveTarget, readRefusal } from "./use-observe-target";
import { useOlderRecords } from "./use-older-records";

const AGENT = "agent";
const LINK =
	"text-foreground underline decoration-border-strong underline-offset-2 hover:decoration-foreground";

function logFile(records: readonly LogRecord[], clock: (at: number) => string) {
	return records
		.flatMap((record) =>
			record.kind === "line"
				? [
						`${clock(record.at)} ${record.stream === "stderr" ? "ERR" : "OUT"} ${record.message}`,
					]
				: [],
		)
		.join("\n");
}

/** Keeps the lines on screen for the render in which a changed spec (pause, follow) hasn't reported yet. */
function useHeld<T>(stream: LiveStreamView<T>, key: string): LiveStreamView<T> {
	const held = useRef<{ key: string; view: LiveStreamView<T> }>(undefined);
	useEffect(() => {
		if (stream.started) held.current = { key, view: stream };
	}, [stream, key]);
	const previous = held.current;
	return !stream.started && previous?.key === key
		? { ...previous.view, loadOlder: stream.loadOlder, refresh: stream.refresh }
		: stream;
}

export interface LogsBlockProps {
	target: ObserveTarget;
	/** `stream=errors`: Errors only is preselected and the block scrolls into view. */
	errorsFirst?: boolean;
}

export function LogsBlock({
	target,
	errorsFirst = false,
}: Readonly<LogsBlockProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const { deviceId, serviceId, name } = target;
	const [picked, setPicked] = useState(AGENT);
	const [follow, setFollow] = useState(true);
	const [errorsOnly, setErrorsOnly] = useState(errorsFirst);
	const [saved, setSaved] = useState<{ file: string; lines: number } | null>(
		null,
	);
	const blockRef = useRef<HTMLDivElement>(null);

	const scopes = useMemo<LogScope[]>(
		() =>
			serviceId
				? []
				: [
						{ value: AGENT, label: t("observe.logs.agent", "Agent") },
						...(target.services ?? []).map((service) => ({
							value: service.serviceId,
							label: service.serviceId,
						})),
					],
		[serviceId, target.services, t],
	);
	const scope =
		serviceId ??
		(scopes.some((option) => option.value === picked) ? picked : AGENT);
	const placementId = scope === AGENT ? null : scope;
	const service = placementId
		? target.services?.find((row) => row.serviceId === placementId)
		: undefined;

	const gate = useGate(
		"logs",
		deviceId,
		placementId ? { placementId, projectId: service?.projectId } : undefined,
	);
	const refusal = readRefusal(
		t,
		target,
		gate,
		"logs",
		placementId
			? { serviceId: placementId, projectId: service?.projectId }
			: null,
		time,
	);
	const raw = useLiveStream<unknown>(
		deviceId,
		liveWanted(target) && !refusal
			? { kind: "logs", placementId, follow }
			: null,
	);
	const stream = useHeld(raw, `${deviceId}|${scope}`);

	useEffect(() => {
		if (errorsFirst)
			blockRef.current?.scrollIntoView?.({ block: "start", behavior: "auto" });
	}, [errorsFirst]);

	const wire = useMemo(() => recordsOf(stream.data), [stream.data]);
	const older = useOlderRecords(wire.length, stream.loadOlder);
	const evicted = older.available
		? undefined
		: lastGap(stream.gaps, "evicted_through")?.through;
	const records = useMemo(() => logRecords(wire, evicted), [wire, evicted]);

	const phase = livePhase(target);
	const title =
		scope === AGENT
			? t("observe.logs.agentTitle", "Agent logs")
			: t("observe.logs.serviceTitle", "Logs of {{service}}", {
					service: scope,
				});
	const behind = stream.behind ?? 0;
	const stampText = follow
		? stream.freshness.age === "live"
			? t("observe.logs.following", "following")
			: undefined
		: behind > 0
			? t("observe.logs.behind", "paused · {{count, number}} behind", {
					count: behind,
				})
			: t("observe.logs.paused", "paused");

	const download = () => {
		const shown = errorsOnly
			? records.filter(
					(record) => record.kind === "line" && record.stream === "stderr",
				)
			: records;
		const lines = shown.filter((record) => record.kind === "line").length;
		const day = new Date(time.now).toISOString().slice(0, 10);
		const file = `${fileSlug(name)}-${fileSlug(scope)}-${day}.log`;
		if (
			downloadText(
				file,
				logFile(shown, (at) => time.clock(at)),
			)
		)
			setSaved({ file, lines });
	};

	const note = serviceId ? (
		<Trans
			t={t}
			i18nKey="observe.logs.noteService"
			defaults="Output of {{service}}: Output is its standard output, Errors its standard error stream. Agent logs are on <1>{{device}} › Activity & logs</1>."
			values={{ service: serviceId, device: name }}
			components={{
				1: (
					<a
						className={LINK}
						{...link({ screen: "device", deviceId, tab: "activity" })}
					/>
				),
			}}
		/>
	) : scope === AGENT ? (
		t(
			"observe.logs.noteAgent",
			"Agent logs only. Service output is on each service's page, or pick a service above.",
		)
	) : (
		t(
			"observe.logs.notePicked",
			"Output of {{service}}. Errors are its standard error stream.",
			{ service: scope },
		)
	);

	const hasData = stream.data !== undefined;
	const body = refusal ? (
		<GateNotice
			kind="noaccess"
			title={t("observe.logs.noAccess", "No access to logs.")}
			text={refusal}
		/>
	) : stream.rejected && !hasData ? (
		<StateView
			kind={stream.freshness.age === "unsupported" ? "unsupported" : "noaccess"}
			title={t("observe.logs.rejected", "{{device}} didn't return these logs", {
				device: name,
			})}
			text={t(
				"observe.logs.rejectedText",
				"Reading logs needs Read logs on this device or service.",
			)}
		/>
	) : !hasData && phase !== "open" ? (
		<LiveDataState target={target} what="logs" />
	) : !hasData ? (
		<StateView
			kind="loading"
			title={t("observe.logs.reading", "Reading logs…")}
		/>
	) : (
		<LogViewer
			records={records}
			label={
				scope === AGENT
					? t("observe.logs.agentLabel", "Agent log lines")
					: t("observe.logs.serviceLabel", "Output of {{service}}", {
							service: scope,
						})
			}
			scopes={scopes.length > 1 ? scopes : undefined}
			scope={scope}
			onScopeChange={(next) => {
				setPicked(next);
				setSaved(null);
			}}
			errorsOnly={errorsOnly}
			onErrorsOnlyChange={setErrorsOnly}
			follow={follow}
			onFollowChange={setFollow}
			onLoadOlder={older.available ? older.load : undefined}
			loadingOlder={older.loading}
			onDownload={records.length ? download : undefined}
			stamp={
				<FreshnessStamp
					{...stampOf(stream.freshness)}
					{...(stampText ? { text: stampText } : {})}
				/>
			}
			result={
				saved ? (
					<InlineResult tone="good" onDismiss={() => setSaved(null)}>
						{t("observe.logs.saved", {
							count: saved.lines,
							file: saved.file,
							defaultValue_one: "Saved {{count, number}} line as {{file}}.",
							defaultValue_other: "Saved {{count, number}} lines as {{file}}.",
						})}
					</InlineResult>
				) : null
			}
			note={note}
			emptyText={
				scope === AGENT
					? t(
							"observe.logs.emptyAgent",
							"The agent hasn't logged anything yet.",
						)
					: t(
							"observe.logs.emptyService",
							"{{service}} hasn't written anything since it started.",
							{ service: scope },
						)
			}
		/>
	);

	// The viewer holds the source select; without a viewer (no access, nothing read) it moves to the head.
	const viewer = hasData && !refusal;
	return (
		<div ref={blockRef} className="min-w-0 scroll-mt-14">
			<Block
				id="observe-logs"
				icon={ScrollText}
				title={title}
				tools={
					!viewer && scopes.length > 1 ? (
						<SmallSelect
							value={scope}
							onChange={setPicked}
							label={t("observe.logs.source", "Log source")}
							options={scopes}
						/>
					) : null
				}
			>
				{body}
			</Block>
		</div>
	);
}
