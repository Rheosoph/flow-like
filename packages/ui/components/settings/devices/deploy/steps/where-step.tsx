"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import {
	Ban,
	Box,
	CircleSlash,
	Clock,
	Copy,
	Lock,
	LockOpen,
	OctagonX,
	Server,
	Sparkles,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import {
	type ArtifactUsage,
	agentSupports,
	readArtifactUsage,
} from "../../../../../lib/device-management/agent-reads";
import {
	type DeployTargetDraft,
	PLAN_ACKNOWLEDGEMENTS,
	type PlanAcknowledgement,
	type PlanException,
	type PlanExceptionCode,
	type PlanTarget,
	type PlanTargetService,
	type TargetChoice,
	claimedEventIds,
} from "../../../../../lib/device-management/model/deploy-plan";
import { humanFileSize } from "../../../../../lib/utils";
import { useBackend } from "../../../../../state/backend-state";
import { agentTooOldCopy } from "../../copy/eligibility-copy";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { DvButton } from "../../primitives/dv-button";
import { CheckField } from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { useDevicesRoute } from "../../routing/use-devices-route";
import { useDeviceWorkspace } from "../../workspace/device-workspace-provider";
import { useOverlay } from "../../workspace/overlay-store";
import { deviceCall } from "../../workspace/use-live";
import {
	eventName,
	issueText,
	ownServiceFixLabel,
	planNames,
} from "../deploy-copy";
import type { DeployDevice } from "../deploy-facts";
import {
	DeploySelect,
	HeadChip,
	HubStamp,
	Note,
	type SelectOption,
	TargetsStamp,
} from "../deploy-parts";
import {
	type ExceptionRow,
	ExceptionsTable,
	exceptionRows,
} from "../exceptions-table";
import type { PlanStepProps } from "../step-props";
import { TargetCard } from "../target-card";
import { localTriggerKey } from "../use-deploy-reads";

/* Step 3 · Where (APP §3.7): devices as checkbox cards, each with its plan line; gated ones say why. */

const CARD_CAP = 8;

function storageText(t: DevicesT, usage: ArtifactUsage | undefined) {
	const budget = (usage?.device ?? usage?.project)?.bytes;
	if (!budget) return undefined;
	return budget.max === null
		? t("devices:deploy.card.storageUsed", "{{used}} used by app versions", {
				used: humanFileSize(budget.used),
			})
		: t(
				"devices:deploy.card.storage",
				"{{used}} of {{max}} used by app versions",
				{
					used: humanFileSize(budget.used),
					max: humanFileSize(budget.max),
				},
			);
}

/** BG17: room for app versions on each picked, connected device whose agent reports it; no line otherwise. */
function useStorageRoom(
	devices: readonly DeployDevice[],
	appId: string | null,
): Record<string, ArtifactUsage | undefined> {
	const workspace = useDeviceWorkspace();
	const able = devices.filter(
		(device) =>
			device.isLive && agentSupports(device.features, "artifact_capacity"),
	);
	const results = useQueries({
		queries: able.map((device) => ({
			queryKey: [
				"devices-deploy-storage",
				workspace.scopeKey,
				device.id,
				appId,
			],
			queryFn: async () => {
				const read = await readArtifactUsage(
					deviceCall(workspace, device.id, "poll"),
					device.features,
					{ projectId: appId },
				);
				return read.kind === "ok" ? read.data : null;
			},
			staleTime: Number.POSITIVE_INFINITY,
			retry: false,
		})),
	});
	return Object.fromEntries(
		able.map((device, index) => [device.id, results[index]?.data ?? undefined]),
	);
}

function PlanText({
	icon: Icon = Box,
	tone,
	children,
}: Readonly<{
	icon?: typeof Box;
	tone?: "warning" | "critical";
	children: ReactNode;
}>) {
	return (
		<p
			className={
				tone === "critical"
					? "flex items-start gap-1.5 text-xs text-critical"
					: tone === "warning"
						? "flex items-start gap-1.5 text-xs text-warning"
						: "flex items-start gap-1.5 text-ui"
			}
		>
			<Icon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
			<span className="min-w-0">{children}</span>
		</p>
	);
}

const Mono = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="font-mono">{children}</span>
);

interface PlanLineProps extends PlanStepProps {
	device: DeployDevice;
	target: PlanTarget;
	own: DeployTargetDraft;
}

function appServices(device: DeployDevice, appId: string | null) {
	return (device.services ?? []).filter(
		(service) => service.projectId === appId,
	);
}

/** Update entries: which service of the app this device updates. */
function UpdateLine({ device, target, own, state }: Readonly<PlanLineProps>) {
	const { t } = useTranslation("devices");
	const { draft, plan } = state;
	const services = appServices(device, draft.appId);
	const [service] = target.services;
	if (device.services === null)
		return (
			<PlanText icon={Lock}>
				{t(
					"deploy.where.updateLocked",
					"Update: its services load when you unlock.",
				)}
			</PlanText>
		);
	if (!services.length)
		return (
			<PlanText icon={Ban} tone="warning">
				{t("deploy.where.notHere", "{{app}} doesn't run here.", {
					app: plan.app?.name ?? "",
				})}
			</PlanText>
		);
	const choice = own.choices[service?.key ?? "main"];
	const picked = choice && choice.kind !== "new" ? choice.serviceId : "";
	return (
		<>
			{services.length > 1 && draft.keepEvents ? (
				<DeploySelect
					id={`deploy-plan-${device.id}`}
					mono
					label={t("deploy.where.planOn", "Plan on {{device}}", {
						device: device.name,
					})}
					value={picked}
					options={services.map((row) => ({
						value: row.serviceId,
						label: t("deploy.where.updateOption", "Update {{service}}", {
							service: row.serviceId,
						}),
					}))}
					onChange={(serviceId) =>
						state.updateTarget(device.id, (value) => ({
							...value,
							choices: {
								...value.choices,
								main: { kind: "update", serviceId },
							},
						}))
					}
				/>
			) : (
				<PlanText>
					{t("deploy.where.update", "Update")} <Mono>{picked}</Mono>
				</PlanText>
			)}
			<p className="text-xs text-muted-foreground">
				{draft.version === "keep"
					? t(
							"deploy.where.updateKeep",
							"Settings or events only; it keeps its version.",
						)
					: state.versionLabel
						? t(
								"deploy.where.updateNewestLabel",
								"To {{version}}, the newest version.",
								{ version: state.versionLabel },
							)
						: t("deploy.where.updateNewest", "To the newest version.")}
			</p>
		</>
	);
}

type ChoiceValue = "new" | `add:${string}`;

function choiceValue(choice: TargetChoice | undefined): ChoiceValue {
	return choice?.kind === "add" ? `add:${choice.serviceId}` : "new";
}

function toChoice(value: ChoiceValue): TargetChoice {
	return value === "new"
		? { kind: "new" }
		: { kind: "add", serviceId: value.slice(4) };
}

/** One planned service on a device that already runs the app: a new service, or add its events to an existing one (APP §3.15). */
function AddChoice({ device, target, own, state }: Readonly<PlanLineProps>) {
	const { t } = useTranslation("devices");
	const { plan, draft } = state;
	const [planned] = plan.services;
	const [service] = target.services;
	const existing = appServices(device, draft.appId).filter(
		(row) => row.events !== null,
	);
	const what =
		planned.events.length === 1
			? eventName(plan, planned.events[0])
			: t("deploy.where.events", {
					count: planned.events.length,
					defaultValue_one: "{{count, number}} event",
					defaultValue_other: "{{count, number}} events",
				});
	const options: SelectOption<ChoiceValue>[] = [
		{
			value: "new",
			label: t("deploy.where.newOption", "New service {{service}}", {
				service: service.kind === "new" ? service.serviceId : planned.id,
			}),
		},
		...existing.map((row) => ({
			value: `add:${row.serviceId}` as const,
			label: t("deploy.where.addOption", "Add {{what}} to {{service}}", {
				what,
				service: row.serviceId,
			}),
		})),
	];
	const added = existing.find(
		(row) => service.kind === "add" && row.serviceId === service.serviceId,
	);
	return (
		<>
			<DeploySelect
				id={`deploy-plan-${device.id}`}
				mono
				label={t("deploy.where.planOn", "Plan on {{device}}", {
					device: device.name,
				})}
				value={choiceValue(own.choices[planned.key])}
				options={options}
				onChange={(value) =>
					state.updateTarget(device.id, (current) => ({
						...current,
						choices: { ...current.choices, [planned.key]: toChoice(value) },
					}))
				}
			/>
			{added ? (
				<p className="text-xs text-muted-foreground">
					{added.desired === "stopped"
						? t(
								"deploy.where.addStopped",
								"{{service}} is stopped as you asked; it stays stopped, so {{what}} won't answer until you start it.",
								{ service: added.serviceId, what },
							)
						: t(
								"deploy.where.addRunning",
								"{{service}} also serves {{what}}; its other events, data and cloud access stay.",
								{ service: added.serviceId, what },
							)}
				</p>
			) : (
				<p className="text-xs text-muted-foreground">
					{existing.length === 1
						? t(
								"deploy.where.addHintOne",
								"Or add {{what}} to {{service}} as an update of that service.",
								{ what, service: existing[0].serviceId },
							)
						: t(
								"deploy.where.addHint",
								"Or add {{what}} to a service that already runs here, as an update of that service.",
								{ what },
							)}
				</p>
			)}
		</>
	);
}

function ServiceNotes({
	device,
	service,
	state,
	check,
}: Readonly<PlanLineProps & { service: PlanTargetService }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const time = useAreaTime();
	const { plan } = state;
	const claimed = claimedEventIds(plan.app);
	const issues = check.issues.filter(
		(issue) =>
			issue.step === "where" &&
			issue.deviceId === device.id &&
			issue.serviceKey === service.key &&
			issue.code !== "update_needs_service",
	);
	const both = (eventId: string, on: boolean) =>
		state.updateTarget(device.id, (value) => ({
			...value,
			serveBoth: on
				? [...value.serveBoth, eventId]
				: value.serveBoth.filter((id) => id !== eventId),
		}));
	return (
		<>
			{service.renamedFrom ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"deploy.where.renamed",
						"{{from}} is taken here; this one becomes {{to}}.",
						{ from: service.renamedFrom, to: service.serviceId },
					)}
				</p>
			) : null}
			{issues.map((issue) => (
				<PlanText
					key={`${issue.code}:${String(issue.params?.event ?? "")}`}
					icon={issue.severity === "warning" ? TriangleAlert : OctagonX}
					tone={issue.severity === "warning" ? "warning" : "critical"}
				>
					{issueText(t, issue, planNames(t, plan, state.facts, time.at))}
					{/* A bot can't be connected from two programs: the deploy waits until this computer stops it. */}
					{issue.code === "bot_local_trigger" ? (
						<span className="mt-1.5 block">
							<StopLocalTrigger
								appId={plan.app?.id}
								eventId={String(issue.params?.event ?? "")}
							/>
						</span>
					) : null}
				</PlanText>
			))}
			{service.leftOut.map((row) => (
				<PlanText key={row.eventId} icon={CircleSlash} tone="warning">
					{row.why === "refuse"
						? t(
								"deploy.where.leftOutRefuse",
								"Leaves out {{event}}: {{reason}}",
								{
									event: eventName(plan, row.eventId),
									reason: row.detail,
								},
							)
						: row.why === "agent"
							? t(
									"deploy.where.leftOutAgent",
									"Leaves out {{event}}: {{reason}}",
									{
										event: eventName(plan, row.eventId),
										reason: agentTooOldCopy(t, device.name, row.feature).long,
									},
								)
							: t(
									"deploy.where.leftOutDuplicate",
									"Leaves out {{event}}: {{service}} already serves it here.",
									{
										event: eventName(plan, row.eventId),
										service: row.serviceId,
									},
								)}{" "}
					{row.why === "agent" ? (
						<DvButton
							variant="link"
							size="xs"
							onClick={() =>
								navigate({
									screen: "device",
									deviceId: device.id,
									tab: "settings",
								})
							}
						>
							{agentTooOldCopy(t, device.name, row.feature).fix}
						</DvButton>
					) : null}
					{/* A schedule or bot runs in one place: it is never served by two services of one device. */}
					{row.why === "duplicate" && !claimed.has(row.eventId) ? (
						<DvButton
							variant="link"
							size="xs"
							onClick={() => both(row.eventId, true)}
						>
							{t("deploy.where.serveBoth", "Serve it in both")}
						</DvButton>
					) : null}
				</PlanText>
			))}
		</>
	);
}

function PlanLine(props: Readonly<PlanLineProps>) {
	const { t } = useTranslation("devices");
	const { device, target, own, state } = props;
	const { plan, draft } = state;
	if (draft.entry === "update") {
		const [updated] = target.services;
		return (
			<>
				<UpdateLine {...props} />
				{updated ? <ServiceNotes {...props} service={updated} /> : null}
			</>
		);
	}
	const addable =
		plan.services.length === 1 &&
		appServices(device, draft.appId).some((row) => row.events !== null);
	return (
		<>
			{addable ? <AddChoice {...props} /> : null}
			{target.services.map((service) => (
				<div key={service.key} className="flex min-w-0 flex-col gap-1">
					{addable ? null : (
						<PlanText>
							{t("deploy.where.newService", "New service")}{" "}
							<Mono>{service.serviceId}</Mono>
							{plan.services.length > 1
								? ` · ${service.events
										.map((eventId) => eventName(plan, eventId))
										.join(", ")}`
								: null}
						</PlanText>
					)}
					<ServiceNotes {...props} service={service} />
				</div>
			))}
			{own.serveBoth.length ? (
				<PlanText icon={Copy}>
					{t(
						"deploy.where.servesBoth",
						"Serves {{events}} in both services here.",
						{
							events: own.serveBoth
								.map((eventId) => eventName(plan, eventId))
								.join(", "),
						},
					)}{" "}
					<DvButton
						variant="link"
						size="xs"
						onClick={() =>
							state.updateTarget(device.id, (value) => ({
								...value,
								serveBoth: [],
							}))
						}
					>
						{t("deploy.where.leaveOut", "Leave it out again")}
					</DvButton>
				</PlanText>
			) : null}
		</>
	);
}

/** Selected first, then the ones that can take a deploy, by name. */
function sortDevices(
	devices: readonly DeployDevice[],
	selected: ReadonlySet<string>,
): DeployDevice[] {
	const rank = (device: DeployDevice) =>
		selected.has(device.id) ? 0 : device.gate ? 2 : 1;
	return [...devices].sort(
		(a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name),
	);
}

function useListedDevices({ state }: PlanStepProps) {
	const { devices, draft, appRead } = state;
	return useMemo(() => {
		const selected = new Set(draft.targets.map((target) => target.deviceId));
		const hubRuns = new Set(
			(appRead.placements.data?.placements ?? []).map((row) => row.device_id),
		);
		const listed =
			draft.entry === "update"
				? devices.filter(
						(device) =>
							selected.has(device.id) ||
							hubRuns.has(device.id) ||
							appServices(device, draft.appId).length > 0,
					)
				: devices;
		return { selected, listed: sortDevices(listed, selected) };
	}, [
		devices,
		draft.targets,
		draft.entry,
		draft.appId,
		appRead.placements.data,
	]);
}

function WhereFoot({
	listed,
	selected,
	fixed,
	state,
}: Readonly<{
	listed: readonly DeployDevice[];
	selected: ReadonlySet<string>;
	fixed: boolean;
	state: PlanStepProps["state"];
}>) {
	const { t } = useTranslation("devices");
	const { openUnlock, openUnlockSeveral } = useOverlay();
	const locked = listed.filter(
		(device) => selected.has(device.id) && device.locked && !device.gate,
	);
	const gated = listed.filter(
		(device) => device.gate && device.gate.code !== "staged",
	).length;
	const ready = listed.filter(
		(device) => !device.gate && !selected.has(device.id),
	);
	const unlock = () => {
		const [only] = locked;
		if (locked.length === 1 && only) openUnlock(only.id, { connectLive: true });
		else openUnlockSeveral();
	};
	return (
		<>
			<div className="flex flex-wrap items-center gap-2">
				<output className="min-w-0 flex-1 text-xs text-muted-foreground">
					{[
						t(
							"deploy.where.selected",
							"{{count, number}} of {{total, number}} devices selected",
							{ count: selected.size, total: listed.length },
						),
						locked.length
							? t("deploy.where.lockedCount", "{{count, number}} locked", {
									count: locked.length,
								})
							: "",
						gated
							? t(
									"deploy.where.gatedCount",
									"{{count, number}} can't take a deploy right now",
									{ count: gated },
								)
							: "",
					]
						.filter(Boolean)
						.join(" · ")}
				</output>
				{locked.length && !fixed ? (
					<DvButton size="sm" icon={LockOpen} onClick={unlock}>
						{t(
							"deploy.where.unlockSelected",
							"Unlock {{count, number}} selected…",
							{
								count: locked.length,
							},
						)}
					</DvButton>
				) : null}
				{ready.length && !fixed ? (
					<DvButton
						size="sm"
						variant="ghost"
						onClick={() => {
							for (const device of ready) state.toggleDevice(device.id, true);
						}}
					>
						{t("deploy.where.selectReady", "Select all ready")}
					</DvButton>
				) : null}
			</div>
			{locked.length ? (
				<Note icon={Lock}>
					{t(
						"deploy.where.lockedNote",
						"Locked devices can be picked. Their platform, agent and isolation load when you unlock, and Continue waits until every picked device is unlocked.",
					)}
				</Note>
			) : null}
		</>
	);
}

function DevicesBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, route, check } = props;
	const { plan, draft, revoked } = state;
	const [all, setAll] = useState(false);
	const { selected, listed } = useListedDevices(props);
	const picked = listed.filter((device) => selected.has(device.id));
	const storage = useStorageRoom(picked, draft.appId);
	const fixed = draft.entry === "update" && !!route.serviceId;
	const shown = all ? listed : listed.slice(0, CARD_CAP);
	const appName = plan.app?.name ?? "";
	const none = check.issues.find(
		(issue) => issue.step === "where" && issue.code === "no_targets",
	);
	return (
		<Block
			title={t("deploy.where.devices", "Devices")}
			icon={Server}
			summary={
				<HeadChip>
					{t("deploy.where.selectedCount", "{{count, number}} selected", {
						count: selected.size,
					})}
				</HeadChip>
			}
			stamp={<HubStamp />}
			foot={
				<>
					{draft.entry === "update"
						? t(
								"deploy.where.footUpdate",
								"Only devices that run {{app}} are listed.",
								{ app: appName },
							)
						: t(
								"deploy.where.footNew",
								"Only devices you can deploy to are selectable; the reason is under the others.",
							)}
					{revoked
						? ` ${t("deploy.where.revoked", {
								count: revoked,
								defaultValue_one:
									"{{count, number}} revoked device isn't listed.",
								defaultValue_other:
									"{{count, number}} revoked devices aren't listed.",
							})}`
						: ""}
				</>
			}
		>
			{listed.length ? (
				<fieldset className="m-0 grid min-w-0 grid-cols-2 items-start gap-3 border-0 p-0 @max-[560px]/wherestep:grid-cols-1">
					<legend className="sr-only">
						{t("deploy.where.legend", "Deploy to")}
					</legend>
					{shown.map((device) => {
						const target = plan.targets.find(
							(row) => row.deviceId === device.id,
						);
						const own = draft.targets.find((row) => row.deviceId === device.id);
						return (
							<TargetCard
								key={device.id}
								device={device}
								checked={selected.has(device.id)}
								fixed={fixed}
								appName={appName}
								storage={storageText(t, storage[device.id])}
								plan={
									target && own ? (
										<PlanLine
											{...props}
											device={device}
											target={target}
											own={own}
										/>
									) : undefined
								}
								onCheckedChange={(on) => state.toggleDevice(device.id, on)}
							/>
						);
					})}
				</fieldset>
			) : (
				<StateView
					kind="empty"
					title={
						draft.entry === "update"
							? t(
									"deploy.where.noneRun",
									"{{app}} isn't on a device you can see",
									{
										app: appName,
									},
								)
							: t("deploy.where.noDevices", "You don't have any devices yet")
					}
					text={t(
						"deploy.where.noDevicesText",
						"Set up a device first; it shows here once it has checked in.",
					)}
				/>
			)}
			{listed.length > CARD_CAP ? (
				<DvButton
					size="sm"
					variant="ghost"
					className="w-fit"
					onClick={() => setAll(!all)}
				>
					{all
						? t("deploy.where.showFewer", "Show fewer")
						: t("deploy.where.showMore", {
								count: listed.length - CARD_CAP,
								defaultValue_one: "Show {{count, number}} more device",
								defaultValue_other: "Show {{count, number}} more devices",
							})}
				</DvButton>
			) : null}
			{none && state.reached > 2 ? (
				<p className="text-xs text-critical">
					{issueText(t, none, planNames(t, plan))}
				</p>
			) : null}
			<WhereFoot
				listed={listed}
				selected={selected}
				fixed={fixed}
				state={state}
			/>
		</Block>
	);
}

const SCHEDULE_WARNINGS: readonly PlanExceptionCode[] = [
	"schedule_two_devices",
	"schedule_local_trigger",
];

/** A schedule that would run in more than one place needs the person's yes, once per device. */
function ScheduleTwiceCheck({
	state,
	deviceId,
}: Readonly<{ state: PlanStepProps["state"]; deviceId: string }>) {
	const { t } = useTranslation("devices");
	const own = state.draft.targets.find((row) => row.deviceId === deviceId);
	return (
		<CheckField
			id={`deploy-schedule-twice-${deviceId}`}
			checked={own?.over.scheduleTwice === true}
			onCheckedChange={(scheduleTwice) =>
				state.updateTarget(deviceId, (value) => ({
					...value,
					over: { ...value.over, scheduleTwice },
				}))
			}
		>
			{t(
				"deploy.where.scheduleTwice",
				"Run it here as well: it then runs in more than one place",
			)}
		</CheckField>
	);
}

/**
 * The desktop app runs this schedule itself. One click removes that trigger
 * here, so the device is the only place that runs it and nothing is left to
 * acknowledge.
 */
function StopLocalTrigger({
	appId,
	eventId,
}: Readonly<{ appId: string | undefined; eventId: string }>) {
	const { t } = useTranslation("devices");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [stop, setStop] = useState<"idle" | "busy" | "failed">("idle");
	const sinks = backend.sinkState;
	if (!sinks) return null;
	const run = async () => {
		setStop("busy");
		try {
			await sinks.removeEventSink(eventId);
			await queryClient.invalidateQueries({
				queryKey: localTriggerKey(appId, eventId),
			});
			setStop("idle");
		} catch {
			setStop("failed");
		}
	};
	return (
		<span className="flex flex-col items-start gap-1">
			<DvButton
				size="xs"
				busy={stop === "busy"}
				data-act="stop-local-trigger"
				onClick={() => void run()}
			>
				{t(
					"deploy.exception.stopLocalTrigger",
					"Stop running it on this computer",
				)}
			</DvButton>
			{stop === "failed" ? (
				<span role="alert" className="text-xs text-critical">
					{t(
						"deploy.exception.stopLocalTriggerFailed",
						"It couldn't be stopped here. Turn it off in Events on this computer.",
					)}
				</span>
			) : null}
		</span>
	);
}

const isAcknowledgement = (
	code: PlanExceptionCode,
): code is PlanAcknowledgement =>
	(PLAN_ACKNOWLEDGEMENTS as readonly string[]).includes(code);

const ACKNOWLEDGE_LABEL: Record<PlanAcknowledgement, (t: DevicesT) => string> =
	{
		once_soon: (t) =>
			t(
				"devices:deploy.where.acknowledge.onceSoon",
				"Deploy it anyway: it may not run",
			),
		bot_open: (t) =>
			t(
				"devices:deploy.where.acknowledge.botOpen",
				"Let anyone who can message it start runs",
			),
		bot_other_computers: (t) =>
			t(
				"devices:deploy.where.acknowledge.botOtherComputers",
				"No other computer runs it",
			),
		endpoint_shared_token: (t) =>
			t(
				"devices:deploy.where.acknowledge.endpointSharedToken",
				"Let the service's token call it",
			),
	};

/** One yes per device and warning: it covers every row of that warning on the device. */
function AcknowledgeCheck({
	state,
	deviceId,
	code,
}: Readonly<{
	state: PlanStepProps["state"];
	deviceId: string;
	code: PlanAcknowledgement;
}>) {
	const { t } = useTranslation("devices");
	const own = state.draft.targets.find((row) => row.deviceId === deviceId);
	const given = own?.over.acknowledged ?? [];
	return (
		<CheckField
			id={`deploy-acknowledge-${code}-${deviceId}`}
			checked={given.includes(code)}
			onCheckedChange={(on) =>
				state.updateTarget(deviceId, (value) => {
					const current = value.over.acknowledged ?? [];
					return {
						...value,
						over: {
							...value.over,
							acknowledged: on
								? [...new Set([...current, code])]
								: current.filter((row) => row !== code),
						},
					};
				})
			}
		>
			{ACKNOWLEDGE_LABEL[code](t)}
		</CheckField>
	);
}

/** "Deploy it as its own service": the Endpoint keeps a token of its own, so nothing is left to confirm. */
function OwnServiceFix({
	state,
	eventId,
}: Readonly<{ state: PlanStepProps["state"]; eventId: string }>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			size="xs"
			data-act="own-service"
			onClick={() =>
				state.update({
					ownService: [
						...new Set([...(state.draft.ownService ?? []), eventId]),
					],
				})
			}
		>
			{ownServiceFixLabel(t)}
		</DvButton>
	);
}

/**
 * What an exception row offers besides its sentence: "Serve it in both" for an
 * event another service already serves (never for a schedule or a bot), the
 * acknowledgement on the first row of each device's warning, the way to stop
 * a schedule that this computer runs itself, and the own service of an
 * Endpoint that has its own token.
 */
function useExceptionExtras({ state }: PlanStepProps) {
	const { t } = useTranslation("devices");
	const claimed = claimedEventIds(state.plan.app);
	const asked = new Set<string>();
	const first = (key: string) => {
		if (asked.has(key)) return false;
		asked.add(key);
		return true;
	};
	return (
		exception: PlanException,
	): Partial<Pick<ExceptionRow, "check" | "action">> => {
		const eventId = String(exception.params?.event ?? "");
		const { code, deviceId } = exception;
		if (isAcknowledgement(code)) {
			const action =
				code === "endpoint_shared_token" ? (
					<OwnServiceFix state={state} eventId={eventId} />
				) : undefined;
			return {
				...(first(`${deviceId}\n${code}`)
					? {
							check: (
								<AcknowledgeCheck
									state={state}
									deviceId={deviceId}
									code={code}
								/>
							),
						}
					: {}),
				...(action ? { action } : {}),
			};
		}
		if (SCHEDULE_WARNINGS.includes(code)) {
			const action =
				code === "schedule_local_trigger" ? (
					<StopLocalTrigger appId={state.plan.app?.id} eventId={eventId} />
				) : undefined;
			if (!first(deviceId)) return action ? { action } : {};
			return {
				check: <ScheduleTwiceCheck state={state} deviceId={deviceId} />,
				...(action ? { action } : {}),
			};
		}
		if (code !== "left_out_duplicate" || claimed.has(eventId)) return {};
		return {
			action: (
				<DvButton
					size="xs"
					onClick={() =>
						state.updateTarget(exception.deviceId, (value) => ({
							...value,
							serveBoth: [...value.serveBoth, eventId],
						}))
					}
				>
					{t("deploy.where.serveBoth", "Serve it in both")}
				</DvButton>
			),
		};
	};
}

/**
 * An update has no "Events on these devices" block. A schedule it adds that
 * would run in a second place is still said, with its acknowledgement, and so
 * is every other warning that needs a yes (a one-time schedule due very soon,
 * a bot it adds, an Endpoint that loses its own token).
 */
function ScheduleWarnings(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { state, check } = props;
	const extras = useExceptionExtras(props);
	const exceptions = check.exceptions.filter(
		(exception) =>
			exception.step === "where" &&
			(SCHEDULE_WARNINGS.includes(exception.code) ||
				isAcknowledgement(exception.code)),
	);
	if (!exceptions.length) return null;
	const schedulesOnly = exceptions.every((exception) =>
		SCHEDULE_WARNINGS.includes(exception.code),
	);
	const rows = exceptionRows(
		t,
		state.plan,
		exceptions,
		extras,
		state.facts,
		time.at,
	);
	return (
		<Block
			title={
				schedulesOnly
					? t("deploy.where.schedulesTitle", "Schedules in this update")
					: t("deploy.where.confirmTitle", "To confirm in this update")
			}
			icon={schedulesOnly ? Clock : TriangleAlert}
			stamp={
				<TargetsStamp targets={state.plan.targets} devices={state.devices} />
			}
		>
			{schedulesOnly ? (
				<ExceptionsTable
					label={t(
						"deploy.where.schedulesTable",
						"Schedules that run in more than one place",
					)}
					shared={t(
						"deploy.where.schedulesShared",
						"A schedule runs once in every place it is deployed to.",
					)}
					rows={rows}
				/>
			) : (
				<ExceptionsTable
					label={t(
						"deploy.where.confirmTable",
						"Warnings this update asks you to confirm",
					)}
					shared={t(
						"deploy.where.confirmShared",
						"Each warning needs your yes on its device.",
					)}
					rows={rows}
				/>
			)}
		</Block>
	);
}

/** "Events on these devices": only pairs that differ are rows (APP §3.7 item 4). */
function EventsOnDevices(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { state, check } = props;
	const { plan, mode, devices } = state;
	const count = plan.targets.length;
	const extras = useExceptionExtras(props);
	const exceptions = check.exceptions.filter(
		(exception) => exception.step === "where" && exception.code !== "renamed",
	);
	const rows = exceptionRows(t, plan, exceptions, extras, state.facts, time.at);
	const events = new Set(plan.services.flatMap((service) => service.events));
	const shared = !count
		? t(
				"deploy.where.eventsNone",
				"Pick devices to see which events can run where.",
			)
		: rows.length
			? t("deploy.where.eventsRest", {
					count: events.size,
					defaultValue_one:
						"{{count, number}} event picked. Where it's left out is listed below.",
					defaultValue_other:
						"{{count, number}} events picked. The rest of them can run on every picked device.",
				})
			: t("deploy.where.eventsSame", {
					count: events.size,
					defaultValue_one: "Same on every device: the event can run.",
					defaultValue_other:
						"Same on every device: all {{count, number}} events can run.",
				});
	return (
		<Block
			title={
				count > 1
					? t("deploy.where.eventsOnThese", "Events on these devices")
					: t("deploy.where.eventsOnThis", "Events on this device")
			}
			icon={Sparkles}
			stamp={
				mode === "offline" && count ? (
					<FreshnessStamp
						source="local"
						age="current"
						text={t("deploy.where.provisional", "provisional")}
					/>
				) : (
					<TargetsStamp targets={plan.targets} devices={devices} />
				)
			}
		>
			<ExceptionsTable
				label={t("deploy.where.eventsTable", "Events that differ by device")}
				shared={shared}
				rows={rows}
			/>
			{mode === "offline" ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"deploy.where.offlineRecheck",
						"The device checks the events again after the copy arrives (step 6).",
					)}
				</p>
			) : null}
		</Block>
	);
}

export function WhereStep(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state } = props;
	const { app, mode, draft } = state;
	return (
		<div className="@container/wherestep flex min-w-0 flex-col gap-4">
			<WizardStepHeader
				step={3}
				total={8}
				title={t("deploy.title.where", "Where it runs")}
				lede={
					mode === "offline"
						? t(
								"deploy.where.ledeOffline",
								"Pick one or more devices. Each gets its own service and settings.",
							)
						: t(
								"deploy.where.lede",
								"Pick one or more devices. Each gets its own service, settings and cloud access.",
							)
				}
			/>
			{app ? (
				<>
					<DevicesBlock {...props} />
					{draft.entry === "update" ? (
						<ScheduleWarnings {...props} />
					) : (
						<EventsOnDevices {...props} />
					)}
				</>
			) : (
				<StateView
					kind="notloaded"
					title={t("deploy.needApp", "Choose an app first")}
					text={t(
						"deploy.needAppText",
						"This step depends on the app you deploy. Go back to What.",
					)}
					actions={
						<DvButton size="sm" onClick={() => props.goTo("what")}>
							{t("deploy.goToWhat", "Go to What")}
						</DvButton>
					}
				/>
			)}
		</div>
	);
}
