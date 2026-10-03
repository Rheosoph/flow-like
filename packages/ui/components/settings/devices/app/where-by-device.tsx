"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleArrowUp,
	CircleDashed,
	CirclePlay,
	Cloud,
	CloudOff,
	Copy,
	Ellipsis,
	Eye,
	EyeOff,
	FileText,
	HardDrive,
	Info,
	Link2,
	Lock,
	LockOpen,
	Play,
	Plus,
	RotateCw,
	Server,
	Settings2,
	Square,
	Trash2,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import type {
	AppDeviceGroup,
	AppServiceRow,
	MatrixRow,
} from "../../../../lib/device-management/model/app-plan";
import {
	fleetFacts,
	rolloutEndsAt,
} from "../../../../lib/device-management/model/device-view";
import { nextScheduledRun } from "../../../../lib/device-management/model/schedule-where";
import type {
	DeviceViewModel,
	GateResult,
} from "../../../../lib/device-management/model/types";
import { identityName } from "../access/person-name";
import { appCopy } from "../copy/app-copy";
import { scheduleRunNames, scheduledLine } from "../copy/schedule-copy";
import { VersionCell } from "../primitives/app-chips";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { CellSub, DvTable, GroupRow, Td, Th, Tr } from "../primitives/dv-table";
import {
	FreshnessStamp,
	type FreshnessStampProps,
	sameSource,
} from "../primitives/freshness-stamp";
import { InlineConfirmRow } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import { Meter } from "../primitives/meter";
import { PairedPins } from "../primitives/paired-pins";
import {
	type DesiredRun,
	type ObservedRun,
	RequestedActual,
} from "../primitives/requested-actual";
import { StateView } from "../primitives/state-view";
import {
	HealthChip,
	KeyChip,
	PresenceChip,
	RelationshipChip,
	StatusChip,
} from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { copyText } from "../primitives/use-copy";
import { useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { keyChipOf } from "../shell/keys-popover";
import {
	type ServiceCommand,
	serviceGateExtra,
	useAttentionState,
	useGates,
	useInlineResults,
	useOverlay,
	useServiceCommands,
} from "../workspace";
import {
	APP_LINKS,
	AppMenu,
	LINK,
	type MenuEntry,
	PLAIN_CHIP,
	Person,
	ShowMore,
	UnknownAction,
	useAppPage,
	useCapped,
	useDayTime,
	useDeviceNames,
	useGateText,
	useNameList,
} from "./app-shared";
import {
	type CloudState,
	EVENT_NAME_CAP,
	GROUP_CAP,
	SENTENCE_NAME_CAP,
	UNKNOWN_GROUP_CAP,
	type WritesFacts,
	blocksDeploy,
	capList,
	cloudOf,
	eventNames,
	shortMoney,
	stagedVersionOf,
	uploadOf,
	versionName,
	writesFacts,
} from "./app-view-local";
import {
	answersRequests,
	cellLines,
	eventsOfKind,
	ruleRows,
} from "./kind-lines";
import { openRunNow } from "./run-now";
import {
	type AwareInput,
	useRemoveService,
	useWhatStays,
	useWhoNotices,
} from "./use-service-actions";

export const BY_DEVICE_COLS = ["22%", "19%", "14%", "14%", "12%", "13%", "6%"];
const SPAN = BY_DEVICE_COLS.length;

const OBSERVED: readonly string[] = [
	"unknown",
	"starting",
	"running",
	"stopping",
	"stopped",
	"backoff",
	"failed",
	"removed",
];

export const desiredRun = (desired: string): DesiredRun =>
	desired === "stopped" ? "stopped" : "running";
export const observedRun = (observed: string): ObservedRun =>
	OBSERVED.includes(observed) ? (observed as ObservedRun) : "unknown";

interface Endpoint {
	address: string;
	tls: boolean;
}

/** The address a service answers on, once its settings were read live. */
export function useEndpoint(
	deviceId: string,
	serviceId: string,
): Endpoint | null {
	const { input } = useAttentionState();
	const facts = input.live[deviceId]?.placements?.[serviceId];
	const host = facts?.host;
	const port = facts?.port;
	const tls = !!facts?.tlsCertificateId;
	return useMemo(
		() => (host && port ? { address: [host, port].join(":"), tls } : null),
		[host, port, tls],
	);
}

function ServiceCell({
	row,
	names,
	endpoint,
}: Readonly<{
	row: AppServiceRow;
	names: string[] | null;
	endpoint: Endpoint | null;
}>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const link = useRouteLink();
	const rules = useMemo(() => ruleRows(view), [view]);
	const served = row.events ?? [];
	const kinds = served.map(
		(event) => rules.get(event.event_id)?.eligibility.kind,
	);
	// Services that run on their own: nothing answers a request, so there is no address to show.
	const answers = kinds.some(
		(kind) => kind !== undefined && answersRequests(kind),
	);
	const background =
		kinds.includes("background") &&
		!answers &&
		kinds.every((kind) => kind !== undefined);
	// Forms and quick actions of a service without a web endpoint run only from Devices.
	const started = !answers && kinds.includes("on_demand");
	const { shown, rest } = capList(names ?? [], EVENT_NAME_CAP);
	return (
		<>
			<span className="flex min-w-0 items-center gap-1.5">
				<PairedPins
					desired={desiredRun(row.view.desired)}
					observed={observedRun(row.view.observed)}
					conv={row.view.conv}
				/>
				<a
					{...link(APP_LINKS.service(row.deviceId, row.serviceId))}
					title={row.serviceId}
					className={cx(LINK, "min-w-0 truncate font-mono font-semibold")}
				>
					{row.serviceId}
				</a>
			</span>
			{names ? (
				<CellSub
					title={names.join(" · ")}
					className="text-ui text-ink-2"
					data-events=""
				>
					{shown.join(" · ")}
					{rest.length
						? ` ${t("app.device.moreEvents", "+{{count, number}}", { count: rest.length })}`
						: ""}
				</CellSub>
			) : (
				<CellSub data-events="unknown">
					{t(
						"app.device.eventsUnknown",
						"Events unknown: the status snapshot has no event list",
					)}
				</CellSub>
			)}
			{endpoint ? (
				<CellSub className="inline-flex max-w-full items-center gap-1 font-mono">
					{endpoint.tls ? (
						<Lock
							aria-label={t("app.device.tls", "Encrypted")}
							className="size-3 shrink-0"
						/>
					) : (
						<Link2 aria-hidden className="size-3 shrink-0" />
					)}
					<span className="truncate">{endpoint.address}</span>
				</CellSub>
			) : background ? (
				<CellSub>
					{t("app.device.background", "Runs in the background")}
				</CellSub>
			) : null}
			{started ? (
				<CellSub data-kind-line="on_demand">
					{t("app.device.onDemand", "Started by a person · from Devices")}
				</CellSub>
			) : null}
			<KindLines row={row} rules={rules} />
		</>
	);
}

/** What a service's schedules and bots do: when the repeating ones run next, each one-time schedule, each bot's state. */
function KindLines({
	row,
	rules,
}: Readonly<{ row: AppServiceRow; rules: ReadonlyMap<string, MatrixRow> }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const served = row.events ?? [];
	const scheduled = eventsOfKind(rules, served, "scheduled");
	const repeating = scheduled.filter((rule) => !rule.eligibility.once);
	const own = [
		...scheduled.filter((rule) => rule.eligibility.once),
		...eventsOfKind(rules, served, "bot"),
	];
	const next = nextScheduledRun(
		row.view,
		repeating.map((rule) => rule.eventId),
		time.nowS,
	);
	const names = (rule: MatrixRow) => () =>
		scheduleRunNames(t, {
			deviceId: row.deviceId,
			serviceId: row.serviceId,
			device: deviceName(row.deviceId),
			eventId: rule.eventId,
			...(rule.where ? { where: rule.where } : {}),
			deviceName,
			siblings: view.services
				.filter((entry) => entry.deviceId === row.deviceId)
				.map((entry) => entry.view),
		});
	return (
		<>
			{repeating.length ? (
				<CellSub data-schedule="">{scheduledLine(t, next, time)}</CellSub>
			) : null}
			{own.map((rule) => {
				const [line] = cellLines(t, rule, row.view, names(rule), time);
				if (!line) return null;
				return (
					<CellSub key={rule.eventId} data-kind-line={rule.eligibility.kind}>
						{own.length > 1
							? t("app.device.kindLine", "{{event}} · {{line}}", {
									event: rule.name,
									line,
								})
							: line}
					</CellSub>
				);
			})}
		</>
	);
}

function VersionColumn({ row }: Readonly<{ row: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const time = useAreaTime();
	const copy = appCopy(t);
	const staged = stagedVersionOf(row, view.versions);
	const upload = uploadOf(row, view.services, data.uploads);
	const newest = view.versions[0];
	const hash = row.view.appVersion?.hash;
	const label = row.version
		? versionName(row.version)
		: hash
			? hash.slice(0, 8)
			: undefined;
	const named = !!row.version?.label;
	const sub = row.staged
		? staged
			? copy.staged(versionName(staged))
			: t("app.device.updateStaged", "Update staged")
		: upload
			? upload.state === "paused"
				? t("app.device.uploadPaused", "Uploading a new version · paused")
				: t("app.device.uploading", "Uploading a new version")
			: undefined;
	const title =
		row.version && newest && row.version.builtAt && newest.builtAt
			? copy.driftTitle({
					version: versionName(row.version),
					date: time.at(row.version.builtAt),
					newest: versionName(newest),
					newestDate: time.at(newest.builtAt),
				})
			: undefined;
	return (
		<VersionCell
			{...(label ? { label } : {})}
			{...(named && hash ? { hash } : {})}
			behind={row.behind}
			{...(title ? { title } : {})}
			{...(sub ? { sub } : {})}
		/>
	);
}

interface WritesLine {
	id: string;
	text: string;
	warn: boolean;
}

/** APP §2.9 Data line 2: what waits in the write buffer, and what is paused. */
function writesLines(
	t: DevicesT,
	facts: WritesFacts | null,
	buffering: boolean | undefined,
): WritesLine[] {
	if (!facts)
		return buffering === false
			? [
					{
						id: "none",
						text: t("devices:app.device.noBuffering", "No write buffering"),
						warn: false,
					},
				]
			: [];
	if (!facts.waiting && !facts.paused)
		return [
			{
				id: "idle",
				text: t(
					"devices:app.device.writesIdle",
					"Buffers writes · nothing waiting",
				),
				warn: false,
			},
		];
	const lines: WritesLine[] = [];
	if (facts.waiting)
		lines.push({
			id: "waiting",
			text: facts.conflicts
				? t("devices:app.device.writesConflict", {
						waiting: facts.waiting,
						count: facts.conflicts,
						defaultValue_one:
							"{{waiting, number}} waiting · {{count, number}} conflict",
						defaultValue_other:
							"{{waiting, number}} waiting · {{count, number}} conflicts",
					})
				: t("devices:app.device.writesWaiting", "{{count, number}} waiting", {
						count: facts.waiting,
					}),
			warn: facts.conflicts > 0,
		});
	if (facts.paused)
		lines.push({
			id: "paused",
			text: t(
				"devices:app.device.writesPaused",
				"{{count, number}} paused: cloud access changed",
				{ count: facts.paused },
			),
			warn: true,
		});
	return lines;
}

function DataCell({ row }: Readonly<{ row: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	if (row.data.where === "device")
		return (
			<>
				<span className="inline-flex items-center gap-1">
					<HardDrive aria-hidden className="size-3.5 shrink-0 text-ink-2" />
					{t("app.device.onDevice", "On the device")}
				</span>
				<CellSub>
					{row.data.since
						? t("app.device.since", "Since {{date}}", {
								date: time.at(row.data.since),
							})
						: t("app.device.sinceUnknown", "Since: unknown")}
				</CellSub>
			</>
		);
	const live = input.live[row.deviceId];
	const settings = live?.placements?.[row.serviceId];
	const lines = writesLines(
		t,
		writesFacts(row, live?.offlineQueues?.[row.serviceId]),
		settings ? !!settings.offlineWrites : undefined,
	);
	return (
		<>
			<span className="inline-flex items-center gap-1">
				<Cloud aria-hidden className="size-3.5 shrink-0 text-ink-2" />
				{t("app.device.inCloud", "In the cloud")}
			</span>
			{lines.map((line) => (
				<CellSub
					key={line.id}
					data-writes={line.id}
					className={line.warn ? "text-warning" : undefined}
				>
					{line.text}
				</CellSub>
			))}
		</>
	);
}

/** A one-sentence cloud state; the text flows under its icon, so a 12 % column needs fewer lines. */
function CloudNote({
	icon: Icon,
	children,
}: Readonly<{ icon: typeof Info; children: ReactNode }>) {
	return (
		<span data-cloud-note="" className="block text-xs text-muted-foreground">
			<Icon aria-hidden className="mr-1 inline size-3.5 align-[-0.15em]" />
			{children}
		</span>
	);
}

function CloudCell({
	row,
	cloud,
}: Readonly<{ row: AppServiceRow; cloud: CloudState }>) {
	const { t, i18n } = useTranslation("devices");
	const locale = i18n?.language ?? "en";
	if (cloud.state === "hidden")
		return (
			<CloudNote icon={EyeOff}>
				{t(
					"app.device.cloudHidden",
					"Hidden: only the approver and the device owner see it",
				)}
			</CloudNote>
		);
	if (cloud.state === "none")
		return (
			<CloudNote icon={CloudOff}>
				{t("app.device.cloudNone", "No cloud access")}
			</CloudNote>
		);
	if (cloud.state !== "approved")
		return (
			<CloudNote icon={CircleDashed}>
				{t("app.device.cloudUnknown", "Not loaded yet")}
			</CloudNote>
		);
	const { approval } = cloud;
	const billing = approval.billing;
	const spend = billing
		? t("app.device.spend", "{{used}} of {{limit}}", {
				used: shortMoney(billing.used, locale),
				limit: shortMoney(billing.limit, locale),
			})
		: null;
	const access =
		row.mode === "offline" || approval.files === null
			? t("app.device.modelAccess", "Model access")
			: approval.files === "read_write"
				? t("app.device.filesWrite", "Files: read & write")
				: t("app.device.filesRead", "Files: read only");
	return (
		<>
			{billing && spend ? (
				<span className="flex min-w-0 flex-col gap-1">
					<span className="font-medium tabular-nums">{spend}</span>
					<Meter
						label={spend}
						className="w-15"
						segments={[
							{
								value: billing.limit ? (billing.used / billing.limit) * 100 : 0,
								tone: "neutral",
							},
							{
								value: billing.limit
									? (billing.reserved / billing.limit) * 100
									: 0,
								tone: "reserved",
							},
						]}
					/>
				</span>
			) : (
				<span className="text-xs text-muted-foreground">
					{approval.active
						? t("app.device.noLimit", "No spending limit")
						: t("app.device.approvalEnded", "Access ended")}
				</span>
			)}
			<CellSub>{access}</CellSub>
		</>
	);
}

const PHASE: Record<string, "checking" | "switching" | "restoring"> = {
	validating: "checking",
	activating: "switching",
	rolling_back: "restoring",
};

function LastChangeCell({ row }: Readonly<{ row: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const time = useAreaTime();
	const rollout = row.view.rollout;
	const phase = rollout ? PHASE[rollout.state] : undefined;
	if (rollout && phase) {
		const endsAt = rolloutEndsAt(rollout);
		const words = {
			checking: t("app.device.updateChecking", "Safe update · checking"),
			switching: t("app.device.updateSwitching", "Safe update · switching"),
			restoring: t(
				"app.device.updateRestoring",
				"Safe update · restoring the previous version",
			),
		};
		return (
			<>
				<span className="inline-flex items-start gap-1 text-info">
					<RotateCw aria-hidden className="mt-0.5 size-3.5 shrink-0" />
					{words[phase]}
				</span>
				{endsAt && endsAt > time.nowS ? (
					<CellSub className="tabular-nums">
						{t("app.device.timeLeft", "{{time}} left", {
							time: time.countdown(endsAt),
						})}
					</CellSub>
				) : null}
			</>
		);
	}
	const change = row.lastChange;
	if (!change)
		return (
			<span className="text-muted-foreground">
				{t(
					"app.device.settingsUnknownDate",
					"Settings v{{settings}} · date unknown",
					{
						settings: row.view.settings.latest,
					},
				)}
			</span>
		);
	const version = change.hash
		? view.versions.find((entry) => entry.hash === change.hash)
		: undefined;
	const detail = {
		deploy: t("app.device.changeDeploy", "Deployed"),
		update: version
			? t("app.device.changeUpdateTo", "Updated to {{version}}", {
					version: versionName(version),
				})
			: t("app.device.changeUpdate", "Updated"),
		settings: t("app.device.changeSettings", "Settings changed"),
		rollback: t("app.device.changeRollback", "Rolled back"),
	}[change.kind];
	return (
		<>
			<span className="inline-flex flex-wrap items-center gap-x-1">
				{time.at(change.at)}
				{change.by ? (
					<>
						<span aria-hidden>·</span>
						<Person userId={change.by} />
					</>
				) : null}
			</span>
			<CellSub>
				{change.settings
					? t(
							"app.device.changeWithSettings",
							"{{detail}} · settings v{{settings}}",
							{
								detail,
								settings: change.settings,
							},
						)
					: detail}
			</CellSub>
			<CellSub>
				<FreshnessStamp
					compact
					source="local"
					age="current"
					text={t("app.device.trackedHere", "tracked on this computer")}
				/>
			</CellSub>
		</>
	);
}

type Confirming = "start" | "stop" | "restart" | null;

/**
 * Why a menu entry can't run. A link into the deploy wizard isn't held back by
 * a gate the wizard clears itself (unlock, connect); a command sent from this
 * page is held back by every gate, or its confirm would end in nothing.
 */
const gateReason = (
	text: ReturnType<typeof useGateText>,
	gate: GateResult,
	wizard = false,
): string | null => {
	if (gate.ok || (wizard && !blocksDeploy(gate))) return null;
	const reason = text(gate)?.reason;
	return typeof reason === "string" ? reason : null;
};

function useRowMenu(
	row: AppServiceRow,
	commands: ReturnType<typeof useServiceCommands>,
	remove: ReturnType<typeof useRemoveService>,
	confirm: (next: Confirming) => void,
): () => MenuEntry[] {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const gateText = useGateText();
	const { deviceId, serviceId } = row;
	const target = useMemo(
		() => ({
			placementId: serviceId,
			projectId: row.view.projectId,
			labels: { service: serviceId },
			extra: serviceGateExtra(row.view),
		}),
		[serviceId, row.view],
	);
	const gates = useGates(["update_service", "run_event"], deviceId, target);
	const started = useMemo(
		() => eventsOfKind(ruleRows(view), row.events ?? [], "on_demand"),
		[view, row.events],
	);
	const newest = view.versions[0];
	return useCallback(() => {
		const change = gateReason(gateText, gates.update_service, true);
		const update = { deviceIds: [deviceId], serviceId };
		const runNow = started.map(
			(rule, index): MenuEntry => ({
				id: `run-now-${rule.eventId}`,
				label: t("app.menu.runNow", "Run {{event}} now…", {
					event: rule.name,
				}),
				icon: CirclePlay,
				blocked: gateReason(gateText, gates.run_event),
				onSelect: () =>
					openRunNow({ deviceId, serviceId, eventId: rule.eventId }),
				...(index === 0 ? { separated: true } : {}),
			}),
		);
		const lifecycle = (
			id: Exclude<Confirming, null>,
			command: ServiceCommand,
			label: string,
			icon: MenuEntry["icon"],
		): MenuEntry => ({
			id,
			label,
			icon,
			blocked: gateReason(gateText, command.gate),
			onSelect: () => confirm(id),
			...(id === "stop" ? { danger: true } : {}),
		});
		return [
			{
				id: "open-service",
				label: t("app.menu.openService", "Open service"),
				icon: Server,
				route: APP_LINKS.service(deviceId, serviceId),
			},
			{
				id: "open-device",
				label: t("app.menu.openDevice", "Open device"),
				icon: HardDrive,
				route: APP_LINKS.device(deviceId),
			},
			{
				id: "update",
				label: newest
					? t("app.menu.updateTo", "Update to {{version}}…", {
							version: versionName(newest),
						})
					: t("app.menu.update", "Update…"),
				icon: CircleArrowUp,
				route: APP_LINKS.deploy(update),
				blocked:
					row.behind === 0
						? t("app.menu.alreadyNewest", "Already runs the newest version")
						: change,
				separated: true,
			},
			{
				id: "settings",
				label: t("app.menu.changeSettings", "Change settings…"),
				icon: Settings2,
				route: APP_LINKS.deploy({ ...update, step: "settings" }),
				blocked: change,
			},
			{
				id: "add-event",
				label: t("app.menu.addEvent", "Add an event…"),
				icon: Plus,
				route: APP_LINKS.deploy({ ...update, step: "what" }),
				blocked: change,
			},
			...runNow,
			{
				...lifecycle(
					"start",
					commands.start,
					t("app.menu.start", "Start"),
					Play,
				),
				separated: true,
			},
			lifecycle(
				"restart",
				commands.restart,
				t("app.menu.restart", "Restart…"),
				RotateCw,
			),
			lifecycle("stop", commands.stop, t("app.menu.stop", "Stop…"), Square),
			{
				id: "logs",
				label: t("app.menu.logs", "Logs"),
				icon: FileText,
				route: APP_LINKS.logs(deviceId, serviceId),
				separated: true,
			},
			{
				id: "remove",
				label: t("app.menu.remove", "Remove service…"),
				icon: Trash2,
				danger: true,
				blocked:
					typeof remove.gate?.reason === "string" ? remove.gate.reason : null,
				onSelect: remove.run,
			},
			{
				id: "copy-id",
				label: t("app.menu.copyServiceId", "Copy service ID"),
				icon: Copy,
				onSelect: () => void copyText(serviceId),
				separated: true,
			},
		];
	}, [
		t,
		gateText,
		gates.update_service,
		gates.run_event,
		commands,
		remove,
		confirm,
		deviceId,
		serviceId,
		newest,
		row.behind,
		started,
	]);
}

function ServiceRows({
	row,
	deviceLabel,
	resources,
	owned,
}: Readonly<{
	row: AppServiceRow;
	deviceLabel: string;
	resources: DeviceViewModel["resources"];
	/** The viewer owns the device: the hub shows them every approval on it. */
	owned: boolean;
}>) {
	const { t } = useTranslation("devices");
	const { view, data, route } = useAppPage();
	const { input } = useAttentionState();
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const [confirming, setConfirming] = useState<Confirming>(null);
	const commands = useServiceCommands(row.deviceId, row.serviceId);
	const results = useInlineResults(commands.resultKey);
	const endpoint = useEndpoint(row.deviceId, row.serviceId);
	const names = useMemo(
		() => eventNames(row, data.eventNames),
		[row, data.eventNames],
	);
	const cloud = useMemo(
		() => cloudOf(row, data.appId, resources, input.me, owned),
		[row, data.appId, resources, input.me, owned],
	);
	const aware: AwareInput = useMemo(
		() => ({
			view,
			row,
			deviceLabel,
			deviceName,
			events: names,
			address: endpoint?.address ?? null,
		}),
		[view, row, deviceLabel, deviceName, names, endpoint?.address],
	);
	const whoNotices = useWhoNotices();
	const whatStays = useWhatStays();
	const remove = useRemoveService(
		aware,
		cloud.state === "approved" ? cloud.approval : null,
		commands,
	);
	const entries = useRowMenu(row, commands, remove, setConfirming);
	const command = confirming ? commands[confirming] : null;
	const labels = {
		service: t("app.device.colService", "Service"),
		state: t("app.device.colState", "Requested → actual"),
		version: t("app.device.colVersion", "Version"),
		data: t("app.device.colData", "Data"),
		cloud: t("app.device.colCloud", "Cloud"),
		change: t("app.device.colChange", "Last change"),
		menu: t("app.device.colMenu", "Actions"),
	};
	const highlighted =
		!!route.eventId &&
		!!row.events?.some((event) => event.event_id === route.eventId);
	const { ready, requested, max } = row.view.instances;
	return (
		<>
			<Tr
				data-service={row.serviceId}
				data-target={highlighted || undefined}
				className={
					highlighted
						? "[&>td:first-child]:border-l-2 [&>td:first-child]:border-l-ring"
						: undefined
				}
			>
				<Td label={labels.service} kind="name">
					<ServiceCell row={row} names={names} endpoint={endpoint} />
				</Td>
				<Td label={labels.state} kind="name">
					<RequestedActual
						desired={desiredRun(row.view.desired)}
						observed={observedRun(row.view.observed)}
						conv={row.view.conv}
						lastKnown={row.lastKnown}
						sub={
							row.lastKnown && row.view.freshness.at
								? t(
										"app.device.instancesLastKnown",
										"last known · {{ago}} · {{ready, number}} of {{requested, number}} ready · max {{max, number}}",
										{
											ago: time.ago(row.view.freshness.at),
											ready,
											requested,
											max,
										},
									)
								: t(
										"app.device.instances",
										"{{ready, number}} of {{requested, number}} ready · max {{max, number}}",
										{ ready, requested, max },
									)
						}
					/>
				</Td>
				<Td label={labels.version}>
					<VersionColumn row={row} />
				</Td>
				<Td label={labels.data}>
					<DataCell row={row} />
				</Td>
				<Td label={labels.cloud}>
					<CloudCell row={row} cloud={cloud} />
				</Td>
				<Td label={labels.change}>
					<LastChangeCell row={row} />
				</Td>
				<Td label={labels.menu} kind="more">
					<AppMenu
						entries={entries}
						trigger={
							<DvButton
								size="xs"
								variant="ghost"
								iconOnly
								icon={Ellipsis}
								busy={commands.start.pending}
								data-act="ad-svc-menu"
								aria-label={t("app.menu.more", "More for {{service}}", {
									service: row.serviceId,
								})}
							/>
						}
					/>
				</Td>
			</Tr>
			{command && confirming ? (
				<InlineConfirmRow
					colSpan={SPAN}
					label={command.label}
					title={command.title}
					sub={command.sub}
					rows={
						confirming === "start"
							? command.rows
							: {
									...command.rows,
									who: whoNotices(aware),
									...(confirming === "stop" ? { stays: whatStays(row) } : {}),
								}
					}
					confirmLabel={command.label}
					tone={command.tone}
					onCancel={() => setConfirming(null)}
					onConfirm={async () => {
						try {
							await command.run({ confirmed: true });
						} finally {
							setConfirming(null);
						}
					}}
				/>
			) : null}
			{results.map((result) => (
				<tr key={result.id} data-result-row="" className="hover:bg-transparent">
					<td colSpan={SPAN} className="border-t border-hairline px-4 py-2">
						<InlineResult tone={result.tone} onDismiss={result.dismiss}>
							{result.text}
						</InlineResult>
					</td>
				</tr>
			))}
			{remove.note ? (
				<tr data-remove-note="" className="hover:bg-transparent">
					<td colSpan={SPAN} className="border-t border-hairline px-4 py-2">
						<InlineResult tone={remove.note.tone} onDismiss={remove.dismiss}>
							{remove.note.text}
						</InlineResult>
					</td>
				</tr>
			) : null}
		</>
	);
}

/** The device's operating system as words; nothing until a status read told it. */
function platformLabel(t: DevicesT, os: string | undefined): string | null {
	if (os === "linux") return t("devices:app.device.platformLinux", "Linux");
	if (os === "macos") return t("devices:app.device.platformMac", "Mac");
	if (os === "windows")
		return t("devices:app.device.platformWindows", "Windows");
	return null;
}

function unknownTitle(
	t: DevicesT,
	group: AppDeviceGroup,
	app: string,
	time: AreaTime,
): { kind: "locked" | "notloaded" | "error"; title: string; text?: string } {
	const unknown = group.unknown;
	const device = group.name;
	if (unknown?.kind === "locked")
		return {
			kind: "locked",
			title: t(
				"devices:app.device.unknownLocked",
				"Unlock {{device}} to see which {{app}} services run there.",
				{ device, app },
			),
			text: t(
				"devices:app.device.unknownLockedText",
				"Until then it counts as unknown, never as not deployed.",
			),
		};
	if (unknown?.kind === "nokeys")
		return {
			kind: "notloaded",
			title: t(
				"devices:app.device.unknownNoKeys",
				"This computer has no keys for {{device}}.",
				{ device },
			),
			text: t(
				"devices:app.device.unknownNoKeysText",
				"Restore them to read which {{app}} services run there.",
				{ app },
			),
		};
	if (unknown?.kind === "offline")
		return {
			kind: "notloaded",
			title: unknown.since
				? t(
						"devices:app.device.unknownOffline",
						"No status from {{device}} since {{time}}.",
						{
							device,
							time: time.at(unknown.since),
						},
					)
				: t(
						"devices:app.device.unknownOfflineNoTime",
						"No status from {{device}}.",
						{ device },
					),
		};
	if (unknown?.kind === "error")
		return {
			kind: "error",
			title: t(
				"devices:app.device.unknownError",
				"The status of {{device}} couldn't be read.",
				{ device },
			),
		};
	return {
		kind: "notloaded",
		title: t(
			"devices:app.device.unknownNotLoaded",
			"The status of {{device}} isn't loaded yet.",
			{ device },
		),
	};
}

function DeviceGroupRows({
	group,
	base,
}: Readonly<{ group: AppDeviceGroup; base: FreshnessStampProps | null }>) {
	const { t } = useTranslation("devices");
	const { view, data, route } = useAppPage();
	const { input } = useAttentionState();
	const link = useRouteLink();
	const time = useAreaTime();
	const device = data.devices.get(group.deviceId);
	const platform = platformLabel(
		t,
		fleetFacts(input).byId.get(group.deviceId)?.inspection?.isolation?.platform,
	);
	const partial = data.coverage.partial.includes(group.deviceId);
	const rowStamps = group.services.map((row) => stampOf(row.view.freshness));
	const stamp = rowStamps[0];
	const differs = stamp && !sameSource(stamp, base);
	const chip = device ? keyChipOf(device.keys, device.live) : null;
	const highlighted = route.focusDeviceId === group.deviceId;
	const state = group.unknown
		? unknownTitle(t, group, view.app.name, time)
		: null;
	return (
		<>
			<GroupRow
				colSpan={SPAN}
				className={highlighted ? "border-l-2 border-l-ring" : undefined}
			>
				<a
					{...link(APP_LINKS.device(group.deviceId))}
					data-device={group.deviceId}
					className={cx(LINK, "font-mono text-ui font-semibold")}
				>
					{group.name}
				</a>
				<PresenceChip
					kind={group.presence.kind}
					{...(group.presence.since === undefined
						? {}
						: { since: group.presence.since })}
					short
					className={PLAIN_CHIP}
				/>
				{device && device.health !== "healthy" && !group.unknown ? (
					<HealthChip level={device.health} />
				) : null}
				{chip ? (
					<KeyChip state={chip.state} transport={chip.transport} />
				) : null}
				{device && group.relationship === "shared" ? (
					<GroupOwner device={device} />
				) : null}
				{partial ? (
					<StatusChip tone="outline" icon={Eye}>
						{t("app.device.partial", "You see {{app}} only", {
							app: view.app.name,
						})}
					</StatusChip>
				) : null}
				{platform ? (
					<span data-platform="" className="text-xs text-muted-foreground">
						{platform}
					</span>
				) : null}
				<span className="flex-1" />
				{differs && stamp ? <FreshnessStamp {...stamp} compact /> : null}
			</GroupRow>
			{state && group.unknown ? (
				<tr data-state-row="" className="hover:bg-transparent">
					<td colSpan={SPAN} className="border-t border-hairline px-4 py-2.5">
						<StateView
							kind={state.kind}
							title={state.title}
							text={state.text}
							actions={
								<UnknownAction
									deviceId={group.deviceId}
									unknown={group.unknown}
								/>
							}
						/>
					</td>
				</tr>
			) : null}
			{group.services.map((row) => (
				<ServiceRows
					key={row.serviceId}
					row={row}
					deviceLabel={group.name}
					resources={device?.resources}
					owned={group.relationship === "owner"}
				/>
			))}
		</>
	);
}

/** "Shared by {owner} · ends …" on a device head, in the page's table and in the Update everywhere list. */
export function GroupOwner({ device }: Readonly<{ device: DeviceViewModel }>) {
	const ownerId = device.row.owner_id;
	const ownerName = identityName(useUserIdentity(ownerId), ownerId);
	return (
		<RelationshipChip
			relationship="shared"
			{...(ownerName ? { ownerName } : {})}
			{...(typeof device.row.access_expires_at === "number"
				? { endsAt: device.row.access_expires_at }
				: {})}
		/>
	);
}

function FootLine({
	icon: Icon,
	children,
}: Readonly<{ icon: typeof Info; children: ReactNode }>) {
	return (
		<p className="flex w-full items-start gap-1.5 text-xs">
			<Icon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span className="min-w-0">{children}</span>
		</p>
	);
}

/** The three foot lines of APP §2.9, each only when true. */
export function WhereFoot() {
	const { t } = useTranslation("devices");
	const { view, openUpdateAll, updateAllGate } = useAppPage();
	const dayTime = useDayTime();
	const copy = appCopy(t);
	const runs = view.newestRuns;
	const newest = runs?.version;
	const behind = !!runs && runs.services < runs.of;
	return (
		<div data-where-foot="" className="flex w-full flex-col gap-1.5">
			{newest && runs && behind && newest.builtAt ? (
				<FootLine icon={CircleArrowUp}>
					{copy.versionFoot({
						version: versionName(newest),
						hash: newest.unpublished ? copy.currentEdits() : newest.short,
						when: dayTime(newest.builtAt),
						mode: view.app.mode,
						running: runs.services,
						total: runs.of,
						// A service whose version can't be told may run the newest one: never "isn't running anywhere".
						...(runs.unknown ? { unknown: runs.unknown } : {}),
					})}{" "}
					{updateAllGate ? null : (
						<DvButton
							variant="link"
							size="xs"
							data-act="ad-update-all"
							onClick={openUpdateAll}
						>
							{t("app.header.updateAll", "Update everywhere…")}
						</DvButton>
					)}
				</FootLine>
			) : null}
			<FootLine icon={Info}>{copy.publishNote(view.app.mode)}</FootLine>
			<p className="flex w-full items-start gap-1.5 text-xs">
				<PairedPins
					desired="running"
					observed="running"
					conv="converged"
					title
					className="mt-0.5"
				/>
				<span className="min-w-0">
					{t(
						"app.device.footPins",
						"Requested is what you asked for; actual is what the device reports. The agent restarts crashed instances on its own, up to 5 times.",
					)}
				</span>
			</p>
		</div>
	);
}

/** APP §2.9: the services of the app, grouped by device, worst first. */
export function WhereByDevice({
	base,
}: Readonly<{ base: FreshnessStampProps | null }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const overlay = useOverlay();
	const unknown = view.groups.filter((group) => group.unknown);
	const shownUnknown = unknown.length > UNKNOWN_GROUP_CAP ? [] : unknown;
	const collapsed = unknown.length > UNKNOWN_GROUP_CAP ? unknown : [];
	const ordered = view.groups.filter(
		(group) => !group.unknown || shownUnknown.includes(group),
	);
	const { shown, rest, showAll } = useCapped(ordered, GROUP_CAP);
	const nameList = useNameList();
	const lockable = collapsed.some((group) => group.unknown?.kind === "locked");
	return (
		<>
			<DvTable
				label={t("app.device.tableLabel", "Services of {{app}} by device", {
					app: view.app.name,
				})}
				cols={BY_DEVICE_COLS}
				head={
					<tr>
						<Th>{t("app.device.colService", "Service")}</Th>
						<Th>{t("app.device.colState", "Requested → actual")}</Th>
						<Th>{t("app.device.colVersion", "Version")}</Th>
						<Th>{t("app.device.colData", "Data")}</Th>
						<Th>{t("app.device.colCloud", "Cloud")}</Th>
						<Th>{t("app.device.colChange", "Last change")}</Th>
						<Th>
							<span className="sr-only">
								{t("app.device.colMenu", "Actions")}
							</span>
						</Th>
					</tr>
				}
			>
				{shown.map((group) => (
					<DeviceGroupRows key={group.deviceId} group={group} base={base} />
				))}
				{collapsed.length ? (
					<tr data-collapsed-unknown="" className="hover:bg-transparent">
						<td colSpan={SPAN} className="border-t border-hairline px-4 py-2.5">
							<StateView
								kind="locked"
								title={t("app.device.moreUnknown", {
									count: collapsed.length,
									defaultValue_one:
										"{{count, number}} more device · status unknown",
									defaultValue_other:
										"{{count, number}} more devices · status unknown",
								})}
								text={t(
									"app.device.moreUnknownText",
									"Which {{app}} services run on {{names}} isn't readable on this computer yet. They count as unknown, never as not deployed.",
									{
										app: view.app.name,
										names: nameList(
											collapsed.map((group) => group.name),
											SENTENCE_NAME_CAP,
										),
									},
								)}
								actions={
									lockable ? (
										<DvButton
											size="sm"
											icon={LockOpen}
											data-act="unlock-several"
											onClick={() => overlay.openUnlockSeveral()}
										>
											{t("app.coverage.unlockSeveral", "Unlock several…")}
										</DvButton>
									) : undefined
								}
							/>
						</td>
					</tr>
				) : null}
			</DvTable>
			{rest.length ? (
				<p
					data-capped=""
					className="flex flex-wrap items-center gap-x-3 gap-y-1 border-t border-hairline px-4 py-2 text-xs text-muted-foreground"
				>
					<span>
						{t(
							"app.device.capped",
							"Showing the {{shown, number}} most severe of {{total, number}} devices",
							{ shown: shown.length, total: ordered.length },
						)}
					</span>
					<ShowMore
						count={rest.length}
						onClick={showAll}
						act="ad-more-devices"
					/>
				</p>
			) : null}
		</>
	);
}
