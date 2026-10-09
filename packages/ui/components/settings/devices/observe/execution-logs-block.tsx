"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeft, Download, ListChecks, RefreshCw } from "lucide-react";
import { useCallback, useId, useState } from "react";
import {
	type DeviceExecutionRun,
	readExecutionLogs,
	readExecutionRuns,
} from "../../../../lib/device-management/execution-logs";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvInput, Field } from "../primitives/form-fields";
import { GateNotice } from "../primitives/gate-notice";
import { StateView } from "../primitives/state-view";
import { StatusChip } from "../primitives/status-chip";
import { useGate } from "../workspace";
import { useDeviceCall } from "../workspace/use-live";
import { LiveDataState, livePhase } from "./live-state";
import { downloadText, fileSlug } from "./observe-data";
import { SmallSelect } from "./small-select";
import { useExecutionPage } from "./use-execution-page";
import { type ObserveTarget, readRefusal } from "./use-observe-target";

const FEATURES = { execution_logs: 1 } as const;
const runIdentity = (run: DeviceExecutionRun) => run.run_id;

function levelLabel(t: DevicesT, level: number) {
	return [
		t("observe.executions.debug", "Debug"),
		t("observe.executions.info", "Info"),
		t("observe.executions.warning", "Warning"),
		t("observe.executions.error", "Error"),
		t("observe.executions.fatal", "Fatal"),
	][level];
}

function Unsupported() {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="unsupported"
			title={t(
				"observe.executions.unsupported",
				"Update the device agent to read execution logs",
			)}
			text={t(
				"observe.executions.unsupportedText",
				"This agent does not expose stored workflow executions.",
			)}
		/>
	);
}

function PageLimit() {
	const { t } = useTranslation("devices");
	return (
		<p className="text-xs text-muted-foreground">
			{t(
				"observe.executions.limit",
				"The device's paging limit has been reached. More entries remain in its local log store.",
			)}
		</p>
	);
}

function ReadError({
	error,
	retry,
}: Readonly<{ error: string; retry(): void }>) {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="error"
			title={t("observe.executions.failed", "Execution logs could not be read")}
			text={error}
			actions={
				<DvButton size="sm" onClick={retry}>
					{t("observe.executions.retry", "Try again")}
				</DvButton>
			}
		/>
	);
}

function RunLogs({
	deviceId,
	serviceId,
	run,
}: Readonly<{
	deviceId: string;
	serviceId: string;
	run: DeviceExecutionRun;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const call = useDeviceCall(deviceId);
	const fieldId = useId();
	const [nodeDraft, setNodeDraft] = useState("");
	const [nodeId, setNodeId] = useState("");
	const [minLevel, setMinLevel] = useState("0");
	const read = useCallback(
		async (offset: number) => {
			const answer = await readExecutionLogs(call, FEATURES, {
				placementId: serviceId,
				runId: run.run_id,
				offset,
				nodeId: nodeId || undefined,
				minLevel: Number(minLevel),
			});
			return answer.kind === "ok"
				? {
						kind: "ok" as const,
						data: {
							rows: answer.data.logs,
							next: answer.data.next_offset,
							limitReached: answer.data.limit_reached,
						},
					}
				: answer;
		},
		[call, serviceId, run.run_id, nodeId, minLevel],
	);
	const page = useExecutionPage(read);
	const nodeValid =
		!nodeDraft.trim() || /^[A-Za-z0-9_:.-]{1,128}$/u.test(nodeDraft.trim());

	const download = () =>
		downloadText(
			`${fileSlug(serviceId)}-${fileSlug(run.run_id)}.log`,
			page.rows
				.map((log) =>
					[
						time.abs(log.start / 1_000_000),
						levelLabel(t, log.log_level),
						log.node_id ?? "",
						log.operation_id ?? "",
						log.message,
						log.truncated
							? t(
									"observe.executions.truncated",
									"Message shortened by the device.",
								)
							: "",
					]
						.filter(Boolean)
						.join("\t"),
				)
				.join("\n"),
		);

	return (
		<div
			className="flex min-w-0 flex-col gap-3"
			data-execution-detail={run.run_id}
		>
			<div className="flex flex-wrap items-end gap-3">
				<form
					className="flex min-w-0 flex-1 items-end gap-2"
					onSubmit={(event) => {
						event.preventDefault();
						if (nodeValid) setNodeId(nodeDraft.trim());
					}}
				>
					<Field
						id={fieldId}
						className="min-w-0 flex-1"
						label={t("observe.executions.node", "Node ID")}
						error={
							nodeValid
								? undefined
								: t("observe.executions.invalidNode", "Enter a valid node ID.")
						}
					>
						<DvInput
							mono
							value={nodeDraft}
							maxLength={128}
							placeholder={t("observe.executions.allNodes", "All nodes")}
							onChange={(event) => setNodeDraft(event.target.value)}
						/>
					</Field>
					<DvButton type="submit" size="sm" disabled={!nodeValid}>
						{t("observe.executions.filter", "Filter")}
					</DvButton>
				</form>
				<SmallSelect
					label={t("observe.executions.level", "Minimum log level")}
					value={minLevel}
					onChange={setMinLevel}
					options={[
						{
							value: "0",
							label: t("observe.executions.allLevels", "All levels"),
						},
						{
							value: "1",
							label: t("observe.executions.infoAbove", "Info and above"),
						},
						{
							value: "2",
							label: t(
								"observe.executions.warningsAbove",
								"Warnings and above",
							),
						},
						{
							value: "3",
							label: t("observe.executions.errorsAbove", "Errors and above"),
						},
					]}
				/>
			</div>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					size="xs"
					variant="ghost"
					icon={RefreshCw}
					busy={page.busy}
					onClick={page.refresh}
				>
					{t("observe.executions.refreshLogs", "Refresh logs")}
				</DvButton>
				{page.rows.length ? (
					<DvButton
						size="xs"
						variant="ghost"
						icon={Download}
						onClick={download}
					>
						{t("observe.executions.download", "Download loaded logs")}
					</DvButton>
				) : null}
			</div>
			{page.unsupported ? (
				<Unsupported />
			) : page.error ? (
				<ReadError error={page.error} retry={page.refresh} />
			) : page.busy && !page.rows.length ? (
				<StateView
					kind="loading"
					title={t("observe.executions.readingLogs", "Reading execution logs…")}
				/>
			) : !page.rows.length ? (
				<p className="text-ui text-muted-foreground">
					{t(
						"observe.executions.noLogs",
						"No stored log entries match these filters.",
					)}
				</p>
			) : (
				<ol
					className="m-0 flex max-h-[36rem] min-w-0 list-none flex-col overflow-y-auto p-0"
					aria-label={t("observe.executions.entries", "Execution log entries")}
				>
					{page.rows.map((log, index) => (
						<li
							// Entries can share timestamps and operation IDs; the page position identifies each record.
							// biome-ignore lint/suspicious/noArrayIndexKey: records remain in their fetched order
							key={index}
							className="min-w-0 border-t border-hairline py-3 first:border-t-0"
							data-execution-log=""
						>
							<div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
								<time title={time.abs(log.start / 1_000_000)}>
									{time.clock(log.start / 1_000_000)}
								</time>
								<StatusChip
									tone={
										log.log_level >= 3
											? "critical"
											: log.log_level === 2
												? "warning"
												: "outline"
									}
								>
									{levelLabel(t, log.log_level)}
								</StatusChip>
								{log.node_id ? (
									<span className="break-all font-mono">
										{t("observe.executions.nodeValue", "Node {{node}}", {
											node: log.node_id,
										})}
									</span>
								) : null}
								{log.operation_id ? (
									<span className="break-all font-mono">
										{t(
											"observe.executions.operationValue",
											"Operation {{operation}}",
											{ operation: log.operation_id },
										)}
									</span>
								) : null}
							</div>
							<pre className="m-0 mt-2 whitespace-pre-wrap break-words font-mono text-xs">
								{log.message}
							</pre>
							{log.truncated ? (
								<p className="mt-1 text-xs text-muted-foreground">
									{t(
										"observe.executions.truncated",
										"Message shortened by the device.",
									)}
								</p>
							) : null}
						</li>
					))}
				</ol>
			)}
			{page.limitReached ? <PageLimit /> : null}
			{page.next !== null ? (
				<div>
					<DvButton
						size="sm"
						variant="ghost"
						busy={page.busy}
						onClick={page.loadMore}
					>
						{t("observe.executions.moreLogs", "Load more log entries")}
					</DvButton>
				</div>
			) : null}
		</div>
	);
}

function Executions({
	deviceId,
	serviceId,
}: Readonly<{ deviceId: string; serviceId: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const call = useDeviceCall(deviceId);
	const [selected, setSelected] = useState<DeviceExecutionRun | null>(null);
	const read = useCallback(
		async (offset: number) => {
			const answer = await readExecutionRuns(call, FEATURES, {
				placementId: serviceId,
				offset,
			});
			return answer.kind === "ok"
				? {
						kind: "ok" as const,
						data: {
							rows: answer.data.runs,
							next: answer.data.next_offset,
							limitReached: answer.data.limit_reached,
						},
					}
				: answer;
		},
		[call, serviceId],
	);
	const page = useExecutionPage(read, runIdentity);

	if (selected)
		return (
			<div className="flex min-w-0 flex-col gap-3">
				<div>
					<DvButton
						size="sm"
						variant="ghost"
						icon={ArrowLeft}
						onClick={() => setSelected(null)}
					>
						{t("observe.executions.back", "All executions")}
					</DvButton>
				</div>
				<div className="flex min-w-0 flex-col gap-1 text-ui">
					<strong>{time.abs(selected.start / 1_000_000)}</strong>
					<span className="break-all font-mono text-xs">
						{t("observe.executions.runValue", "Run {{run}}", {
							run: selected.run_id,
						})}
					</span>
					<span className="break-all text-xs text-muted-foreground">
						{t(
							"observe.executions.flowVersion",
							"Flow {{flow}}, version {{version}}",
							{ flow: selected.board_id, version: selected.version },
						)}
					</span>
				</div>
				<RunLogs
					key={selected.run_id}
					deviceId={deviceId}
					serviceId={serviceId}
					run={selected}
				/>
			</div>
		);

	return (
		<div className="flex min-w-0 flex-col gap-3">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<p className="text-xs text-muted-foreground">
					{t(
						"observe.executions.hint",
						"Finished executions stored on this device. Choose a run to inspect its node logs. Runs appear after they finish; older agents did not record this history.",
					)}
				</p>
				<DvButton
					size="xs"
					variant="ghost"
					icon={RefreshCw}
					busy={page.busy}
					onClick={page.refresh}
				>
					{t("observe.executions.refresh", "Refresh executions")}
				</DvButton>
			</div>
			{page.unsupported ? (
				<Unsupported />
			) : page.error ? (
				<ReadError error={page.error} retry={page.refresh} />
			) : page.busy && !page.rows.length ? (
				<StateView
					kind="loading"
					title={t("observe.executions.reading", "Reading executions…")}
				/>
			) : !page.rows.length ? (
				<p className="text-ui text-muted-foreground">
					{t(
						"observe.executions.empty",
						"No finished executions have been recorded for this service yet.",
					)}
				</p>
			) : (
				<ul
					className="m-0 flex list-none flex-col p-0"
					aria-label={t("observe.executions.list", "Workflow executions")}
				>
					{page.rows.map((run) => (
						<li
							key={run.run_id}
							className="border-t border-hairline first:border-t-0"
						>
							<button
								type="button"
								data-execution-run={run.run_id}
								onClick={() => setSelected(run)}
								className="flex w-full min-w-0 flex-col gap-1 rounded px-2 py-3 text-left hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-ring"
								aria-label={t(
									"observe.executions.inspect",
									"Inspect execution {{run}}",
									{ run: run.run_id },
								)}
							>
								<span className="flex flex-wrap items-center gap-2 text-ui">
									<span>{time.abs(run.start / 1_000_000)}</span>
									{run.log_level >= 2 ? (
										<StatusChip
											tone={run.log_level >= 3 ? "critical" : "warning"}
											title={t(
												"observe.executions.highestLevel",
												"Highest log level: {{level}}",
												{ level: levelLabel(t, run.log_level) },
											)}
										>
											{levelLabel(t, run.log_level)}
										</StatusChip>
									) : null}
									<span className="text-xs text-muted-foreground">
										{t("observe.executions.duration", "{{duration}} ms", {
											duration: Math.max(
												0,
												(run.end - run.start) / 1000,
											).toLocaleString(time.locale, {
												maximumFractionDigits: 1,
											}),
										})}
									</span>
								</span>
								<span className="break-all text-xs text-muted-foreground">
									{t(
										"observe.executions.flowVersion",
										"Flow {{flow}}, version {{version}}",
										{ flow: run.board_id, version: run.version },
									)}
								</span>
								{run.event_id ? (
									<span className="break-all text-xs text-muted-foreground">
										{t("observe.executions.eventValue", "Event {{event}}", {
											event: run.event_id,
										})}
									</span>
								) : null}
								<span className="break-all font-mono text-xs text-muted-foreground">
									{run.run_id}
								</span>
							</button>
						</li>
					))}
				</ul>
			)}
			{page.limitReached ? <PageLimit /> : null}
			{page.next !== null ? (
				<div>
					<DvButton
						size="sm"
						variant="ghost"
						busy={page.busy}
						onClick={page.loadMore}
					>
						{t("observe.executions.more", "Load older executions")}
					</DvButton>
				</div>
			) : null}
		</div>
	);
}

export function ExecutionLogsBlock({
	target,
}: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const gate = useGate("logs", target.deviceId, {
		placementId: target.serviceId ?? undefined,
		projectId: target.service?.projectId,
	});
	if (!target.serviceId) return null;
	const refusal = readRefusal(t, target, gate, "logs", {
		serviceId: target.serviceId,
		projectId: target.service?.projectId,
	});
	return (
		<Block
			id="observe-executions"
			icon={ListChecks}
			title={t("observe.executions.title", "Workflow executions")}
		>
			{refusal ? (
				<GateNotice
					kind="noaccess"
					title={t(
						"observe.executions.noAccess",
						"No access to execution logs.",
					)}
					text={refusal}
				/>
			) : livePhase(target) !== "open" ? (
				<LiveDataState target={target} what="logs" />
			) : target.features?.execution_logs !== 1 ? (
				<Unsupported />
			) : (
				<Executions
					key={`${target.deviceId}/${target.serviceId}`}
					deviceId={target.deviceId}
					serviceId={target.serviceId}
				/>
			)}
		</Block>
	);
}
