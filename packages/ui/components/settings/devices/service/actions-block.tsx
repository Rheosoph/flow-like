"use client";

import { useTranslation } from "@flow-like/locales";
import { Globe, Zap } from "lucide-react";
import { useMemo } from "react";
import type {
	GateResult,
	ServiceAction,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { RunNowButton } from "../app/run-now";
import { eventTypeLabel } from "../copy/eligibility-copy";
import { scheduleOutcome } from "../copy/schedule-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { type GateTarget, serviceGateExtra, useGate } from "../workspace";
import { gateLine } from "./config-parts";
import {
	type ListRun,
	appEventRows,
	countersFresh,
	listRun,
	reportedEntries,
	serviceEventIds,
} from "./service-events";
import type { ServiceApp } from "./service-header";

/* Service › Status: the quick actions and forms a person can run on the service, with Run now… and what the last runs did. */

export interface ActionRow {
	eventId: string;
	/** The event's name; undefined when the app no longer has it, or its names can't be read. */
	name?: string;
	/** `quick_action` or `generic_form`, when the app's events can be read. */
	eventType?: string;
}

/** The quick actions and forms of a service: what its process reported, and the events it serves that the app lists as started by a person. */
export function actionRows(
	service: Pick<ServiceView, "actions" | "events">,
	app: Pick<ServiceApp, "view">,
): ActionRow[] {
	const known = appEventRows(app);
	const ids = serviceEventIds(
		service,
		reportedEntries(service.actions),
		(id) => known.get(id)?.eligibility.kind === "on_demand",
	);
	return ids.map((eventId) => {
		const row = known.get(eventId);
		return {
			eventId,
			...(row?.name ? { name: row.name } : {}),
			...(row ? { eventType: row.eventType } : {}),
		};
	});
}

/** "Quick action", "Form · 3 fields"; the type alone before the device said how many fields it has. */
function kindText(
	t: DevicesT,
	row: ActionRow,
	entry: ServiceAction | undefined,
) {
	if (entry?.kind === "action")
		return t("devices:serviceStatus.actions.action", "Quick action");
	if (entry?.kind === "form")
		return t("devices:serviceStatus.actions.form", {
			count: entry.fields,
			defaultValue_one: "Form · {{count, number}} field",
			defaultValue_other: "Form · {{count, number}} fields",
		});
	return row.eventType ? eventTypeLabel(t, row.eventType) : null;
}

function unreportedLine(
	t: DevicesT,
	state: Exclude<ListRun<ServiceAction>["state"], "reported">,
): string {
	switch (state) {
		case "stopped":
			return t(
				"devices:serviceStatus.actions.stopped",
				"Can't be run while the service is not running.",
			);
		case "not_reported":
			return t(
				"devices:serviceStatus.actions.notReported",
				"Not reported yet.",
			);
		case "needs_agent":
			return t(
				"devices:serviceStatus.actions.needsAgent",
				"Update the device agent to run actions from here.",
			);
		case "unknown":
			return t(
				"devices:serviceStatus.actions.unknown",
				"Not in this status. Connect live to see its runs.",
			);
	}
}

/** The numbers a live row carries: the last run, runs going now, runs since the service started. */
function counterLines(t: DevicesT, entry: ServiceAction, time: AreaTime) {
	const lines: string[] = [];
	if ((entry.running ?? 0) > 0)
		lines.push(
			t("devices:serviceStatus.actions.running", {
				count: entry.running,
				defaultValue_one: "{{count, number}} run is going now.",
				defaultValue_other: "{{count, number}} runs are going now.",
			}),
		);
	if (typeof entry.last_at === "number" && entry.last_outcome)
		lines.push(
			t(
				"devices:serviceStatus.actions.last",
				"Last run {{time}} · {{outcome}}",
				{
					time: time.ago(entry.last_at, "short"),
					outcome: scheduleOutcome(t, entry.last_outcome),
				},
			),
		);
	if (typeof entry.runs === "number")
		lines.push(
			entry.runs === 0
				? t(
						"devices:serviceStatus.actions.noRuns",
						"No runs since the service started.",
					)
				: t("devices:serviceStatus.actions.runs", {
						count: entry.runs,
						failed: entry.failed ?? 0,
						defaultValue_one:
							"{{count, number}} run since the service started · {{failed, number}} failed",
						defaultValue_other:
							"{{count, number}} runs since the service started · {{failed, number}} failed",
					}),
		);
	return lines;
}

/**
 * Why "Run now…" is not offered in this row: a form that takes a file, or a
 * reason the row already says (the service is not running, its agent can't
 * run actions). null: offered, disabled with its reason while the gate is closed.
 */
function offState(run: ListRun<ServiceAction>): "file" | "said" | null {
	if (run.state === "reported" && run.entry.file_fields > 0) return "file";
	return run.state === "stopped" || run.state === "needs_agent" ? "said" : null;
}

/** "This form takes a file": the service page is the only door that can send one. */
function FileOnly({
	service,
	hasPage,
}: Readonly<{ service: ServiceView; hasPage: boolean }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (!hasPage)
		return (
			<p data-action-file="" className="text-ink-2">
				{t(
					"serviceStatus.actions.fileOnlyNoPage",
					"This form takes a file. Files can only be sent from a service page, and this service has none.",
				)}
			</p>
		);
	return (
		<p
			data-action-file=""
			className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ink-2"
		>
			<span>
				{t(
					"serviceStatus.actions.fileOnly",
					"This form takes a file. Open it on the service page.",
				)}
			</span>
			<DvButton asChild size="xs" icon={Globe} className="w-fit">
				<a
					{...link({
						screen: "service",
						deviceId: service.deviceId,
						serviceId: service.serviceId,
						tab: "endpoint",
					})}
				>
					{t("serviceStatus.actions.openPage", "Service page")}
				</a>
			</DvButton>
		</p>
	);
}

interface ItemProps {
	row: ActionRow;
	service: ServiceView;
	gate: GateResult;
	/** The service has a web endpoint, so it has a service page. */
	hasPage: boolean;
	onRunNow(eventId: string): void;
}

function ActionItem({ row, service, gate, hasPage, onRunNow }: ItemProps) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const locked = service.freshness.age === "locked";
	const run = listRun(service, service.actions, row.eventId);
	const entry = run.state === "reported" ? run.entry : undefined;
	const name = row.name ?? t("service.serves.removed", "A removed event");
	const kind = kindText(t, row, entry);
	const off = locked ? "said" : offState(run);
	const lines = locked
		? [t("view.matrix.locked", "Unknown until unlocked")]
		: run.state !== "reported"
			? [unreportedLine(t, run.state)]
			: countersFresh(service)
				? counterLines(t, run.entry, time)
				: [];
	return (
		<li
			data-action={row.eventId}
			data-action-state={locked ? "locked" : run.state}
			className="flex min-w-0 flex-col gap-1 border-t border-hairline py-2.5 text-ui first:border-t-0 first:pt-0 last:pb-0"
		>
			<div className="flex min-w-0 flex-wrap items-start justify-between gap-x-3 gap-y-1.5">
				<p className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
					<b className="font-semibold">{name}</b>
					{kind ? <span className="text-muted-foreground">{kind}</span> : null}
				</p>
				{off === null ? (
					<RunNowButton
						gate={gateLine(t, time, gate)}
						onOpen={() => onRunNow(row.eventId)}
						label={t("serviceStatus.actions.runNow", "Run now…")}
					/>
				) : null}
			</div>
			{lines.map((line) => (
				<p key={line} data-action-line="" className="text-ink-2">
					{line}
				</p>
			))}
			{off === "file" ? <FileOnly service={service} hasPage={hasPage} /> : null}
		</li>
	);
}

/** What `run_event` is checked against for this service. */
export function runEventTarget(service: ServiceView): GateTarget {
	return {
		placementId: service.serviceId,
		projectId: service.projectId,
		labels: { service: service.serviceId },
		extra: serviceGateExtra(service),
	};
}

/**
 * The quick actions and forms of the service: Run now… for each, or why it
 * can't be run from here, and what its last runs did. Counts and outcomes
 * only; no input and no result of anyone's run.
 */
export function ActionsBlock({
	service,
	app,
	hasPage,
	onRunNow,
}: Readonly<{
	service: ServiceView;
	app: ServiceApp;
	hasPage: boolean;
	onRunNow(eventId: string): void;
}>) {
	const { t } = useTranslation("devices");
	const rows = actionRows(service, app);
	const target = useMemo(() => runEventTarget(service), [service]);
	const gate = useGate("run_event", service.deviceId, target);
	if (!rows.length) return null;
	return (
		<Block
			id="service-actions"
			icon={Zap}
			title={t("serviceStatus.actions.title", "Actions and forms")}
			count={rows.length}
			stamp={<FreshnessStamp {...stampOf(service.freshness)} />}
			foot={t(
				"serviceStatus.actions.rule",
				"Everyone who may start {{service}} can run these from Devices.",
				{ service: service.serviceId },
			)}
		>
			<ul className="flex min-w-0 flex-col">
				{rows.map((row) => (
					<ActionItem
						key={row.eventId}
						row={row}
						service={service}
						gate={gate}
						hasPage={hasPage}
						onRunNow={onRunNow}
					/>
				))}
			</ul>
			{service.actionsTruncated ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceStatus.actions.truncated",
						"The device reported only some of this service's actions and forms.",
					)}
				</p>
			) : null}
		</Block>
	);
}
