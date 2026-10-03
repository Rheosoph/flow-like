"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import {
	Box,
	ChevronRight,
	Gauge,
	GitCompare,
	ListChecks,
	Rocket,
	Server,
	ShieldCheck,
	TriangleAlert,
	Zap,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";
import {
	DEPLOYMENT_CONFIG_BYTES,
	eventKind,
	readExistingDeployment,
} from "../../../../../lib/device-management/deployment";
import { deviceKeys } from "../../../../../lib/device-management/hub/queries";
import {
	type AppEventInput,
	appEventRule,
} from "../../../../../lib/device-management/model/app-plan";
import {
	type ConfigDiffRow,
	type DeployDraft,
	type DeployOrder,
	type DeployPlan,
	type DiffField,
	type PlanException,
	type PlanExceptionCode,
	type PlanTarget,
	type PlanTargetService,
	diffPlacementConfig,
	scheduleIds,
} from "../../../../../lib/device-management/model/deploy-plan";
import type { DeployRunState } from "../../../../../lib/device-management/model/deploy-run";
import type {
	CopyParams,
	DeployRoute,
	DeployStepId,
} from "../../../../../lib/device-management/model/types";
import { nextRuns } from "../../../../../lib/device-management/schedule";
import { formatEuroMicros } from "../../../../../lib/device-resources";
import { humanFileSize } from "../../../../../lib/utils";
import { agentTooOldCopy } from "../../copy/eligibility-copy";
import { gateCopy } from "../../copy/gate-copy";
import {
	scheduleTime,
	scheduleWordsInline,
	scheduleZone,
} from "../../copy/schedule-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../../primitives/consequence-preview";
import { type DiffRow, DiffRows } from "../../primitives/diff-rows";
import { DvButton } from "../../primitives/dv-button";
import { CellSub, DvTable, Td, Th, Tr } from "../../primitives/dv-table";
import {
	CheckField,
	ChoiceCards,
	Field,
	InputWithUnit,
	SwitchField,
} from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { GateInline } from "../../primitives/gate-notice";
import { IdRef } from "../../primitives/id-ref";
import { InlineResult } from "../../primitives/inline-result";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { Meter } from "../../primitives/meter";
import { Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { useDevicesRoute } from "../../routing/use-devices-route";
import {
	deviceCall,
	useDeviceWorkspace,
	useGate,
	useWidthBucket,
} from "../../workspace";
import { flowLabel } from "../deploy-copy";
import type { DeployStepProps } from "../step-props";
import {
	DEADLINE_RANGE,
	type ExistingRead,
	type ReviewTarget,
	type ReviewUnread,
	SAFE_UPDATE_TIMINGS,
	STABILIZE_RANGE,
	type SafeUpdateTimings,
	type WireProblem,
	deployTarget,
	reviewTargets,
	strategyReasonText,
	timingIssues,
	wantsAccess,
} from "../update-path";
import { useFlowNames } from "../use-deploy-reads";
import {
	deployRunExtras,
	deployRunTitleRef,
	deployWhat,
	setDeployRunExtras,
	useDeployRun,
	useLiveTargets,
	useWizardSlot,
} from "../use-deploy-run";
import { HEAD_CHIP } from "./copy-upload-step";

/* What each target would get comes from `reviewTargets` (APP §3.12); this reads the services the plan updates. */

interface UpdateRead {
	target: PlanTarget;
	service: PlanTargetService;
}

function updatesOf(plan: DeployPlan): UpdateRead[] {
	const updates: UpdateRead[] = [];
	for (const target of plan.targets) {
		for (const service of target.services) {
			if (service.kind !== "new") updates.push({ target, service });
		}
	}
	return updates;
}

interface ReadResult {
	data?: ExistingRead["data"];
	isError?: boolean;
}

function unreadOf(
	target: PlanTarget,
	result: ReadResult | undefined,
): ReviewUnread | undefined {
	if (target.locked) return "locked";
	if (result?.isError) return "failed";
	return result?.data ? undefined : "loading";
}

const targetOf = (update: UpdateRead) => update.target;

/** Reads the services this plan updates while their devices are unlocked. */
function useExistingServices(plan: DeployPlan): Map<string, ExistingRead> {
	const workspace = useDeviceWorkspace();
	const appId = plan.app?.id ?? plan.draft.appId ?? "";
	const updates = updatesOf(plan);
	useLiveTargets(updates.map(targetOf));
	const root = deviceKeys.root(workspace.scopeKey);
	const queries = updates.map(({ target, service }) => {
		const { deviceId } = target;
		const { serviceId } = service;
		function queryFn() {
			const call = deviceCall(workspace, deviceId, "user");
			return readExistingDeployment(call, serviceId, appId);
		}
		return {
			queryKey: [...root, "deploy-existing", deviceId, serviceId],
			queryFn,
			enabled: !target.locked && appId !== "",
			staleTime: 15_000,
			retry: false,
		};
	});
	const results = useQueries({ queries });
	const reads = new Map<string, ExistingRead>();
	updates.forEach(({ target, service }, index) => {
		const result = results[index];
		const key = deployTarget(target.deviceId, service.serviceId);
		reads.set(key, { data: result?.data, unread: unreadOf(target, result) });
	});
	return reads;
}

function readState(read: ExistingRead): string | number {
	if (read.unread) return read.unread;
	return read.data ? read.data.config_revision : "";
}

function readsSignature(reads: ReadonlyMap<string, ExistingRead>): string {
	const parts: string[] = [];
	reads.forEach(function part(read, key) {
		parts.push(`${key}:${readState(read)}`);
	});
	return parts.join("|");
}

function useReviewTargets(props: Readonly<DeployStepProps>): ReviewTarget[] {
	const { plan } = props;
	const stored = deployRunExtras(plan.draft.deploymentId).prepared;
	const bundle = props.prepared ?? stored;
	const reads = useExistingServices(plan);
	const signature = readsSignature(reads);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `signature` stands for the reads' content
	return useMemo(
		function targets() {
			return reviewTargets(plan, bundle, reads);
		},
		[plan, bundle, signature],
	);
}

/* Copy. */

function eventName(plan: DeployPlan, id: string): string {
	const events = plan.app ? plan.app.events : [];
	for (const event of events) {
		if (event.id === id) return event.name;
	}
	return id;
}

function eventNames(c: Context, ids: readonly string[]): string {
	const names = ids.map(function named(id) {
		return eventName(c.plan, id);
	});
	return c.list(names);
}

interface Context {
	t: DevicesT;
	time: AreaTime;
	plan: DeployPlan;
	list(values: readonly string[]): string;
}

function useReviewContext(plan: DeployPlan): Context {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return useMemo(() => {
		const format = new Intl.ListFormat(time.locale, {
			style: "long",
			type: "conjunction",
		});
		return { t, time, plan, list: (values) => format.format(values) };
	}, [t, time, plan]);
}

function deployButtonLabel(c: Context, items: readonly ReviewTarget[]): string {
	const { t, plan } = c;
	const [only] = plan.targets;
	const count = plan.targets.length;
	if (plan.draft.entry === "update")
		return count === 1
			? t("devices:deployShip.review.updateOne", "Update {{service}}", {
					service: items[0]?.service.serviceId ?? deployWhat(plan),
				})
			: t(
					"devices:deployShip.review.updateMany",
					"Update {{count, number}} services",
					{ count: items.length },
				);
	if (count === 1 && only)
		return t(
			"devices:deployShip.review.deployOne",
			"Deploy {{what}} to {{device}}",
			{ what: deployWhat(plan), device: only.name },
		);
	return t(
		"devices:deployShip.review.deployMany",
		"Deploy to {{count, number}} devices",
		{ count },
	);
}

const STEP_NAME: Record<DeployStepId, (t: DevicesT) => string> = {
	what: (t) => t("devices:deployShip.step.what", "What"),
	how: (t) => t("devices:deployShip.step.how", "How it runs"),
	where: (t) => t("devices:deployShip.step.where", "Where"),
	settings: (t) => t("devices:deployShip.step.settings", "Settings"),
	endpoint: (t) => t("devices:deployShip.step.endpoint", "Endpoint & limits"),
	access_cost: (t) => t("devices:deployShip.step.accessCost", "Access & cost"),
	copy_upload: (t) => t("devices:deployShip.step.copyUpload", "Copy & upload"),
	review: (t) => t("devices:deployShip.step.review", "Review"),
	rollout: (t) => t("devices:deployShip.step.rollout", "Rollout"),
};

/* Plan table (several targets). */

const EXCEPTION_COPY: Record<
	PlanExceptionCode,
	(c: Context, params: CopyParams, device: string) => string
> = {
	left_out_refuse: (c, p) =>
		c.t(
			"devices:deployShip.exception.refuse",
			"Leaves out {{event}}: the device can't run it",
			{ event: eventNames(c, [String(p.event)]) },
		),
	left_out_duplicate: (c, p) =>
		c.t(
			"devices:deployShip.exception.duplicate",
			"Leaves out {{event}}: {{service}} already serves it here",
			{ event: eventNames(c, [String(p.event)]), service: p.service },
		),
	left_out_agent: (c, p, device) =>
		c.t(
			"devices:deployShip.exception.agentFeature",
			"Leaves out {{event}}: {{reason}}",
			{
				event: eventNames(c, [String(p.event)]),
				reason: agentTooOldCopy(
					c.t,
					device,
					typeof p.feature === "string" ? p.feature : undefined,
				).long,
			},
		),
	once_soon: (c, p) =>
		c.t(
			"devices:deployShip.exception.onceSoon",
			"{{event}} runs in less than 5 minutes; it doesn't run if the deploy takes longer",
			{ event: eventNames(c, [String(p.event)]) },
		),
	bot_open: (c, p) =>
		c.t(
			"devices:deployShip.exception.botOpen",
			"Anyone who can message {{event}} starts runs here",
			{ event: eventNames(c, [String(p.event)]) },
		),
	bot_other_computers: (c, p) =>
		c.t(
			"devices:deployShip.exception.botOtherComputers",
			"Other computers may still run {{event}} in the desktop app",
			{ event: eventNames(c, [String(p.event)]) },
		),
	endpoint_shared_token: (c, p) =>
		c.t(
			"devices:deployShip.exception.endpointSharedToken",
			"{{event}} answers to {{service}}'s access token, not its own",
			{ event: eventNames(c, [String(p.event)]), service: p.service },
		),
	schedule_two_devices: (c, p) =>
		c.t(
			"devices:deployShip.exception.scheduleTwoDevices",
			"{{event}} runs on {{count, number}} devices, once on each",
			{ event: eventNames(c, [String(p.event)]), count: Number(p.count) },
		),
	schedule_local_trigger: (c, p) =>
		c.t(
			"devices:deployShip.exception.scheduleLocalTrigger",
			"This computer also runs {{event}}",
			{ event: eventNames(c, [String(p.event)]) },
		),
	renamed: (c, p) =>
		c.t(
			"devices:deployShip.exception.renamed",
			"Named {{to}} here: {{from}} is taken",
			{ to: p.to, from: p.from },
		),
	port_moved: (c, p) =>
		c.t(
			"devices:deployShip.exception.portMoved",
			"Port {{to}}: {{from}} is in use there",
			{ to: p.to, from: p.from },
		),
	no_certificate: (c) =>
		c.t("devices:deployShip.exception.unencrypted", "Serves unencrypted"),
	runs_as_agent: (c) =>
		c.t(
			"devices:deployShip.exception.agent",
			"Runs with the agent's full access",
		),
};

function planText(c: Context, item: ReviewTarget): string {
	const { t, plan } = c;
	if (item.service.kind === "new")
		return t("devices:deployShip.plan.newService", "New service");
	if (item.service.kind === "add")
		return t("devices:deployShip.plan.addEvents", "Add {{events}} to", {
			events: eventNames(c, item.service.events),
		});
	return plan.draft.version === "keep"
		? t("devices:deployShip.plan.updateSettings", "Update · settings only")
		: t("devices:deployShip.plan.updateNewest", "Update to the newest version");
}

function PlanCell({ c, item }: Readonly<{ c: Context; item: ReviewTarget }>) {
	return (
		<>
			{planText(c, item)}
			<CellSub className="font-mono">{item.service.serviceId}</CellSub>
		</>
	);
}

/** "0.0.0.0:8080", or only what is known while the service keeps its address. */
function addressOf(item: ReviewTarget): string {
	const hosting = item.existing?.config.hosting;
	const { endpoint } = item.target;
	const host = endpoint.host ?? hosting?.host ?? "";
	const port = item.service.port ?? endpoint.port ?? hosting?.port ?? "";
	return host ? `${host}:${port}` : `${port}`;
}

function EndpointCell({
	c,
	item,
}: Readonly<{ c: Context; item: ReviewTarget }>) {
	if (!item.hosted) return <>–</>;
	const { certificateId } = item.target.endpoint;
	const certificate =
		certificateId === undefined
			? item.existing?.config.tls_certificate_id
			: certificateId;
	const address = addressOf(item);
	const port = address.lastIndexOf(":");
	return (
		<>
			<span title={address} className="flex min-w-0 font-mono text-xs">
				<span className="truncate">
					{port > 0 ? address.slice(0, port) : address}
				</span>
				{port > 0 ? (
					<span className="shrink-0">{address.slice(port)}</span>
				) : null}
			</span>
			<CellSub>
				{certificate
					? c.t("devices:deployShip.plan.certificate", "with a certificate")
					: c.t("devices:deployShip.plan.plain", "unencrypted")}
			</CellSub>
		</>
	);
}

function spendOf(plan: DeployPlan, deviceId: string): number | undefined {
	const { spending } = plan.draft;
	if (!spending) return undefined;
	const own = plan.draft.targets.find((target) => target.deviceId === deviceId)
		?.over.spendingLimitMicros;
	return own ?? spending.limitMicros;
}

function accessCell(c: Context, item: ReviewTarget): string {
	const { t, plan } = c;
	if (item.service.kind !== "new")
		return t("devices:deployShip.plan.accessKept", "Kept");
	if (!wantsAccess(plan))
		return t("devices:deployShip.plan.accessNone", "None");
	const files = {
		none: t("devices:deployShip.plan.modelsOnly", "Model access"),
		read_only: t("devices:deployShip.plan.read", "Read"),
		read_write: t("devices:deployShip.plan.readWrite", "Read & write"),
	}[plan.mode === "online" ? plan.draft.approval.files : "none"];
	const spend = spendOf(plan, item.target.deviceId);
	return spend === undefined ? files : `${files} · ${formatEuroMicros(spend)}`;
}

function howCell(c: Context, item: ReviewTarget): string {
	const { t, plan } = c;
	if (item.service.kind === "new")
		return plan.draft.start
			? t("devices:deployShip.plan.starts", "Starts after deploy")
			: t("devices:deployShip.plan.staysStopped", "Stays stopped");
	const strategy = item.strategy;
	if (!strategy?.known)
		return t("devices:deployShip.plan.decidedLater", "Decided when unlocked");
	if (strategy.strategy === "safe")
		return t("devices:deployShip.strategy.safe", "Safe update");
	const [reason] = strategy.reasons;
	return reason
		? t("devices:deployShip.plan.quickBecause", "Quick update: {{reason}}", {
				reason: strategyReasonText(t, reason),
			})
		: t("devices:deployShip.strategy.quick", "Quick update");
}

const PLAN_COLS = ["18%", "20%", "17%", "18%", "13%", "14%"];
/** The warning edge: on the row's first cell, and on the card's edge once the table stacks (560 px). */
const WARNING_ROW =
	"[&>td:first-child]:border-l-2 [&>td:first-child]:border-l-warning-solid @max-[560px]/tbl:border-l-2! @max-[560px]/tbl:border-l-warning-solid! @max-[560px]/tbl:[&>td:first-child]:border-l-0";

function PlanRow({
	c,
	item,
	exceptions,
}: Readonly<{
	c: Context;
	item: ReviewTarget;
	exceptions: readonly PlanException[];
}>) {
	const { t } = c;
	const [open, setOpen] = useState(false);
	const label = {
		device: t("devices:deployShip.plan.colDevice", "Device"),
		plan: t("devices:deployShip.plan.colPlan", "Plan"),
		events: t("devices:deployShip.plan.colEvents", "Events"),
		endpoint: t("devices:deployShip.plan.colEndpoint", "Endpoint"),
		access: t("devices:deployShip.plan.colAccess", "Access"),
		how: t("devices:deployShip.plan.colHow", "How"),
	};
	return (
		<>
			<Tr
				data-tone={exceptions.length ? "warning" : undefined}
				className={exceptions.length ? WARNING_ROW : undefined}
			>
				<Td label={label.device} kind="mono" title={item.target.name}>
					{item.target.name}
					{exceptions.length ? (
						<DvButton
							variant="link"
							size="xs"
							icon={TriangleAlert}
							aria-expanded={open}
							className="flex font-sans text-warning"
							onClick={() => setOpen((value) => !value)}
						>
							{t("devices:deployShip.plan.differences", {
								count: exceptions.length,
								defaultValue_one: "{{count, number}} difference",
								defaultValue_other: "{{count, number}} differences",
							})}
						</DvButton>
					) : null}
				</Td>
				<Td label={label.plan}>
					<PlanCell c={c} item={item} />
				</Td>
				<Td label={label.events}>{eventNames(c, item.service.events)}</Td>
				<Td label={label.endpoint}>
					<EndpointCell c={c} item={item} />
				</Td>
				<Td label={label.access}>{accessCell(c, item)}</Td>
				<Td label={label.how}>{howCell(c, item)}</Td>
			</Tr>
			{open ? (
				<tr>
					<td colSpan={6} className="px-4 pb-2">
						<ul className="list-disc pl-5 text-ui text-ink-2">
							{exceptions.map((exception, index) => (
								<li key={`${exception.code}-${index}`}>
									{EXCEPTION_COPY[exception.code](
										c,
										exception.params ?? {},
										item.target.name,
									)}
								</li>
							))}
						</ul>
					</td>
				</tr>
			) : null}
		</>
	);
}

function PlanTable({
	c,
	items,
	exceptions,
}: Readonly<{
	c: Context;
	items: readonly ReviewTarget[];
	exceptions: readonly PlanException[];
}>) {
	const { t } = c;
	return (
		<DvTable
			label={t("devices:deployShip.plan.title", "Plan")}
			cols={PLAN_COLS}
			stackAt={560}
			head={
				<tr>
					<Th>{t("devices:deployShip.plan.colDevice", "Device")}</Th>
					<Th>{t("devices:deployShip.plan.colPlan", "Plan")}</Th>
					<Th>{t("devices:deployShip.plan.colEvents", "Events")}</Th>
					<Th>{t("devices:deployShip.plan.colEndpoint", "Endpoint")}</Th>
					<Th>{t("devices:deployShip.plan.colAccess", "Access")}</Th>
					<Th>{t("devices:deployShip.plan.colHow", "How")}</Th>
				</tr>
			}
		>
			{items.map((item) => (
				<PlanRow
					key={item.key}
					c={c}
					item={item}
					exceptions={exceptions.filter(
						(exception) => exception.deviceId === item.target.deviceId,
					)}
				/>
			))}
		</DvTable>
	);
}

/* One target: what changes, or the new service. */

const shortHash = (value: unknown) =>
	typeof value === "string" ? value.slice(0, 8) : "";

function plainValue(value: unknown): string {
	return typeof value === "string" ? value : JSON.stringify(value ?? null);
}

function isolationText(c: Context, value: unknown): string {
	const limits = value as {
		profile?: string;
		cpu_millis?: number;
		memory_bytes?: number;
	} | null;
	if (!limits || limits.profile !== "linux_sandbox")
		return c.t("devices:deployShip.diff.asAgent", "Runs as the agent");
	return c.t(
		"devices:deployShip.diff.sandboxed",
		"Sandboxed · {{cores}} cores · {{memory}}",
		{
			cores: ((limits.cpu_millis ?? 0) / 1000).toFixed(1),
			memory: humanFileSize(limits.memory_bytes ?? 0),
		},
	);
}

function bufferingText(c: Context, value: unknown): string {
	const writes = value as { tables?: unknown[]; files?: unknown[] } | null;
	return c.t(
		"devices:deployShip.diff.buffering",
		"{{tables, number}} tables, {{folders, number}} folders",
		{
			tables: writes?.tables?.length ?? 0,
			folders: writes?.files?.length ?? 0,
		},
	);
}

function eventText(c: Context, key: string | undefined, value: unknown) {
	const pins = value as {
		event_version?: number[];
		board_version?: number[];
	} | null;
	return c.t(
		"devices:deployShip.diff.eventPins",
		"{{event}} · {{version}} · flow {{flow}}",
		{
			event: eventNames(c, [key ?? ""]),
			version: pins?.event_version?.join(".") ?? "",
			flow: pins?.board_version?.join(".") ?? "",
		},
	);
}

interface DiffFormat {
	label(c: Context): string;
	value(c: Context, row: ConfigDiffRow, value: unknown): ReactNode;
}

const keyed = (row: ConfigDiffRow, value: unknown) =>
	`${row.key ?? ""} = ${plainValue(value)}`;

const DIFF_FORMAT: Record<DiffField, DiffFormat> = {
	app_version: {
		label: (c) => c.t("devices:deployShip.diff.appVersion", "App version"),
		value: (_c, _row, value) => shortHash(value),
	},
	definitions: {
		label: (c) => c.t("devices:deployShip.diff.definitions", "Definitions"),
		value: (_c, _row, value) => shortHash(value),
	},
	event: {
		label: (c) => c.t("devices:deployShip.diff.event", "Event"),
		value: (c, row, value) => eventText(c, row.key, value),
	},
	variable: {
		label: (c) => c.t("devices:deployShip.diff.setting", "Setting"),
		value: (_c, row, value) => keyed(row, value),
	},
	secret: {
		label: (c) => c.t("devices:deployShip.diff.secret", "Secret"),
		value: (_c, row) => row.key ?? "",
	},
	endpoint_host: {
		label: (c) => c.t("devices:deployShip.diff.address", "Address"),
		value: (_c, _row, value) => plainValue(value),
	},
	endpoint_port: {
		label: (c) => c.t("devices:deployShip.diff.port", "Port"),
		value: (_c, _row, value) => plainValue(value),
	},
	token: {
		label: (c) => c.t("devices:deployShip.diff.token", "Access token"),
		value: (c) => c.t("devices:deployShip.diff.newToken", "new token"),
	},
	certificate: {
		label: (c) => c.t("devices:deployShip.diff.certificate", "Certificate"),
		value: (_c, _row, value) => shortHash(value),
	},
	instances: {
		label: (c) => c.t("devices:deployShip.diff.instances", "Max instances"),
		value: (_c, _row, value) => plainValue(value),
	},
	cloud_access: {
		label: (c) => c.t("devices:deployShip.diff.cloudAccess", "Cloud access"),
		value: (c) => c.t("devices:deployShip.diff.approval", "approval"),
	},
	spending: {
		label: (c) => c.t("devices:deployShip.diff.spending", "Spending limit"),
		value: (c) => c.t("devices:deployShip.diff.limit", "limit"),
	},
	write_buffering: {
		label: (c) =>
			c.t("devices:deployShip.diff.writeBuffering", "Write buffering"),
		value: bufferingValue,
	},
	isolation: {
		label: (c) => c.t("devices:deployShip.diff.isolation", "Isolation"),
		value: (c, _row, value) => isolationText(c, value),
	},
	packages: {
		label: (c) =>
			c.t("devices:deployShip.diff.packages", "Models and packages"),
		value: (c) => c.t("devices:deployShip.diff.pins", "pinned set"),
	},
};

function bufferingValue(c: Context, _row: ConfigDiffRow, value: unknown) {
	return bufferingText(c, value);
}

function diffRows(c: Context, item: ReviewTarget): DiffRow[] {
	if (!item.config || !item.existing) return [];
	return diffPlacementConfig(item.existing.config, item.config).map((row) => {
		const format = DIFF_FORMAT[row.field];
		return {
			kind: row.kind,
			label: format.label(c),
			...(row.kind === "added"
				? {}
				: { before: format.value(c, row, row.before) }),
			...(row.kind === "removed"
				? {}
				: { after: format.value(c, row, row.after) }),
		};
	});
}

function unreadText(t: DevicesT, item: ReviewTarget): string {
	const device = item.target.name;
	const texts: Record<ReviewUnread, string> = {
		locked: t(
			"devices:deployShip.review.unreadLocked",
			"Unlock {{device}} to see what changes.",
			{ device },
		),
		loading: t(
			"devices:deployShip.review.unreadLoading",
			"Reading the service on {{device}}…",
			{ device },
		),
		failed: t(
			"devices:deployShip.review.unreadFailed",
			"The service on {{device}} couldn't be read. Connect to the device and try again.",
			{ device },
		),
	};
	return texts[item.unread ?? "loading"];
}

function ChangesBlock({
	c,
	item,
}: Readonly<{ c: Context; item: ReviewTarget }>) {
	const { t } = c;
	const existing = item.existing;
	const rows = diffRows(c, item);
	return (
		<Block
			id="dp-diff"
			icon={GitCompare}
			title={t("devices:deployShip.review.changes", "What changes")}
			count={existing ? rows.length : undefined}
			summary={
				existing
					? t(
							"devices:deployShip.review.settingsStep",
							"settings v{{from, number}} → v{{to, number}}",
							{
								from: existing.config_revision,
								to: existing.config_revision + 1,
							},
						)
					: undefined
			}
			stamp={
				<FreshnessStamp
					source={existing ? "live" : "local"}
					age={existing ? "current" : "notloaded"}
				/>
			}
			flush
		>
			{existing ? (
				<DiffRows
					rows={rows}
					emptyText={t(
						"devices:deployShip.review.noChanges",
						"No changes: the device already runs exactly this.",
					)}
				/>
			) : (
				<StateView
					kind={item.unread === "locked" ? "locked" : "notloaded"}
					title={unreadText(t, item)}
				/>
			)}
		</Block>
	);
}

const keyCount = (value: unknown) =>
	Object.keys((value as object | undefined) ?? {}).length;

function settingsSummary(c: Context, config: Record<string, unknown>) {
	const values = keyCount(config.variables);
	const secrets = keyCount(config.secret_overrides);
	const parts: string[] = [];
	if (values)
		parts.push(
			c.t(
				"devices:deployShip.summary.values",
				"{{count, number}} set for this service",
				{ count: values },
			),
		);
	if (secrets)
		parts.push(
			c.t("devices:deployShip.summary.secrets", {
				count: secrets,
				defaultValue_one: "{{count, number}} secret",
				defaultValue_other: "{{count, number}} secrets",
			}),
		);
	return parts.length
		? parts.join(" · ")
		: c.t("devices:deployShip.summary.defaults", "All app defaults");
}

interface SummaryProps {
	c: Context;
	item: ReviewTarget;
}

/** What one service is sent: Review's "New service" block and Rollout's "What you deployed". */
function ServiceSummary({ c, item }: Readonly<SummaryProps>) {
	const { t, plan } = c;
	const { config } = item;
	const device = item.target.name;
	const mode =
		plan.mode === "offline"
			? t("devices:deployShip.mode.offline", "offline copy")
			: t("devices:deployShip.mode.online", "runs online");
	const where =
		item.service.kind === "new"
			? t("devices:deployShip.summary.newOn", " · new on {{device}}", {
					device,
				})
			: t("devices:deployShip.summary.on", " · on {{device}}", { device });
	return (
		<KeyValueList>
			<KvRow label={t("devices:deployShip.summary.service", "Service")}>
				<span className="font-mono">{item.service.serviceId}</span>
				{where}
			</KvRow>
			<KvRow label={t("devices:deployShip.summary.app", "App")}>
				{plan.app?.name} · {mode}
			</KvRow>
			<KvRow label={t("devices:deployShip.plan.colEvents", "Events")}>
				{eventNames(c, item.service.events)}
			</KvRow>
			{config ? (
				<KvRow
					label={t("devices:deployShip.summary.settingsLabel", "Settings")}
				>
					{settingsSummary(c, config)}
				</KvRow>
			) : null}
			<KvRow label={t("devices:deployShip.summary.endpoint", "Web endpoint")}>
				{item.hosted ? (
					<EndpointCell c={c} item={item} />
				) : (
					t("devices:deployShip.summary.none", "None")
				)}
			</KvRow>
			<KvRow label={t("devices:deployShip.summary.access", "Cloud access")}>
				{accessCell(c, item)}
			</KvRow>
			{config ? (
				<KvRow label={t("devices:deployShip.diff.isolation", "Isolation")}>
					{isolationText(c, config.resources)}
				</KvRow>
			) : null}
		</KeyValueList>
	);
}

function NewServiceBlock({ c, item }: Readonly<SummaryProps>) {
	const { t } = c;
	return (
		<Block
			id="dp-summary"
			icon={Box}
			title={t("devices:deployShip.review.newService", "New service")}
			summary={t("devices:deployShip.review.settingsV1", "settings v1")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("devices:deployShip.review.yourChoices", "your choices")}
				/>
			}
		>
			<ServiceSummary c={c} item={item} />
		</Block>
	);
}

const NO_READS: ReadonlyMap<string, ExistingRead> = new Map();

interface PlanSummaryProps {
	plan: DeployPlan;
	prepared?: DeployStepProps["prepared"];
}

/** What the plan sent to each service, from the plan alone (no device is read for it). */
export function PlanSummary({ plan, prepared }: Readonly<PlanSummaryProps>) {
	const c = useReviewContext(plan);
	const bundle = prepared ?? deployRunExtras(plan.draft.deploymentId).prepared;
	const items = useMemo(
		function targets() {
			return reviewTargets(plan, bundle, NO_READS);
		},
		[plan, bundle],
	);
	const rows = items.map(function summary(item) {
		return <ServiceSummary key={item.key} c={c} item={item} />;
	});
	return (
		<div className="flex flex-col gap-3" data-plan-summary="">
			{rows}
		</div>
	);
}

/* How it's applied. */

function StartGate({
	deviceId,
	name,
	appId,
}: Readonly<{ deviceId: string; name: string; appId: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const gate = useGate("start", deviceId, { projectId: appId });
	if (gate.ok) return null;
	return (
		<GateInline kind={gate.kind}>
			{t(
				"deployShip.apply.needsStart",
				"The service stays stopped on {{device}}: {{reason}}",
				{ device: name, reason: gateCopy(t, gate, time).inline },
			)}
		</GateInline>
	);
}

function orderHint(t: DevicesT, order: DeployOrder, first: string): string {
	const hints: Record<DeployOrder, string> = {
		one: t(
			"devices:deployShip.apply.hintOne",
			"Each device starts after the one before it finishes.",
		),
		all: t(
			"devices:deployShip.apply.hintAll",
			"Every device starts at the same time.",
		),
		first: t(
			"devices:deployShip.apply.hintFirst",
			"{{device}} goes first; the rest start once it's done.",
			{ device: first },
		),
	};
	return hints[order];
}

function OrderFields({
	c,
	draft,
	update,
}: Readonly<{ c: Context } & Pick<DeployStepProps, "draft" | "update">>) {
	const { t, plan } = c;
	return (
		<>
			<div className="flex flex-col items-start gap-1.5">
				<span className="text-ui font-medium">
					{t("devices:deployShip.apply.order", "Order")}
				</span>
				<Segmented
					label={t("devices:deployShip.apply.order", "Order")}
					wrap
					value={draft.order}
					onChange={(order) => update({ order })}
					options={[
						{
							value: "one",
							label: t("devices:deployShip.apply.one", "One device at a time"),
						},
						{
							value: "all",
							label: t("devices:deployShip.apply.all", "All at once"),
						},
						{
							value: "first",
							label: t(
								"devices:deployShip.apply.first",
								"First device, then the rest",
							),
						},
					]}
				/>
				<p className="text-xs text-muted-foreground">
					{orderHint(t, draft.order, plan.targets[0]?.name ?? "")}
				</p>
			</div>
			<CheckField
				id="dp-stop"
				checked={draft.stopOnFail}
				onCheckedChange={(stopOnFail) => update({ stopOnFail })}
			>
				{t("devices:deployShip.apply.stopOnFail", "Stop if a device fails")}
				<span className="text-muted-foreground">
					{t(
						"devices:deployShip.apply.stopOnFailHint",
						" · the rest wait for you",
					)}
				</span>
			</CheckField>
		</>
	);
}

interface ApplyChoices {
	timings: SafeUpdateTimings;
	setTimings(next: SafeUpdateTimings): void;
	startStopped: boolean;
	setStartStopped(next: boolean): void;
}

function TimingFields({
	c,
	choices,
}: Readonly<{ c: Context; choices: ApplyChoices }>) {
	const { t } = c;
	const { timings, setTimings } = choices;
	const issues = timingIssues(timings);
	const rangeError = (min: number, max: number) =>
		t("devices:deployShip.apply.range", "Enter {{min}} to {{max}} seconds.", {
			min,
			max,
		});
	return (
		<div className="grid gap-3 @min-[560px]/devices:grid-cols-2">
			<Field
				id="dp-stabilize"
				label={t("devices:deployShip.apply.stabilize", "Must stay healthy for")}
				hint={t(
					"devices:deployShip.apply.stabilizeHint",
					"2 to 60 s after the new instances are ready",
				)}
				error={
					issues.includes("stabilize")
						? rangeError(STABILIZE_RANGE.min, STABILIZE_RANGE.max)
						: undefined
				}
			>
				<InputWithUnit
					unit="s"
					numeric
					inputMode="numeric"
					value={String(timings.stabilizeSeconds)}
					onChange={(event) =>
						setTimings({
							...timings,
							stabilizeSeconds: Number(event.target.value),
						})
					}
				/>
			</Field>
			<Field
				id="dp-deadline"
				label={t("devices:deployShip.apply.deadline", "Time limit to start")}
				hint={t(
					"devices:deployShip.apply.deadlineHint",
					"10 to 600 s. If it isn't healthy by then, the device restores the current version.",
				)}
				error={
					issues.includes("deadline")
						? rangeError(DEADLINE_RANGE.min, DEADLINE_RANGE.max)
						: undefined
				}
			>
				<InputWithUnit
					unit="s"
					numeric
					inputMode="numeric"
					value={String(timings.deadlineSeconds)}
					onChange={(event) =>
						setTimings({
							...timings,
							deadlineSeconds: Number(event.target.value),
						})
					}
				/>
			</Field>
		</div>
	);
}

const STRATEGY_CARDS =
	"[&>legend]:sr-only [&_[role=radiogroup]]:grid-cols-2 @max-[560px]/applystep:[&_[role=radiogroup]]:grid-cols-1";

/** One update target: Safe or Quick, with why safe isn't offered and the two timings. */
function StrategyChoice({
	c,
	item,
	update,
	choices,
}: Readonly<
	{ c: Context; item: ReviewTarget; choices: ApplyChoices } & Pick<
		DeployStepProps,
		"update"
	>
>) {
	const { t } = c;
	const info = item.strategy;
	if (!info) return null;
	const forced = info.reasons.filter(
		(reason) => reason.code !== "chosen_quick",
	);
	return (
		<div className="@container/applystep flex flex-col gap-3">
			<ChoiceCards
				id="dp-strategy"
				legend={t("devices:deployShip.apply.strategy", "Update strategy")}
				className={STRATEGY_CARDS}
				value={info.strategy}
				onValueChange={(strategy) => update({ strategy })}
				options={[
					{
						value: "safe",
						icon: ShieldCheck,
						title: t("devices:deployShip.strategy.safe", "Safe update"),
						hint: t(
							"devices:deployShip.apply.safeHint",
							"The current version keeps running until the new one proves healthy.",
						),
						disabled: forced.length > 0,
					},
					{
						value: "quick",
						icon: Zap,
						title: t("devices:deployShip.strategy.quick", "Quick update"),
						hint: t(
							"devices:deployShip.apply.quickHint",
							"Stops the service, then starts the new version.",
						),
					},
				]}
			/>
			{forced.length ? (
				<ul className="list-disc pl-5 text-xs text-muted-foreground">
					{forced.map((reason) => (
						<li key={reason.code}>
							{t(
								"devices:deployShip.apply.safeUnavailable",
								"Safe update isn't available: {{reason}}.",
								{ reason: strategyReasonText(t, reason) },
							)}
						</li>
					))}
				</ul>
			) : null}
			{info.strategy === "safe" ? (
				<TimingFields c={c} choices={choices} />
			) : null}
			{info.known && item.existing?.desired_state !== "running" ? (
				<SwitchField
					id="dp-start-stopped"
					checked={choices.startStopped}
					onCheckedChange={choices.setStartStopped}
				>
					{t(
						"devices:deployShip.apply.startStopped",
						"Start it with the new version (it's stopped now)",
					)}
				</SwitchField>
			) : null}
		</div>
	);
}

function UpdateLine({ c, item }: Readonly<SummaryProps>) {
	const how = c.t(
		"devices:deployShip.apply.onDevice",
		"on {{device}}: {{how}}",
		{ device: item.target.name, how: howCell(c, item) },
	);
	return (
		<li>
			<span className="font-mono">{item.service.serviceId}</span> {how}
		</li>
	);
}

interface UpdatesProps {
	c: Context;
	items: readonly ReviewTarget[];
}

function UpdatesList({ c, items }: Readonly<UpdatesProps>) {
	const { t } = c;
	const lines = items.map(function line(item) {
		return <UpdateLine key={item.key} c={c} item={item} />;
	});
	const title = t("devices:deployShip.apply.updates", "Updates");
	const hint = t(
		"devices:deployShip.apply.updatesHint",
		"Safe where the device and the events allow it. Each device checks the new version for up to 2 min.",
	);
	return (
		<div className="flex flex-col gap-1">
			<span className="text-label font-medium text-muted-foreground">
				{title}
			</span>
			<ul className="list-disc pl-5 text-ui">{lines}</ul>
			<p className="text-xs text-muted-foreground">{hint}</p>
		</div>
	);
}

function StartField({
	c,
	items,
	draft,
	update,
}: Readonly<
	{ c: Context; items: readonly ReviewTarget[] } & Pick<
		DeployStepProps,
		"draft" | "update"
	>
>) {
	const { t, plan } = c;
	const appId = plan.app?.id ?? "";
	const news = items.filter((item) => item.service.kind === "new");
	return (
		<div className="flex flex-col gap-1.5">
			<SwitchField
				id="dp-start"
				checked={draft.start}
				onCheckedChange={(start) => update({ start })}
			>
				{t("devices:deployShip.apply.start", "Start after deploy")}
			</SwitchField>
			<p className="text-xs text-muted-foreground">
				{news.length
					? t(
							"devices:deployShip.apply.startHintNew",
							"New services are created stopped. With this on, each starts right after.",
						)
					: t(
							"devices:deployShip.apply.startHintUpdate",
							"Turn this off to leave the service stopped after the update.",
						)}
			</p>
			{draft.start
				? news.map((item) => (
						<StartGate
							key={item.key}
							deviceId={item.target.deviceId}
							name={item.target.name}
							appId={appId}
						/>
					))
				: null}
		</div>
	);
}

function ApplyBlock({
	c,
	items,
	choices,
	draft,
	update,
}: Readonly<
	{ c: Context; items: readonly ReviewTarget[]; choices: ApplyChoices } & Pick<
		DeployStepProps,
		"draft" | "update"
	>
>) {
	const { t, plan } = c;
	const multi = plan.targets.length > 1;
	const updates = items.filter((item) => item.service.kind !== "new");
	const [single] = items.length === 1 ? updates : [];
	const quickSingle = single?.strategy?.strategy === "quick";
	const showStart = updates.length < items.length || quickSingle;
	return (
		<Block
			id="dp-apply"
			icon={Rocket}
			title={t("devices:deployShip.apply.title", "How it's applied")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("devices:deployShip.review.yourChoice", "your choice")}
				/>
			}
			bodyClassName="flex flex-col gap-4"
		>
			{multi ? <OrderFields c={c} draft={draft} update={update} /> : null}
			{showStart ? (
				<StartField c={c} items={items} draft={draft} update={update} />
			) : null}
			{single ? (
				<StrategyChoice c={c} item={single} update={update} choices={choices} />
			) : null}
			{multi && updates.length ? <UpdatesList c={c} items={updates} /> : null}
		</Block>
	);
}

/* Before you deploy (APP §7.7). */

interface Consequences {
	what: string[];
	who: string[];
	stays: string[];
	undo: string[];
	first: string[];
	irreversible: number;
}

/** "Each gets cloud access until 30 Oct and a €25.00 spending limit." */
function accessSentence(c: Context): string {
	const { t, plan, time } = c;
	const { approval, spending } = plan.draft;
	const date = time.at(approval.expiresAt);
	const online = plan.mode === "online";
	if (!spending)
		return online
			? t(
					"devices:deployShip.conseq.cloud",
					"Each gets cloud access until {{date}}.",
					{ date },
				)
			: t(
					"devices:deployShip.conseq.model",
					"Each gets model access until {{date}}.",
					{ date },
				);
	const limit = formatEuroMicros(spending.limitMicros);
	return online
		? t(
				"devices:deployShip.conseq.cloudSpend",
				"Each gets cloud access until {{date}} and a {{limit}} spending limit.",
				{ date, limit },
			)
		: t(
				"devices:deployShip.conseq.modelSpend",
				"Each gets model access until {{date}} and a {{limit}} spending limit.",
				{ date, limit },
			);
}

/** "0.0.0.0:8080 starts answering on edge-berlin-01 and studio-mac-mini." per address. */
function answersSentence(c: Context, hosted: readonly ReviewTarget[]): string {
	const byAddress = new Map<string, string[]>();
	for (const item of hosted) {
		const address = addressOf(item);
		byAddress.set(address, [
			...(byAddress.get(address) ?? []),
			item.target.name,
		]);
	}
	const answers = [...byAddress].map(([address, devices]) =>
		c.t(
			"devices:deployShip.conseq.answersAt",
			"{{address}} starts answering on {{devices}}.",
			{ address, devices: c.list(devices) },
		),
	);
	return `${answers.join(" ")} ${c.t(
		"devices:deployShip.conseq.tokenOnce",
		"Clients need the access token shown once after deploy.",
	)}`;
}

function newWho(c: Context, news: readonly ReviewTarget[]): string {
	const { t, plan } = c;
	if (!plan.draft.start)
		return t(
			"devices:deployShip.conseq.nobodyYet",
			"Nobody yet: the new services stay stopped until someone starts them.",
		);
	const hosted = news.filter((item) => item.hosted);
	if (hosted.length) return answersSentence(c, hosted);
	return t(
		"devices:deployShip.conseq.background",
		"Runs {{events}} in the background on {{devices}}.",
		{
			events: eventNames(c, [
				...new Set(news.flatMap((item) => item.service.events)),
			]),
			devices: c.list(news.map((item) => item.target.name)),
		},
	);
}

function newConsequences(
	c: Context,
	news: readonly ReviewTarget[],
	out: Consequences,
) {
	if (!news.length) return;
	const { t, plan } = c;
	const services = c.list([
		...new Set(news.map((item) => item.service.serviceId)),
	]);
	out.what.push(
		t(
			"devices:deployShip.conseq.creates",
			"Creates {{services}} on {{devices}}, serving {{events}}.",
			{
				services,
				devices: c.list(news.map((item) => item.target.name)),
				events: eventNames(c, [
					...new Set(news.flatMap((item) => item.service.events)),
				]),
			},
		),
	);
	if (wantsAccess(plan)) out.what.push(accessSentence(c));
	out.who.push(newWho(c, news));
	out.stays.push(
		plan.mode === "online"
			? t(
					"devices:deployShip.conseq.staysOnline",
					"Data stays in the cloud; nothing else on these devices changes.",
				)
			: t(
					"devices:deployShip.conseq.staysOffline",
					"Each new service starts with the copy's data. Other services on these devices aren't touched.",
				),
	);
	out.undo.push(
		t(
			"devices:deployShip.conseq.undoNew",
			"Stop or remove {{services}} on a device. Access and spending limits stay until you revoke them.",
			{ services },
		),
	);
}

interface UpdateNames {
	service: string;
	device: string;
}

/** A quick update stops the service: who notices, and why it isn't a safe one. */
function quickConsequence(
	c: Context,
	item: ReviewTarget,
	names: UpdateNames,
	out: Consequences,
) {
	const { t } = c;
	const { service, device } = names;
	out.irreversible += 1;
	out.what.push(
		t(
			"devices:deployShip.conseq.quick",
			"{{service}} on {{device}} stops, then starts the newest version.",
			{ service, device },
		),
	);
	out.who.push(
		t(
			"devices:deployShip.conseq.quickWho",
			"{{service}} is down for about 10 s on {{device}}.",
			{ service, device },
		),
	);
	const reason = item.strategy?.reasons.find(
		(value) => value.code !== "chosen_quick",
	);
	if (!reason) return;
	out.first.push(
		t(
			"devices:deployShip.conseq.noSafe",
			"Safe update isn't available on {{device}}: {{reason}}.",
			{ device, reason: strategyReasonText(t, reason) },
		),
	);
}

function updateConsequence(c: Context, item: ReviewTarget, out: Consequences) {
	const { t, plan } = c;
	const service = item.service.serviceId;
	const device = item.target.name;
	if (plan.draft.version === "keep") {
		out.what.push(
			t(
				"devices:deployShip.conseq.settings",
				"{{service}} on {{device}} gets new settings.",
				{ service, device },
			),
		);
		return;
	}
	if (item.strategy?.strategy !== "safe") {
		quickConsequence(c, item, { service, device }, out);
		return;
	}
	out.what.push(
		t(
			"devices:deployShip.conseq.safe",
			"{{service}} on {{device}} switches to the newest version with a safe update.",
			{ service, device },
		),
	);
}

function staysAfterUpdate(c: Context, keep: boolean): string {
	const { t, plan } = c;
	if (keep)
		return t(
			"devices:deployShip.conseq.staysKeep",
			"The app version and its data stay. Nothing is uploaded.",
		);
	return plan.mode === "online"
		? t(
				"devices:deployShip.conseq.staysUpdateOnline",
				"Data stays in the cloud. Cloud access and spending limits stay.",
			)
		: t(
				"devices:deployShip.conseq.staysUpdateOffline",
				"The data on the device stays as it is; the tables in the new copy aren't used.",
			);
}

function undoAfterUpdate(t: DevicesT, keep: boolean, safe: boolean): string {
	if (keep)
		return t(
			"devices:deployShip.conseq.undoSettings",
			"Change the settings back the same way.",
		);
	return safe
		? t(
				"devices:deployShip.conseq.undoSafe",
				"If it isn't healthy, the device restores the current version on its own.",
			)
		: t(
				"devices:deployShip.conseq.undoQuick",
				"Not with one click. The previous version's files stay on the device.",
			);
}

function updateConsequences(
	c: Context,
	updates: readonly ReviewTarget[],
	out: Consequences,
) {
	if (!updates.length) return;
	const { t, plan } = c;
	for (const item of updates) updateConsequence(c, item, out);
	const keep = plan.draft.version === "keep";
	const safe = updates.some((item) => item.strategy?.strategy === "safe");
	if (safe && !keep)
		out.who.push(
			t(
				"devices:deployShip.conseq.safeWho",
				"The current version keeps answering until the new one is healthy.",
			),
		);
	out.stays.push(staysAfterUpdate(c, keep));
	out.undo.push(undoAfterUpdate(t, keep, safe));
}

/** The schedules a target's service takes that it does not serve today, as the plan says. */
function addedSchedules(c: Context, item: ReviewTarget): AppEventInput[] {
	const added = new Set(item.service.addedSchedules);
	return (c.plan.app?.events ?? []).filter((event) => added.has(event.id));
}

/**
 * A schedule that moves to a device: when it runs there, that the hub stops
 * running it, and what its cloud access means for its runs.
 */
function scheduleConsequence(
	c: Context,
	item: ReviewTarget,
	event: AppEventInput,
	out: Consequences,
) {
	const { t, plan, time } = c;
	const { schedule } = appEventRule(event);
	if (!schedule) return;
	const device = item.target.name;
	const service = item.service.serviceId;
	const [next] = nextRuns(schedule, time.now, 1);
	out.what.push(
		t(
			"devices:deployShip.conseq.schedule",
			"{{event}} runs on {{device}} {{words}} ({{zone}}). First run {{time}}. Missed runs are not made up.",
			{
				event: event.name,
				device,
				words: scheduleWordsInline(t, schedule.expression, time.locale),
				zone: scheduleZone(t, schedule),
				time:
					next === undefined
						? ""
						: scheduleTime(t, Math.floor(next / 1000), schedule.timezone, time),
			},
		),
	);
	if (plan.mode !== "online") return;
	out.what.push(
		t(
			"devices:deployShip.conseq.scheduleMoves",
			"The hub stops running {{event}} when {{device}} starts it.",
			{ event: event.name, device },
		),
	);
	out.undo.push(
		t(
			"devices:deployShip.conseq.scheduleUndo",
			"Remove it from the service, or choose Run it on the hub again in Events: the hub runs it again a few minutes later.",
		),
	);
	if (item.service.kind !== "new") return;
	// A new service gets the access this deploy asks for; an existing one keeps what it has.
	if (plan.draft.approval.files === "read_only")
		out.who.push(
			t(
				"devices:deployShip.conseq.scheduleReadOnly",
				"{{service}}'s cloud access is read-only. Runs that change the app's data will fail.",
				{ service },
			),
		);
	if (plan.draft.spending)
		out.who.push(
			t(
				"devices:deployShip.conseq.scheduleLimit",
				"Each run can use hosted models within {{service}}'s spending limit. When the limit is used up, this service's other events lose hosted models too.",
				{ service },
			),
		);
}

/** A one-time schedule that moves to a device: when it runs there, and that it runs at most once (§3.1). */
function onceConsequence(
	c: Context,
	item: ReviewTarget,
	event: AppEventInput,
	out: Consequences,
) {
	const { t } = c;
	const { once } = appEventRule(event);
	if (!once) return;
	const device = item.target.name;
	out.what.push(
		t(
			"devices:deployShip.conseq.once",
			"{{event}} runs once on {{device}} at {{time}} ({{zone}}). If {{device}} isn't running then, or within 15 minutes after, it doesn't run at all.",
			{
				event: event.name,
				device,
				time: `${once.date} ${once.time}`,
				zone: scheduleZone(t, once),
			},
		),
		t(
			"devices:deployShip.conseq.onceAfter",
			"After that nothing runs it again.",
		),
	);
}

function scheduleConsequences(
	c: Context,
	items: readonly ReviewTarget[],
	out: Consequences,
) {
	const { t, plan } = c;
	const schedules = scheduleIds(plan.app);
	for (const item of items)
		for (const event of addedSchedules(c, item)) {
			scheduleConsequence(c, item, event, out);
			onceConsequence(c, item, event, out);
		}
	const switches = items.some(
		(item) =>
			item.service.kind !== "new" &&
			item.service.events.some((eventId) => schedules.has(eventId)),
	);
	if (switches)
		out.who.push(
			t(
				"devices:deployShip.conseq.scheduleSwitch",
				"A scheduled time that falls into the switch is skipped.",
			),
		);
}

/** The events a target's service gets that it doesn't serve today: all of a new service's. */
function addedEvents(c: Context, item: ReviewTarget): AppEventInput[] {
	const served = new Set(
		(item.existing?.config.events ?? []).map((event) => event.event_id),
	);
	const added = new Set(
		item.service.events.filter((eventId) => !served.has(eventId)),
	);
	return (c.plan.app?.events ?? []).filter((event) => added.has(event.id));
}

/** "https://10.0.4.20:8080": where an Endpoint of the service answers. */
function serviceAddress(item: ReviewTarget): string {
	const { certificateId } = item.target.endpoint;
	const certificate =
		certificateId === undefined
			? item.existing?.config.tls_certificate_id
			: certificateId;
	return `${certificate ? "https" : "http"}://${addressOf(item)}`;
}

/** Endpoints a service starts to answer (§2.1): the address, the token that calls them, what the hub keeps doing, how a failure answers. */
function endpointConsequences(
	c: Context,
	item: ReviewTarget,
	out: Consequences,
) {
	const { t, plan } = c;
	const endpoints = addedEvents(c, item).filter(
		(event) => appEventRule(event).route && !event.default_page_id,
	);
	if (!endpoints.length) return;
	const service = item.service.serviceId;
	const address = serviceAddress(item);
	for (const event of endpoints) {
		const route = appEventRule(event).route;
		if (route)
			out.what.push(
				t(
					"devices:deployShip.conseq.endpoint",
					"{{event}} answers {{method}} {{address}}{{path}}.",
					{
						event: event.name,
						method: route.method,
						address,
						path: route.path,
					},
				),
			);
	}
	out.who.push(
		t(
			"devices:deployShip.conseq.endpointToken",
			"Callers need {{service}}'s access token. The token set in Events is not used on a device.",
			{ service },
		),
	);
	const served = item.service.events.filter((eventId) => {
		const event = plan.app?.events.find((row) => row.id === eventId);
		return event ? eventKind(event) === "served" : false;
	});
	if (served.length > 1)
		out.who.push(
			t(
				"devices:deployShip.conseq.endpointShared",
				"Everyone with {{service}}'s access token can call all of its endpoints, pages and chats.",
				{ service },
			),
		);
	const limit = item.existing?.config.hosting?.request_timeout_secs ?? 300;
	out.who.push(
		t(
			"devices:deployShip.conseq.endpointAnswers",
			"A failed run answers 502, and a run is stopped at {{service}}'s time limit ({{limit}}).",
			{
				service,
				limit: t("devices:deployShip.conseq.seconds", "{{value, number}} s", {
					value: limit,
				}),
			},
		),
	);
	const names = c.list(endpoints.map((event) => event.name));
	out.stays.push(
		plan.mode === "online"
			? t(
					"devices:deployShip.conseq.endpointElsewhere",
					"The hub keeps answering {{events}} at its own address.",
					{ events: names },
				)
			: t(
					"devices:deployShip.conseq.endpointElsewhereLocal",
					"This computer keeps answering {{events}} while Flow-Like is open.",
					{ events: names },
				),
	);
}

/** Forms and quick actions a service gets: who may run them from Devices, and from its service page. */
function onDemandConsequences(
	c: Context,
	item: ReviewTarget,
	out: Consequences,
) {
	const { t } = c;
	const service = item.service.serviceId;
	for (const event of addedEvents(c, item)) {
		if (eventKind(event) !== "on_demand") continue;
		out.who.push(
			t(
				"devices:deployShip.conseq.onDemand",
				"{{event}} can be run from Devices by people who may start {{service}}.",
				{ event: event.name, service },
			),
		);
		if (item.hosted)
			out.who.push(
				t(
					"devices:deployShip.conseq.onDemandPage",
					"Anyone with {{service}}'s access token can also run it from the service page.",
					{ service },
				),
			);
	}
}

/** Bots a service takes (§5.2): it answers from the device, what is lost on a restart, how to undo it. */
function botConsequences(c: Context, item: ReviewTarget, out: Consequences) {
	const { t } = c;
	const added = new Set(item.service.addedBots);
	const device = item.target.name;
	const service = item.service.serviceId;
	for (const event of c.plan.app?.events ?? []) {
		if (!added.has(event.id)) continue;
		out.what.push(
			t(
				"devices:deployShip.conseq.bot",
				"{{event}} answers from {{device}} while {{service}} runs.",
				{ event: event.name, device, service },
			),
		);
		if (appEventRule(event).bot?.provider === "telegram")
			out.what.push(
				t(
					"devices:deployShip.conseq.botWebhook",
					"{{device}} removes the bot's Telegram webhook when it first connects.",
					{ device },
				),
			);
		else
			out.who.push(
				t(
					"devices:deployShip.conseq.botGap",
					"Discord messages sent while {{service}} restarts or updates are not answered.",
					{ service },
				),
			);
		out.undo.push(
			t(
				"devices:deployShip.conseq.botUndo",
				"Remove {{event}} from the service, or take it back in Events. Nothing else runs it afterwards until you start it somewhere.",
				{ event: event.name },
			),
		);
	}
}

function kindConsequences(
	c: Context,
	items: readonly ReviewTarget[],
	out: Consequences,
) {
	for (const item of items) {
		endpointConsequences(c, item, out);
		onDemandConsequences(c, item, out);
		botConsequences(c, item, out);
	}
}

/** A flow version this deploy created for an event that follows Latest: said before anything is uploaded. */
function flowConsequences(
	c: Context,
	prepared: DeployStepProps["prepared"],
	flowNames: ReadonlyMap<string, string>,
	out: Consequences,
) {
	const { t, plan } = c;
	const created = (prepared?.flows ?? []).filter((flow) => flow.created);
	for (const flow of created)
		out.what.push(
			t(
				"devices:deployShip.conseq.flowCreated",
				"Created flow version {{version}} of {{flow}} from the current edits. The event keeps following Latest.",
				{
					version: flow.version.join("."),
					flow: flowLabel(t, plan, flowNames, flow.boardId),
				},
			),
		);
	if (created.length)
		out.undo.push(
			t(
				"devices:deployShip.conseq.flowUndo",
				"A created flow version stays in the flow's history.",
			),
		);
}

function whenText(c: Context, label: string): string {
	const { t, plan } = c;
	const { order, stopOnFail } = plan.draft;
	if (plan.targets.length <= 1)
		return t(
			"devices:deployShip.conseq.whenOne",
			"When you select {{label}}.",
			{
				label,
			},
		);
	const texts: Record<DeployOrder, string> = {
		one: t("devices:deployShip.conseq.whenEach", "One device at a time."),
		all: t("devices:deployShip.conseq.whenAll", "All devices at once."),
		first: t(
			"devices:deployShip.conseq.whenFirst",
			"{{device}} first, then the rest at once.",
			{ device: plan.targets[0]?.name ?? "" },
		),
	};
	const stops =
		stopOnFail && order !== "all"
			? ` ${t("devices:deployShip.conseq.stops", "Stops if a device fails.")}`
			: "";
	return `${texts[order]}${stops}`;
}

interface ConsequenceFacts {
	prepared: DeployStepProps["prepared"];
	flowNames: ReadonlyMap<string, string>;
}

function consequenceRows(
	c: Context,
	items: readonly ReviewTarget[],
	label: string,
	facts: ConsequenceFacts,
): ConsequenceRows {
	const { t } = c;
	const out: Consequences = {
		what: [],
		who: [],
		stays: [],
		undo: [],
		first: [],
		irreversible: 0,
	};
	newConsequences(
		c,
		items.filter((item) => item.service.kind === "new"),
		out,
	);
	const updates = items.filter((item) => item.service.kind !== "new");
	updateConsequences(c, updates, out);
	scheduleConsequences(c, items, out);
	kindConsequences(c, items, out);
	flowConsequences(c, facts.prepared, facts.flowNames, out);
	for (const item of items)
		if (item.target.locked)
			out.first.push(
				t("devices:deployShip.conseq.unlock", "Unlock {{device}}.", {
					device: item.target.name,
				}),
			);
	return {
		what: out.what.join(" "),
		who:
			out.who.join(" ") ||
			t("devices:deployShip.conseq.nobody", "Nobody right now."),
		stays: out.stays.join(" "),
		when: whenText(c, label),
		undo: {
			reversible: out.irreversible ? null : true,
			text: [...new Set(out.undo)].join(" "),
		},
		...(out.first.length ? { first: out.first.join(" ") } : {}),
	};
}

/* Size (BG19) and IDs. */

function SizeBlock({
	c,
	items,
}: Readonly<{ c: Context; items: readonly ReviewTarget[] }>) {
	const { t } = c;
	const sized = items.filter((item) => item.bytes !== undefined);
	const [largest] = [...sized].sort((a, b) => (b.bytes ?? 0) - (a.bytes ?? 0));
	if (!largest) return null;
	const bytes = largest.bytes ?? 0;
	const over = bytes > DEPLOYMENT_CONFIG_BYTES;
	const params = { bytes, limit: DEPLOYMENT_CONFIG_BYTES };
	const text = t(
		"devices:deployShip.size.used",
		"{{bytes, number}} of {{limit, number}} bytes",
		params,
	);
	return (
		<Block
			id="dp-size"
			icon={Gauge}
			title={t("devices:deployShip.size.title", "Size")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("devices:deployShip.size.stamp", "calculated now")}
				/>
			}
			bodyClassName="flex flex-col gap-2"
		>
			<div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
				<Meter
					className="w-80 max-w-full"
					label={text}
					segments={[
						{
							value: Math.min(100, (bytes / DEPLOYMENT_CONFIG_BYTES) * 100),
							tone: over ? "critical" : bytes > 10_000 ? "warning" : "neutral",
						},
					]}
				/>
				<span className="text-ui tabular-nums" data-size-text="">
					<Trans
						t={t}
						i18nKey="deployShip.size.usedBold"
						defaults="<1>{{bytes, number}}</1> of {{limit, number}} bytes"
						values={params}
						components={{ 1: <b className="font-semibold" /> }}
					/>
					{sized.length > 1 ? (
						<Trans
							t={t}
							i18nKey="deployShip.size.largestOf"
							defaults=" · largest: <1>{{service}}</1> on <1>{{device}}</1>"
							values={{
								service: largest.service.serviceId,
								device: largest.target.name,
							}}
							components={{ 1: <span className="font-mono" /> }}
						/>
					) : null}
				</span>
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"devices:deployShip.size.hint",
					"A service's settings travel to the device in one message of at most 12,000 bytes.",
				)}
			</p>
			{over ? (
				<InlineResult tone="critical">
					{t(
						"devices:deployShip.size.tooLarge",
						"Too large to deploy on {{device}}. Shorten long setting values or deploy fewer events in one service.",
						{ device: largest.target.name },
					)}
				</InlineResult>
			) : null}
		</Block>
	);
}

function IdsDisclosure({
	c,
	items,
}: Readonly<{ c: Context; items: readonly ReviewTarget[] }>) {
	const { t, plan } = c;
	return (
		<details className="group text-ui">
			<summary className="flex cursor-pointer list-none items-center gap-1.5 [&::-webkit-details-marker]:hidden">
				<ChevronRight
					aria-hidden
					className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90"
				/>
				<b className="font-medium">
					{t("devices:deployShip.ids.title", "IDs")}
				</b>
				<span className="text-muted-foreground">
					{t("devices:deployShip.ids.advanced", "advanced")}
				</span>
			</summary>
			<div className="mt-2 flex flex-col gap-2">
				<KeyValueList>
					{items.map((item) => (
						<KvRow
							key={item.key}
							label={<span className="font-mono">{item.target.name}</span>}
						>
							<IdRef id={item.service.serviceId} />
							{" · "}
							<IdRef
								id={item.existing?.deployment_id ?? plan.draft.deploymentId}
							/>
						</KvRow>
					))}
				</KeyValueList>
				<p className="text-xs text-muted-foreground">
					{t(
						"devices:deployShip.ids.hint",
						"The service ID and the deploy ID. Both are permanent once cloud access is approved; new services of one deploy share the deploy ID.",
					)}
				</p>
			</div>
		</details>
	);
}

/* The primary. */

interface Blocker {
	text: string;
	step?: DeployStepId;
}

const PROBLEM_COPY: Record<
	WireProblem,
	(t: DevicesT, device: string) => string
> = {
	too_large: (t, device) =>
		t(
			"devices:deployShip.block.tooLarge",
			"The settings for {{device}} are too large to send.",
			{ device },
		),
	no_changes: (t, device) =>
		t(
			"devices:deployShip.block.noChanges",
			"Nothing changes on {{device}}: it already runs exactly this.",
			{ device },
		),
	needs_newest: (t, device) =>
		t(
			"devices:deployShip.block.needsNewest",
			"The version on {{device}} is from before app versions were approved. Update it to the newest version to change its settings.",
			{ device },
		),
	invalid: (t, device) =>
		t(
			"devices:deployShip.block.invalid",
			"The settings for {{device}} can't be applied as they are.",
			{ device },
		),
	not_prepared: (t) =>
		t(
			"devices:deployShip.block.notPrepared",
			"The app version isn't prepared yet.",
		),
};

function blockerOf(
	c: Context,
	props: Readonly<DeployStepProps>,
	items: readonly ReviewTarget[],
	choices: ApplyChoices,
): Blocker | null {
	const { t } = c;
	const blocking = props.check.firstBlocking;
	if (blocking) {
		// The frame words the plan's first problem; without it the step is at least named.
		const named = t(
			"devices:deployShip.block.step",
			"{{step}} needs your attention first.",
			{ step: STEP_NAME[blocking.step](t) },
		);
		return { text: props.blockingText ?? named, step: blocking.step };
	}
	const waiting = items.find((item) => item.unread);
	if (waiting) return { text: unreadText(t, waiting), step: "where" };
	const safe = items.some((item) => item.strategy?.strategy === "safe");
	if (safe && items.length === 1 && timingIssues(choices.timings).length)
		return {
			text: t(
				"devices:deployShip.block.timings",
				"Fix the two safe-update times above.",
			),
		};
	const broken = items.find(
		(item) => item.problem && item.problem !== "not_prepared",
	);
	if (broken?.problem)
		return { text: PROBLEM_COPY[broken.problem](t, broken.target.name) };
	return null;
}

function useAnnounce(c: Context, route: DeployRoute) {
	const { navigate } = useDevicesRoute();
	const { t, plan } = c;
	return (state: DeployRunState) => {
		const done = state.rows.filter((row) => row.state === "done").length;
		toast(
			t(
				"devices:deployShip.toast.finished",
				"{{what}} deployed to {{done, number}} of {{total, number}} devices",
				{ what: deployWhat(plan), done, total: state.rows.length },
			),
			{
				action: {
					label: t("devices:deployShip.toast.open", "Open"),
					onClick: () => navigate({ ...route, step: "rollout" }),
				},
			},
		);
	};
}

interface DeployControl {
	c: Context;
	props: DeployStepProps;
	items: readonly ReviewTarget[];
	choices: ApplyChoices;
	label: string;
}

/** Starts the run with what the hook's signature has no slot for, then opens Rollout. */
function useDeploy({ c, props, items, choices }: DeployControl) {
	const { plan } = c;
	const { draft, goTo } = props;
	const { route } = useDevicesRoute();
	const deployRoute: DeployRoute =
		route.screen === "deploy"
			? route
			: {
					screen: "deploy",
					deviceIds: plan.targets.map((target) => target.deviceId),
				};
	const announce = useAnnounce(c, deployRoute);
	const run = useDeployRun(plan, {
		title: deployRunTitleRef(plan),
		oneAtATime: draft.order === "one",
		stopOnFail: draft.stopOnFail,
	});
	const blocker = blockerOf(c, props, items, choices);
	const deploy = () => {
		if (blocker) return;
		setDeployRunExtras(draft.deploymentId, {
			...(props.prepared ? { prepared: props.prepared } : {}),
			timings: choices.timings,
			startStopped: choices.startStopped,
			route: deployRoute,
			announce,
		});
		run.start();
		goTo("rollout");
	};
	return { blocker, deploy, started: run.state.status !== "idle" };
}

function DeployReason({
	blocker,
	goTo,
	t,
}: Readonly<{
	blocker: Blocker;
	goTo: DeployStepProps["goTo"];
	t: DevicesT;
}>) {
	const { step } = blocker;
	return (
		<p
			id="dp-deploy-reason"
			className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 text-xs text-ink-2"
		>
			<span>{blocker.text}</span>
			{step ? (
				<DvButton variant="link" size="xs" onClick={() => goTo(step)}>
					{t("devices:deployShip.block.goTo", "Go to {{step}}", {
						step: STEP_NAME[step](t),
					})}
				</DvButton>
			) : null}
		</p>
	);
}

function DeployButton(control: Readonly<DeployControl>) {
	const { t } = control.c;
	const { goTo } = control.props;
	const bucket = useWidthBucket();
	const { blocker, deploy, started } = useDeploy(control);
	// Next to Back when the frame's foot offers a slot; else the button closes the step.
	const { anchor, slot } = useWizardSlot("foot");
	const button = started ? (
		<DvButton icon={Server} onClick={() => goTo("rollout")}>
			{t("devices:deployShip.review.openRollout", "Open the rollout")}
		</DvButton>
	) : (
		<DvButton
			variant="primary"
			icon={Rocket}
			aria-disabled={blocker ? true : undefined}
			aria-describedby={blocker ? "dp-deploy-reason" : undefined}
			className={bucket === "phone" ? "w-full" : undefined}
			onClick={deploy}
		>
			{control.label}
		</DvButton>
	);
	return (
		<div
			className="flex flex-wrap items-center justify-end gap-x-3 gap-y-2"
			data-deploy-action=""
		>
			<span ref={anchor} hidden />
			{blocker ? <DeployReason blocker={blocker} goTo={goTo} t={t} /> : null}
			{slot ? createPortal(button, slot) : button}
		</div>
	);
}

/* The step. */

function useApplyChoices(draft: DeployDraft): ApplyChoices {
	const stored = deployRunExtras(draft.deploymentId);
	const [timings, setTimings] = useState<SafeUpdateTimings>(
		stored.timings ?? SAFE_UPDATE_TIMINGS,
	);
	const [startStopped, setStartStopped] = useState(
		stored.startStopped === true,
	);
	return { timings, setTimings, startStopped, setStartStopped };
}

function ledeOf(t: DevicesT, plan: DeployPlan): string {
	return plan.targets.length > 1
		? t(
				"devices:deployShip.review.ledeMany",
				"What happens on each device. Nothing has been sent yet.",
			)
		: t(
				"devices:deployShip.review.ledeOne",
				"Check what changes, then deploy. Nothing has been sent yet.",
			);
}

function FirstBlock({
	c,
	items,
	exceptions,
}: Readonly<{
	c: Context;
	items: readonly ReviewTarget[];
	exceptions: readonly PlanException[];
}>) {
	const { t } = c;
	const [single] = items;
	if (items.length > 1 || !single)
		return (
			<Block
				id="dp-plan"
				icon={ListChecks}
				title={t("devices:deployShip.plan.title", "Plan")}
				summary={
					<span className={HEAD_CHIP}>
						{t("devices:deployShip.plan.deviceCount", {
							count: c.plan.targets.length,
							defaultValue_one: "{{count, number}} device",
							defaultValue_other: "{{count, number}} devices",
						})}
					</span>
				}
				stamp={
					<FreshnessStamp
						source="local"
						age="current"
						text={t("devices:deployShip.review.yourChoices", "your choices")}
					/>
				}
				flush
			>
				<PlanTable c={c} items={items} exceptions={exceptions} />
			</Block>
		);
	return single.service.kind === "new" ? (
		<NewServiceBlock c={c} item={single} />
	) : (
		<ChangesBlock c={c} item={single} />
	);
}

export function ReviewStep(props: Readonly<DeployStepProps>) {
	const { plan, draft, update, check, goTo } = props;
	const c = useReviewContext(plan);
	const { t } = c;
	const items = useReviewTargets(props);
	const choices = useApplyChoices(draft);
	const label = deployButtonLabel(c, items);
	const { prepared } = props;
	const flowNames = useFlowNames(plan.app?.id, !!prepared?.flows?.length);
	const header = (
		<WizardStepHeader
			step={7}
			total={8}
			title={t("devices:deployShip.step.review", "Review")}
			lede={ledeOf(t, plan)}
		/>
	);
	if (!items.length)
		return (
			<div className="flex flex-col gap-4">
				{header}
				<StateView
					kind="notloaded"
					title={t("devices:deployShip.review.noDevices", "No devices yet")}
					text={t(
						"devices:deployShip.review.noDevicesText",
						"Pick devices in Where to see the plan.",
					)}
					actions={
						<DvButton size="sm" icon={Server} onClick={() => goTo("where")}>
							{t("devices:deployShip.review.goWhere", "Go to Where")}
						</DvButton>
					}
				/>
			</div>
		);
	return (
		<div className="flex min-w-0 flex-col gap-4">
			{header}
			<FirstBlock c={c} items={items} exceptions={check.exceptions} />
			<ApplyBlock
				c={c}
				items={items}
				choices={choices}
				draft={draft}
				update={update}
			/>
			<Block
				id="dp-conseq"
				icon={ListChecks}
				title={t("devices:deployShip.review.before", "Before you deploy")}
				stamp={
					<FreshnessStamp
						source="local"
						age="current"
						text={t(
							"devices:deployShip.review.fromChoices",
							"from your choices",
						)}
					/>
				}
			>
				<ConsequencePreview
					rows={consequenceRows(c, items, label, { prepared, flowNames })}
				/>
			</Block>
			<SizeBlock c={c} items={items} />
			<IdsDisclosure c={c} items={items} />
			<DeployButton
				c={c}
				props={props}
				items={items}
				choices={choices}
				label={label}
			/>
		</div>
	);
}
