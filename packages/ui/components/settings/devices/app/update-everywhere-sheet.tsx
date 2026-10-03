"use client";

import { useTranslation } from "@flow-like/locales";
import { Cloud, HardDrive, Layers, Lock, LockOpen } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	AppDeviceGroup,
	AppView,
} from "../../../../lib/device-management/model/app-plan";
import {
	type DeployPhase,
	type DeployPlan,
	type PlanDevice,
	type PlanFacts,
	checkPlan,
	makePlan,
	resolvePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRunRow,
	DeployRunState,
} from "../../../../lib/device-management/model/deploy-run";
import { deviceName as nameOfRow } from "../../../../lib/device-management/model/device-view";
import { Checkbox } from "../../../ui/checkbox";
import { appCopy } from "../copy/app-copy";
import { flowLabel, runRefusalText } from "../deploy/deploy-copy";
import { useFlowNames } from "../deploy/use-deploy-reads";
import { type DeployRunHandle, useDeployRun } from "../deploy/use-deploy-run";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { ConsequencePreview } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	FleetRollout,
	type FleetRolloutRow,
	type FleetRowState,
} from "../primitives/fleet-rollout";
import { CheckField } from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateInline } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { PresenceChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { WizardStepper } from "../primitives/wizard";
import { useDeviceWorkspace, useOverlay } from "../workspace";
import {
	APP_LINKS,
	LinkButton,
	PLAIN_CHIP,
	useAppPage,
	useDayTime,
	useDeviceNames,
	useGateText,
} from "./app-shared";
import {
	type UpdateRow,
	blocksDeploy,
	flowsToPublish,
	pinText,
	updateBatches,
	updateRows,
	versionName,
} from "./app-view-local";
import type { AppDevicesData } from "./use-app-devices";
import { GroupOwner } from "./where-by-device";

const CHECK =
	"mt-0.5 border-border-strong shadow-none data-[state=checked]:border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

export interface UpdateRunSpec {
	plans: DeployPlan[];
	oneAtATime: boolean;
	stopOnFail: boolean;
	/** "v1.4.0" when every chosen service runs the same version. */
	from: string | null;
	to: string;
}

/** What the plan model needs to know about the app and each device (APP §3.3). */
export function planFacts(
	data: AppDevicesData,
	now: number,
	platform: "desktop" | "web",
): PlanFacts {
	const devices: Record<string, PlanDevice> = {};
	for (const [id, device] of data.devices) {
		if (device.presence.kind === "revoked") continue;
		const gate = data.deployGates.get(id);
		devices[id] = {
			id,
			name: nameOfRow(device.row),
			gate: blocksDeploy(gate) ? gate : null,
			locked: device.keys.state !== "unlocked",
			services: Array.isArray(device.services)
				? device.services.map((service) => ({
						serviceId: service.serviceId,
						projectId: service.projectId,
						events: service.events?.map((event) => event.event_id) ?? null,
						desired: service.desired,
						maxInstances: service.instances.max,
					}))
				: null,
			...(device.features ? { features: device.features } : {}),
		};
	}
	return {
		app: data.app
			? {
					id: data.app.id,
					name: data.app.name,
					visibility: data.app.visibility,
					events: data.app.events,
				}
			: null,
		devices,
		platform,
		now,
		...(data.role.known ? { isAppOwner: data.role.isOwner } : {}),
		hub: data.hub,
		...(data.view ? { schedules: data.view.schedules } : {}),
	};
}

/** One version-only update plan per batch: every service keeps the events it serves now. */
export function updatePlans(
	rows: readonly UpdateRow[],
	facts: PlanFacts,
	options: { oneAtATime: boolean; stopOnFail: boolean; accept: boolean },
	/** One deployment id per batch; the same id for the same batch keeps a run attached across renders. */
	idOf: (batch: number) => string,
): DeployPlan[] {
	const appId = facts.app?.id;
	if (!appId) return [];
	return updateBatches(rows).map((batch, index) => {
		const draft = makePlan({
			scope: { kind: "app", appId },
			route: { deviceIds: batch.map((row) => row.deviceId), mode: "update" },
			app: facts.app,
			deploymentId: idOf(index),
			now: facts.now,
			...(facts.hub ? { hub: facts.hub } : {}),
		});
		return resolvePlan(
			{
				...draft,
				order: options.oneAtATime ? "one" : "all",
				stopOnFail: options.stopOnFail,
				acceptRemovedEvents: options.accept,
				targets: draft.targets.map((target) => {
					const row = batch.find((entry) => entry.deviceId === target.deviceId);
					return row
						? {
								...target,
								choices: {
									main: { kind: "update" as const, serviceId: row.serviceId },
								},
							}
						: target;
				}),
			},
			facts,
		);
	});
}

const keyOf = (row: Pick<UpdateRow, "deviceId" | "serviceId">) =>
	`${row.deviceId}/${row.serviceId}`;

type Step = "choose" | "strategy" | "review";
const STEPS: readonly Step[] = ["choose", "strategy", "review"];

/** "v1.4.0 → v1.5.0 · Extract invoice 1.4.0 → 1.5.0 · keeps settings v12": what the update changes on one service. */
function useChangeLine(): (row: UpdateRow) => string {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	return useCallback(
		(row) => {
			const newest = view.versions[0];
			if (!newest) return "";
			const service = view.services.find(
				(entry) =>
					entry.deviceId === row.deviceId && entry.serviceId === row.serviceId,
			);
			const pins = new Map(newest.pins.map((pin) => [pin.eventId, pin]));
			const moved = (service?.events ?? []).flatMap((event) => {
				const pin = pins.get(event.event_id);
				const from = pinText(event.event_version);
				const to = pin ? pinText(pin.eventVersion) : from;
				return from === to
					? []
					: [
							t("app.updateAll.pinMove", "{{event}} {{from}} → {{to}}", {
								event: data.eventNames.get(event.event_id) ?? event.event_id,
								from,
								to,
							}),
						];
			});
			return [
				row.from
					? t("app.updateAll.fromTo", "{{from}} → {{to}}", {
							from: versionName(row.from),
							to: versionName(newest),
						})
					: t("app.updateAll.unknownTo", "version unknown → {{to}}", {
							to: versionName(newest),
						}),
				...moved,
				...(service
					? [
							t(
								"app.updateAll.keepsSettings",
								"keeps settings v{{version, number}}",
								{ version: service.view.settings.latest },
							),
						]
					: []),
			].join(" · ");
		},
		[t, view, data.eventNames],
	);
}

/** Why a service can't join now: its own update, or the device's deploy gate. */
function useBlockedReason(): (row: UpdateRow) => ReactNode {
	const { t } = useTranslation("devices");
	const { data } = useAppPage();
	const time = useAreaTime();
	const gateText = useGateText();
	return (row) => {
		if (row.blocked === "staged")
			return t(
				"app.updateAll.staged",
				"An update is staged. Activate or discard it first.",
			);
		if (row.blocked === "busy")
			return row.busyUntil
				? t(
						"app.updateAll.busyUntil",
						"An update is already running. It can join once that finishes (by {{time}}).",
						{ time: time.clock(row.busyUntil) },
					)
				: t(
						"app.updateAll.busy",
						"An update is already running. It can join once that finishes.",
					);
		const gate = data.deployGates.get(row.deviceId);
		return blocksDeploy(gate) ? (gateText(gate)?.reason ?? null) : null;
	};
}

const GROUP_ROW =
	"flex items-start gap-2.5 border-t border-hairline px-3 py-2 first:border-t-0";

/** One device of the list: its head row, then its services; a locked device keeps its place and says how to include it. */
function DeviceServices({
	group,
	rows,
	picked,
	onToggle,
}: Readonly<{
	group: AppDeviceGroup;
	rows: readonly UpdateRow[];
	picked: ReadonlySet<string>;
	onToggle(key: string, on: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const overlay = useOverlay();
	const changeLine = useChangeLine();
	const blockedReason = useBlockedReason();
	const device = data.devices.get(group.deviceId);
	return (
		<li
			data-update-group={group.deviceId}
			className="border-t border-border first:border-t-0"
		>
			<p className="flex flex-wrap items-center gap-x-2.5 gap-y-1 border-b border-hairline bg-surface-sunken px-3 py-1.5">
				<span className="font-mono text-ui font-semibold">{group.name}</span>
				<PresenceChip
					kind={group.presence.kind}
					{...(group.presence.since === undefined
						? {}
						: { since: group.presence.since })}
					short
					className={PLAIN_CHIP}
				/>
				{device && group.relationship === "shared" ? (
					<GroupOwner device={device} />
				) : null}
			</p>
			<ul className="m-0 flex list-none flex-col p-0">
				{rows.map((row) => {
					const key = keyOf(row);
					const blocked = blockedReason(row);
					return (
						<li key={key} data-update-row={key} className={GROUP_ROW}>
							<Checkbox
								checked={picked.has(key) && !blocked}
								disabled={!!blocked}
								onCheckedChange={(next) => onToggle(key, next === true)}
								aria-label={t(
									"app.updateAll.select",
									"Update {{service}} on {{device}}",
									{ service: row.serviceId, device: group.name },
								)}
								className={CHECK}
							/>
							<div className="flex min-w-0 flex-col gap-0.5">
								<p className="font-mono text-ui font-semibold">
									{row.serviceId}
								</p>
								<p className="text-xs text-muted-foreground">
									{changeLine(row)}
								</p>
								{blocked ? (
									<GateInline kind="busy" className="max-w-[60ch]">
										{blocked}
									</GateInline>
								) : null}
							</div>
						</li>
					);
				})}
				{group.unknown?.kind === "locked" ? (
					<li data-update-locked={group.deviceId} className={GROUP_ROW}>
						<Checkbox
							checked={false}
							disabled
							aria-label={t(
								"app.updateAll.lockedSelect",
								"The services on {{device}} can be ticked once it is unlocked",
								{ device: group.name },
							)}
							className={CHECK}
						/>
						<div className="flex min-w-0 flex-1 flex-col gap-0.5">
							<p className="text-ui">
								{t(
									"app.updateAll.lockedTitle",
									"Which {{app}} services run here isn't readable yet",
									{ app: view.app.name },
								)}
							</p>
							<p className="inline-flex items-center gap-1 text-xs text-muted-foreground">
								<Lock aria-hidden className="size-3 shrink-0" />
								{t("app.updateAll.lockedText", "Locked. Unlock to include it.")}
							</p>
						</div>
						<DvButton
							size="sm"
							icon={LockOpen}
							onClick={() => overlay.openUnlock(group.deviceId)}
						>
							{t("app.unknown.unlock", "Unlock…")}
						</DvButton>
					</li>
				) : null}
			</ul>
		</li>
	);
}

function ChooseStep({
	rows,
	picked,
	onToggle,
}: Readonly<{
	rows: readonly UpdateRow[];
	picked: ReadonlySet<string>;
	onToggle(key: string, on: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const dayTime = useDayTime();
	const listId = useId();
	const newest = view.versions[0];
	const groups = useMemo(
		() =>
			view.groups.flatMap((group) => {
				const own = rows.filter((row) => row.deviceId === group.deviceId);
				return own.length || group.unknown?.kind === "locked"
					? [{ group, rows: own }]
					: [];
			}),
		[view.groups, rows],
	);
	const ticked = rows.filter((row) => picked.has(keyOf(row)));
	const ModeIcon = view.app.localOnly ? HardDrive : Cloud;
	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col gap-1">
				<p className="text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
					{t("app.updateAll.version", "New version")}
				</p>
				<p className="text-ui">
					{newest ? (
						<>
							<span className="font-mono font-semibold">
								{versionName(newest)}
							</span>
							{newest.builtAt
								? view.app.localOnly
									? ` · ${t("app.updateAll.changed", "changed on this computer {{when}}", { when: dayTime(newest.builtAt) })}`
									: ` · ${t("app.updateAll.built", "built {{when}}", { when: dayTime(newest.builtAt) })}`
								: ""}
						</>
					) : null}
				</p>
				<p className="text-xs text-muted-foreground">
					{t(
						"app.updateAll.newestOnly",
						"Only the newest version can be prepared. Devices that have an older one keep it until you update them.",
					)}
				</p>
			</div>
			<p className="flex items-start gap-2.5 rounded-lg border border-border bg-surface-sunken px-3 py-2.5 text-ui">
				<ModeIcon
					aria-hidden
					className="mt-0.5 size-4 shrink-0 text-muted-foreground"
				/>
				<span className="min-w-0">
					{view.app.localOnly
						? t(
								"app.updateAll.modeOffline",
								"Sends a new copy of the app from this computer to each device. The data on each device is kept.",
							)
						: t(
								"app.updateAll.modeOnline",
								"Re-pins each service to the event and flow versions published now. Data isn't touched.",
							)}
				</span>
			</p>
			<p
				id={listId}
				className="-mb-2 text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase"
			>
				{t("app.updateAll.services", "Services per device")}
			</p>
			<ul
				aria-labelledby={listId}
				className="m-0 flex list-none flex-col overflow-hidden rounded-lg border border-border p-0"
			>
				{groups.map(({ group, rows: own }) => (
					<DeviceServices
						key={group.deviceId}
						group={group}
						rows={own}
						picked={picked}
						onToggle={onToggle}
					/>
				))}
			</ul>
			<LinkButton
				route={APP_LINKS.deploy({
					mode: "update",
					deviceIds: [...new Set(ticked.map((row) => row.deviceId))],
				})}
				variant="link"
				size="xs"
				act="update-in-wizard"
				className="self-start"
			>
				{t("app.updateAll.inWizard", "Change settings or events too…")}
			</LinkButton>
		</div>
	);
}

function StrategyStep({
	oneAtATime,
	stopOnFail,
	onChange,
}: Readonly<{
	oneAtATime: boolean;
	stopOnFail: boolean;
	onChange(patch: { oneAtATime?: boolean; stopOnFail?: boolean }): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div className="flex flex-col gap-3">
			<p className="max-w-[72ch] text-ui text-ink-2">
				{t(
					"app.updateAll.safe",
					"Safe update: each device starts the new version next to the current one, checks that it stays healthy for 10 s within 2 min, then switches. If it isn't healthy, the device restores the current version on its own. A service that is stopped or can't be checked is updated with a short stop instead.",
				)}
			</p>
			<CheckField
				id="app-update-one"
				checked={oneAtATime}
				onCheckedChange={(next) => onChange({ oneAtATime: next })}
			>
				{t("app.updateAll.oneAtATime", "One device at a time")}
			</CheckField>
			<CheckField
				id="app-update-stop"
				checked={stopOnFail}
				onCheckedChange={(next) => onChange({ stopOnFail: next })}
			>
				{t(
					"app.updateAll.stopOnFail",
					"Stop when a device fails or rolls back",
				)}
			</CheckField>
		</div>
	);
}

function removedEvents(plans: readonly DeployPlan[]): string[] {
	return plans.flatMap((plan) =>
		plan.targets.flatMap((target) =>
			target.services.flatMap((service) => service.removedEvents),
		),
	);
}

/** APP §2.16: choose the services, the strategy, review, then the run shows on the page and in the tray. */
export function UpdateEverywhereSheet({
	open,
	onOpenChange,
	onStart,
}: Readonly<{
	open: boolean;
	onOpenChange(open: boolean): void;
	onStart(spec: UpdateRunSpec): void;
}>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const workspace = useDeviceWorkspace();
	const copy = appCopy(t);
	const rows = useMemo(() => updateRows(view), [view]);
	const [step, setStep] = useState<Step>("choose");
	const [unpicked, setUnpicked] = useState<ReadonlySet<string>>(new Set());
	const [options, setOptions] = useState({
		oneAtATime: true,
		stopOnFail: true,
	});
	const [accept, setAccept] = useState(false);
	const ids = useRef<string[]>([]);
	useEffect(() => {
		if (!open) return;
		ids.current = [];
		setStep("choose");
		setUnpicked(new Set());
		setAccept(false);
	}, [open]);
	const idOf = useCallback((batch: number) => {
		ids.current[batch] ??= crypto.randomUUID();
		return ids.current[batch];
	}, []);
	const usable = useMemo(
		() =>
			rows.filter(
				(row) =>
					!row.blocked && !blocksDeploy(data.deployGates.get(row.deviceId)),
			),
		[rows, data.deployGates],
	);
	const picked = useMemo(
		() => new Set(usable.map(keyOf).filter((key) => !unpicked.has(key))),
		[usable, unpicked],
	);
	const ticked = useMemo(
		() => usable.filter((row) => picked.has(keyOf(row))),
		[usable, picked],
	);
	const newest = view.versions[0];
	const newestName = newest ? versionName(newest) : "";
	const fromNames = [
		...new Set(ticked.map((row) => (row.from ? versionName(row.from) : ""))),
	];
	const from = fromNames.length === 1 && fromNames[0] ? fromNames[0] : null;
	const facts = useMemo(
		() =>
			planFacts(
				data,
				Math.floor(workspace.clock.now() / 1000),
				workspace.deps.platform,
			),
		[data, workspace],
	);
	const plans = useMemo(
		() =>
			step === "review"
				? updatePlans(ticked, facts, { ...options, accept }, idOf)
				: [],
		[step, ticked, facts, options, accept, idOf],
	);
	const checks = plans.map((plan) => checkPlan(plan, facts));
	const removed = removedEvents(plans);
	// No flow version is created without this sentence having been on screen first.
	const publishes = useMemo(() => flowsToPublish(plans), [plans]);
	const flowNames = useFlowNames(data.appId, publishes.length > 0);
	const [firstPlan] = plans;
	// A flow version this update creates makes an app version that has no name yet.
	const to = publishes.length
		? t("app.updateAll.toNow", "the app as it is now")
		: newestName;
	const createsFlows =
		firstPlan && publishes.length
			? t("app.updateAll.createsFlow", {
					count: publishes.length,
					flows: new Intl.ListFormat(undefined, { type: "conjunction" }).format(
						publishes.map((boardId) =>
							flowLabel(t, firstPlan, flowNames, boardId),
						),
					),
					defaultValue_one:
						"Starting it creates a flow version of {{flows}} from the current edits. The version stays in the flow's history.",
					defaultValue_other:
						"Starting it creates flow versions of {{flows}} from the current edits. They stay in the flows' history.",
				})
			: null;
	const blocking = checks
		.map((check) => check.firstBlocking)
		.find((issue) => issue && issue.code !== "removed_events");
	const ready =
		plans.length > 0 && !blocking && (removed.length === 0 || accept);
	const index = STEPS.indexOf(step);
	const stepNames = [
		t("app.updateAll.stepChoose", "Choose"),
		t("app.updateAll.stepStrategy", "Strategy"),
		t("app.updateAll.stepReview", "Review"),
	];
	const services = ticked.length;
	const list = new Intl.ListFormat(undefined, { type: "conjunction" }).format(
		ticked.map((row) => row.serviceId),
	);
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			wide
			icon={Layers}
			title={t("app.updateAll.title", "Update {{app}} everywhere", {
				app: view.app.name,
			})}
			sub={t(
				"app.updateAll.sub",
				"Every service of {{app}} you can see · one version for all",
				{ app: view.app.name },
			)}
			{...(index > 0 ? { onBack: () => setStep(STEPS[index - 1]) } : {})}
			footNote={
				services
					? t(
							"app.updateAll.stepNamed",
							"Step {{step, number}} of 3 · {{name}}",
							{
								step: index + 1,
								name: stepNames[index],
							},
						)
					: t(
							"app.updateAll.stepNone",
							"Step {{step, number}} of 3 · Tick at least one service to go on.",
							{ step: index + 1 },
						)
			}
			foot={
				<>
					<DvButton onClick={() => onOpenChange(false)}>
						{t("app.updateAll.cancel", "Cancel")}
					</DvButton>
					{step === "review" ? (
						<DvButton
							variant="primary"
							icon={Layers}
							data-act="update-all-start"
							aria-disabled={!ready || undefined}
							onClick={() => {
								if (!ready) return;
								onStart({ plans, ...options, from, to });
								onOpenChange(false);
							}}
						>
							{t("app.updateAll.start", {
								count: services,
								defaultValue_one: "Update {{count, number}} service",
								defaultValue_other: "Update {{count, number}} services",
							})}
						</DvButton>
					) : (
						<DvButton
							variant="primary"
							data-act="update-all-next"
							aria-disabled={!services || undefined}
							onClick={() => services && setStep(STEPS[index + 1])}
						>
							{step === "choose"
								? t("app.updateAll.toStrategy", "Next: strategy")
								: t("app.updateAll.toReview", "Next: review")}
						</DvButton>
					)}
				</>
			}
		>
			<WizardStepper
				steps={stepNames}
				current={index}
				label={t("app.updateAll.steps", "Steps of the update")}
			/>
			{step === "choose" ? (
				<ChooseStep
					rows={rows}
					picked={picked}
					onToggle={(key, on) =>
						setUnpicked((current) => {
							const next = new Set(current);
							if (on) next.delete(key);
							else next.add(key);
							return next;
						})
					}
				/>
			) : null}
			{step === "strategy" ? (
				<StrategyStep
					{...options}
					onChange={(patch) =>
						setOptions((current) => ({ ...current, ...patch }))
					}
				/>
			) : null}
			{step === "review" ? (
				<div className="flex flex-col gap-3">
					<ConsequencePreview
						rows={{
							what: (
								<>
									{t("app.updateAll.what", {
										count: services,
										to,
										list,
										defaultValue_one:
											"{{list}} switches to {{to}} with a safe update.",
										defaultValue_other:
											"{{count, number}} services switch to {{to}} with a safe update: {{list}}.",
									})}
									{createsFlows ? (
										<span data-creates-flow=""> {createsFlows}</span>
									) : null}
								</>
							),
							who: t(
								"app.updateAll.who",
								"The current version keeps answering until the new one is healthy.",
							),
							stays: view.app.localOnly
								? t(
										"app.updateAll.staysOffline",
										"The data on each device stays as it is; the tables in the new copy aren't used.",
									)
								: t(
										"app.updateAll.staysOnline",
										"Data stays in the cloud. Cloud access and spending limits stay.",
									),
							when: options.oneAtATime
								? t(
										"app.updateAll.whenOne",
										"One device at a time. Each device checks the new version for up to 2 min.",
									)
								: t(
										"app.updateAll.whenAll",
										"All devices at once. Each device checks the new version for up to 2 min.",
									),
							undo: {
								reversible: true,
								text: t(
									"app.updateAll.undo",
									"Each device can roll back on its own; a finished update can be replaced by another update.",
								),
							},
						}}
					/>
					<p className="text-xs text-muted-foreground">
						{copy.publishNote(view.app.mode)}
					</p>
					{removed.length ? (
						<Banner
							tone="warning"
							title={t("app.updateAll.removedTitle", {
								count: new Set(removed).size,
								defaultValue_one:
									"The newest version no longer has {{count, number}} event these services serve.",
								defaultValue_other:
									"The newest version no longer has {{count, number}} events these services serve.",
							})}
						>
							<CheckField
								id="app-update-accept"
								checked={accept}
								onCheckedChange={setAccept}
							>
								{t(
									"app.updateAll.removedAccept",
									"Update anyway and stop serving them",
								)}
							</CheckField>
						</Banner>
					) : null}
					{blocking ? (
						<Banner
							tone="warning"
							title={t(
								"app.updateAll.blocked",
								"This update can't start from here.",
							)}
						>
							{t(
								"app.updateAll.blockedText",
								"A service needs a choice the quick path doesn't offer. Use “Change settings or events too…” on the first step to open the deploy flow.",
							)}
						</Banner>
					) : null}
				</div>
			) : null}
		</DvSheet>
	);
}

/* Progress (APP §2.16, §7.8). */

function phaseLabels(t: DevicesT): Record<DeployPhase, string> {
	return {
		approve: t("devices:app.updateAll.phase.approve", "Cloud access"),
		spending: t("devices:app.updateAll.phase.spending", "Spending limit"),
		upload: t("devices:app.updateAll.phase.upload", "Sending the new version"),
		check_events: t(
			"devices:app.updateAll.phase.checkEvents",
			"Checking events",
		),
		install: t("devices:app.updateAll.phase.install", "Installing"),
		schedules: t("devices:app.updateAll.phase.schedules", "Moving schedules"),
		create: t("devices:app.updateAll.phase.create", "Creating"),
		secrets: t("devices:app.updateAll.phase.secrets", "Secrets"),
		start: t("devices:app.updateAll.phase.start", "Starting"),
		prepare_update: t("devices:app.updateAll.phase.prepare", "Preparing"),
		check_new: t(
			"devices:app.updateAll.phase.checkNew",
			"Checking the new version",
		),
		switch: t("devices:app.updateAll.phase.switch", "Switching over"),
		stop: t("devices:app.updateAll.phase.stop", "Stopping"),
	};
}

const rowState = (row: DeployRunRow): FleetRowState =>
	row.state === "failed" && row.error?.rolledBack ? "rolled_back" : row.state;

interface PlanRunnerProps {
	index: number;
	plan: DeployPlan;
	title: { code: string; params: Record<string, string | number> };
	spec: Pick<UpdateRunSpec, "oneAtATime" | "stopOnFail">;
	start: boolean;
	onHandle(index: number, handle: DeployRunHandle): void;
}

function PlanRunner(props: Readonly<PlanRunnerProps>) {
	const { index, plan, title, spec, start, onHandle } = props;
	const handle = useDeployRun(plan, {
		title,
		oneAtATime: spec.oneAtATime,
		stopOnFail: spec.stopOnFail,
	});
	const started = useRef(false);
	useEffect(() => {
		if (!start || started.current || handle.state.status !== "idle") return;
		started.current = true;
		handle.start();
	}, [start, handle]);
	useEffect(() => onHandle(index, handle), [index, handle, onHandle]);
	return null;
}

const failedIn = (state: DeployRunState) =>
	state.rows.some((row) => row.state === "failed");

/** What a run row tells about itself, apart from its state and the versions. */
function rowFacts(
	t: DevicesT,
	row: DeployRunRow,
	labels: Record<DeployPhase, string>,
): Pick<
	FleetRolloutRow,
	"service" | "phases" | "step" | "at" | "reason" | "failedWhile" | "progress"
> {
	const at = row.finishedAt ?? row.at;
	// A schedule or a Latest event that stopped the run has its own sentence; anything else is the device's.
	const reason = row.error
		? (runRefusalText(t, row.error) ?? row.error.detail)
		: undefined;
	return {
		...(row.serviceId ? { service: row.serviceId } : {}),
		phases: row.phases.map((phase) => labels[phase]),
		step: Math.min(row.phase, Math.max(0, row.phases.length - 1)),
		...(at ? { at } : {}),
		...(reason ? { reason } : {}),
		...(row.error
			? { failedWhile: labels[row.error.phase].toLowerCase() }
			: {}),
		...(row.progress ? { progress: row.progress } : {}),
	};
}

/** The one thing that moves a row on: retry or continue, unlock, or open the updated service. */
function RunRowActions({
	row,
	state,
	handle,
}: Readonly<{
	row: DeployRunRow;
	state: FleetRowState;
	handle: DeployRunHandle;
}>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	if (state === "failed" || state === "rolled_back")
		return (
			<>
				<DvButton size="xs" onClick={() => handle.retry(row.target)}>
					{t("app.updateAll.retry", "Retry this device")}
				</DvButton>
				{handle.state.status === "held" ? (
					<DvButton
						size="xs"
						variant="ghost"
						onClick={() => handle.skip(row.target)}
					>
						{t("app.updateAll.continue", "Continue with the rest")}
					</DvButton>
				) : null}
			</>
		);
	if (state === "blocked")
		return (
			<DvButton
				size="xs"
				icon={LockOpen}
				onClick={() => overlay.openUnlock(row.deviceId, { connectLive: true })}
			>
				{t("app.unknown.unlock", "Unlock…")}
			</DvButton>
		);
	if (state === "done" && row.serviceId)
		return (
			<LinkButton
				route={APP_LINKS.service(row.deviceId, row.serviceId)}
				size="xs"
				variant="ghost"
			>
				{t("app.updateAll.open", "Open")}
			</LinkButton>
		);
	return null;
}

/**
 * The running Update everywhere as one rollout block. Each plan runs through
 * `useDeployRun`; a second plan (a device with two services of the app)
 * starts when the one before it finished.
 */
export function UpdateRun({
	spec,
	view,
	onRunId,
	onDismiss,
}: Readonly<{
	spec: UpdateRunSpec;
	view: AppView;
	onRunId?(runId: string): void;
	onDismiss?(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const deviceName = useDeviceNames();
	const [handles, setHandles] = useState<Record<number, DeployRunHandle>>({});
	const onHandle = useCallback(
		(index: number, handle: DeployRunHandle) =>
			setHandles((current) =>
				current[index] === handle ? current : { ...current, [index]: handle },
			),
		[],
	);
	const labels = useMemo(() => phaseLabels(t), [t]);
	const total = spec.plans.reduce((sum, plan) => sum + plan.targets.length, 0);
	const title = useMemo(
		() => ({ code: "update", params: { what: view.app.name, count: total } }),
		[view.app.name, total],
	);
	const states = spec.plans.map((_, index) => handles[index]?.state);
	const mayStart = (index: number) =>
		index === 0 ||
		states
			.slice(0, index)
			.every(
				(state) =>
					state?.status === "finished" && !(spec.stopOnFail && failedIn(state)),
			);
	const firstId = states[0]?.id;
	useEffect(() => {
		if (firstId) onRunId?.(firstId);
	}, [firstId, onRunId]);

	const rows: FleetRolloutRow[] = spec.plans.flatMap((_, index) => {
		const handle = handles[index];
		if (!handle) return [];
		return handle.state.rows.map((row): FleetRolloutRow => {
			const state = rowState(row);
			return {
				id: row.target,
				device: deviceName(row.deviceId),
				...rowFacts(t, row, labels),
				state,
				...(spec.from ? { from: spec.from } : {}),
				version: spec.to,
				actions: <RunRowActions row={row} state={state} handle={handle} />,
			};
		});
	});
	const finished =
		states.length > 0 &&
		states.every((state) => state?.status === "finished") &&
		rows.length > 0;
	const done = rows.filter((row) => row.state === "done").length;
	const rolledBack = rows.filter((row) => row.state === "rolled_back");
	const shared = states.find((state) => state?.shared)?.shared;
	const held = states.find((state) => state?.status === "held");
	const anyRunning = states.some((state) => state?.status === "running");
	const finishedAt = Math.max(
		0,
		...states.map((state) => state?.finishedAt ?? 0),
	);
	return (
		<>
			{spec.plans.map((plan, index) => (
				<PlanRunner
					key={plan.draft.deploymentId}
					index={index}
					plan={plan}
					title={title}
					spec={spec}
					start={mayStart(index)}
					onHandle={onHandle}
				/>
			))}
			<FleetRollout
				kind="update"
				title={t("app.updateAll.runTitle", "Update {{app}} everywhere", {
					app: view.app.name,
				})}
				subtitle={[
					spec.from
						? t("app.updateAll.fromTo", "{{from}} → {{to}}", {
								from: spec.from,
								to: spec.to,
							})
						: spec.to,
					t("app.updateAll.safeShort", "safe update"),
					spec.oneAtATime
						? t("app.updateAll.oneShort", "one device at a time")
						: t("app.updateAll.allShort", "all devices at once"),
				].join(" · ")}
				rows={rows}
				stopOnFail={spec.stopOnFail}
				{...(held?.holdBy
					? { heldBy: rows.find((row) => row.id === held.holdBy)?.device }
					: {})}
				{...(shared
					? {
							shared: {
								label:
									shared.phase === "prepare"
										? t(
												"app.updateAll.sharedCopy",
												"Preparing the new copy on this computer",
											)
										: t(
												"app.updateAll.sharedDefinitions",
												"Approving the versions published now",
											),
								state: shared.state,
								source: shared.phase === "prepare" ? "local" : "hub",
							},
						}
					: {})}
				stamp={
					<FreshnessStamp
						source="live"
						age={anyRunning ? "live" : "current"}
						text={
							anyRunning
								? t("app.updateAll.following", "following")
								: t("app.updateAll.tracked", "tracked on this computer")
						}
					/>
				}
				result={
					finished ? (
						<InlineResult
							tone={
								done === rows.length ? "good" : done ? "warning" : "critical"
							}
							{...(onDismiss ? { onDismiss } : {})}
						>
							{rolledBack.length
								? t("app.updateAll.resultRolledBack", {
										done,
										count: rows.length,
										service: rolledBack[0].service ?? "",
										device: rolledBack[0].device,
										defaultValue_one:
											"Updated {{done, number}} of {{count, number}} service. {{service}} rolled back on {{device}}.",
										defaultValue_other:
											"Updated {{done, number}} of {{count, number}} services. {{service}} rolled back on {{device}}.",
									})
								: done === rows.length
									? t("app.updateAll.resultAll", {
											count: done,
											time: time.clock(finishedAt || time.nowS),
											defaultValue_one:
												"Updated {{count, number}} service at {{time}}.",
											defaultValue_other:
												"Updated {{count, number}} services at {{time}}.",
										})
									: t("app.updateAll.resultPartial", {
											done,
											count: rows.length,
											defaultValue_one:
												"Updated {{done, number}} of {{count, number}} service.",
											defaultValue_other:
												"Updated {{done, number}} of {{count, number}} services.",
										})}
						</InlineResult>
					) : undefined
				}
				className={cx("border-0 p-0")}
			/>
		</>
	);
}
