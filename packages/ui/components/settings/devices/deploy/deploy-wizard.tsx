"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Ban,
	ChevronLeft,
	ChevronRight,
	History,
	Hourglass,
	LoaderCircle,
	Server,
	Trash2,
} from "lucide-react";
import {
	Fragment,
	type ReactNode,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
} from "react";
import type {
	DeployResult,
	PlanIssue,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DeployStepId,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../../../ui/breadcrumb";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { useConfirm } from "../primitives/confirm-sheet";
import { DvButton } from "../primitives/dv-button";
import { GateNotice } from "../primitives/gate-notice";
import { Headline } from "../primitives/headline";
import { StateView } from "../primitives/state-view";
import {
	WizardFoot,
	WizardStepper,
	WizardTitleRow,
} from "../primitives/wizard";
import { deployExitHref } from "../routing/devices-href";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import {
	REVIEW_INDEX,
	currentStep,
	deploySteps,
	eventName,
	exitRoute,
	issueText,
	noFlowsText,
	planNames,
	prefilledSteps,
	prepareFailureText,
	stepLabel,
	stepTitle,
	whereGateKind,
	whereGateText,
} from "./deploy-copy";
import { type DeployDevice, limitsOutOfRange } from "./deploy-facts";
import {
	DeploySummary,
	DeploySummaryBar,
	type DeploySummaryInput,
} from "./deploy-summary";
import type { DeployStepProps, PlanStepProps } from "./step-props";
import { AccessCostStep } from "./steps/access-cost-step";
import { CopyUploadStep } from "./steps/copy-upload-step";
import { EndpointLimitsStep } from "./steps/endpoint-limits-step";
import { HowItRunsStep } from "./steps/how-it-runs-step";
import { ReviewStep } from "./steps/review-step";
import { RolloutStep } from "./steps/rollout-step";
import { SettingsStep } from "./steps/settings-step";
import { WhatStep } from "./steps/what-step";
import { WhereStep } from "./steps/where-step";
import { GateFixes } from "./target-card";
import { type DeployDraftState, useDeployDraft } from "./use-deploy-draft";
import {
	type DeployPrepareState,
	useDeployPrepare,
} from "./use-deploy-prepare";
import { useDeployRunState } from "./use-deploy-run";

/* The deploy wizard's frame (APP §3.2, §3.4): header, planning headline, stepper, summary and foot around one of eight steps. */

/** Steps that hand the prepared bundle on; What has nothing to prepare yet and Rollout prepares by itself when it has to. */
const PREPARING: readonly DeployStepId[] = [
	"how",
	"where",
	"settings",
	"endpoint",
	"access_cost",
	"copy_upload",
	"review",
];

const Mono = ({ children }: Readonly<{ children: ReactNode }>) => (
	<span className="font-mono">{children}</span>
);

/** The device a device-first entry names, while no app is chosen yet. */
function entryDevice(state: DeployDraftState): DeployDevice | undefined {
	const [first] = state.plan.targets;
	return state.devices.find((row) => row.id === first?.deviceId);
}

/** What is deployed, in words: the one event of a single-event entry, else the app. */
function deployedThing(state: DeployDraftState, app: string): string {
	const { plan, draft } = state;
	return draft.scope === "event" && draft.events.length === 1
		? eventName(plan, draft.events[0])
		: app;
}

function wizardTitle(t: DevicesT, state: DeployDraftState): ReactNode {
	const { plan, draft } = state;
	if (!plan.app) {
		const device = entryDevice(state);
		return device ? (
			<>
				{t("deploy.frame.titleTo", "Deploy to")} <Mono>{device.name}</Mono>
			</>
		) : (
			t("deploy.frame.title", "Deploy")
		);
	}
	if (draft.entry !== "update")
		return t("deploy.frame.titleDeploy", "Deploy {{what}}", {
			what: deployedThing(state, plan.app.name),
		});
	const one = singleUpdate(state);
	return one ? (
		<>
			{t("deploy.frame.titleUpdate", "Update")} <Mono>{one.service}</Mono>
		</>
	) : (
		t("deploy.frame.titleUpdateApp", "Update {{app}}", { app: plan.app.name })
	);
}

function subTarget(t: DevicesT, state: DeployDraftState): string {
	const [first] = state.plan.targets;
	const count = state.plan.targets.length;
	if (!first) return t("deploy.frame.subNoDevices", "no devices yet");
	return count === 1
		? t("deploy.frame.subTo", "to {{device}}", { device: first.name })
		: t("deploy.frame.subDevices", "{{count, number}} devices", { count });
}

function subKind(t: DevicesT, state: DeployDraftState): string {
	const { plan, draft } = state;
	const count = plan.targets.length;
	if (draft.entry === "update")
		return count > 1
			? t(
					"deploy.frame.subUpdateMany",
					"update of {{count, number}} services",
					{
						count,
					},
				)
			: t("deploy.frame.subUpdate", "update");
	return plan.services.length > 1
		? t("deploy.frame.subNewServices", "{{count, number}} new services", {
				count: plan.services.length,
			})
		: t("deploy.frame.subNewService", "new service");
}

/** The line under the title: app · mode · version · targets · new service | update. */
function wizardSub(t: DevicesT, state: DeployDraftState): string {
	const { plan, draft, mode, versionLabel } = state;
	if (!plan.app || !mode) {
		const device = entryDevice(state);
		return device
			? t("deploy.frame.subPick", "Pick an app for {{device}}", {
					device: device.name,
				})
			: t(
					"deploy.frame.subNone",
					"Install an app version on devices you manage",
				);
	}
	const keeps = draft.entry === "update" && draft.version === "keep";
	return [
		plan.app.name,
		mode === "offline"
			? t("deploy.frame.subOffline", "offline copy")
			: t("deploy.frame.subOnline", "runs online"),
		keeps
			? t("deploy.frame.subKeeps", "keeps each service's version")
			: versionLabel,
		subTarget(t, state),
		subKind(t, state),
	]
		.filter(Boolean)
		.join(" · ");
}

function modeSentence(t: DevicesT, state: DeployDraftState): string {
	const { draft, mode } = state;
	const update = draft.entry === "update";
	if (update && draft.version === "keep")
		return t(
			"deploy.frame.modeKeep",
			"Only settings or events change; nothing is uploaded.",
		);
	if (mode === "offline")
		return update
			? t(
					"deploy.frame.modeOfflineUpdate",
					"It's a local-only app, so the update sends a new copy of the app from this computer. The data on the device stays as it is.",
				)
			: t(
					"deploy.frame.modeOffline",
					"It's a local-only app, so each device gets an offline copy of the app and its data from this computer.",
				);
	return update
		? t(
				"deploy.frame.modeOnlineUpdate",
				"It's an online app, so the update re-pins each service to what's published now. Its data stays in the cloud.",
			)
		: t(
				"deploy.frame.modeOnline",
				"It's an online app, so each device runs the version you deploy and keeps its data in the cloud.",
			);
}

/** Before an app is chosen (device-first). */
function pickLead(t: DevicesT, state: DeployDraftState): string {
	const device = entryDevice(state);
	return device
		? t("deploy.frame.leadPick", "Choose an app for {{device}}.", {
				device: device.name,
			})
		: t("deploy.frame.leadPickAny", "Choose an app to deploy.");
}

/** One service on one device, when the update names exactly that. */
function singleUpdate(state: DeployDraftState) {
	const [first] = state.plan.targets;
	const [service] = first?.services ?? [];
	const one = state.plan.targets.length === 1 && service?.kind !== "new";
	return one && first && service
		? { service: service.serviceId, device: first.name }
		: null;
}

function updateLead(t: DevicesT, state: DeployDraftState, app: string): string {
	const keep = state.draft.version === "keep";
	const one = singleUpdate(state);
	const count = state.plan.targets.length;
	if (one)
		return keep
			? t(
					"deploy.frame.leadChangeOne",
					"Change {{service}}'s settings or events on {{device}}.",
					one,
				)
			: t(
					"deploy.frame.leadUpdateOne",
					"Update {{service}} on {{device}} to the newest version.",
					one,
				);
	return keep
		? t("deploy.frame.leadChangeMany", {
				app,
				count,
				defaultValue_one:
					"Change settings or events of {{app}} on {{count, number}} device.",
				defaultValue_other:
					"Change settings or events of {{app}} on {{count, number}} devices.",
			})
		: t("deploy.frame.leadUpdateMany", {
				app,
				count,
				defaultValue_one:
					"Update {{app}} on {{count, number}} device to the newest version.",
				defaultValue_other:
					"Update {{app}} on {{count, number}} devices to the newest version.",
			});
}

function deployLead(t: DevicesT, state: DeployDraftState, app: string): string {
	const { plan } = state;
	const [first] = plan.targets;
	const count = plan.targets.length;
	const what = deployedThing(state, app);
	if (!first)
		return t(
			"deploy.frame.leadNone",
			"Deploy {{what}}: pick what runs, then where.",
			{ what },
		);
	if (count > 1)
		return t(
			"deploy.frame.leadMany",
			"Deploy {{what}} to {{count, number}} devices.",
			{ what, count },
		);
	const [service] = first.services;
	return t(
		"deploy.frame.leadOne",
		"Deploy {{what}} to {{device}} as {{service}}.",
		{
			what,
			device: first.name,
			service: service?.serviceId ?? plan.services[0]?.id ?? "",
		},
	);
}

function leadSentence(t: DevicesT, state: DeployDraftState): string {
	const { plan, draft } = state;
	if (!plan.app) return pickLead(t, state);
	return draft.entry === "update"
		? updateLead(t, state, plan.app.name)
		: deployLead(t, state, plan.app.name);
}

function restSentences(
	t: DevicesT,
	state: DeployDraftState,
	stepIndex: number,
): string {
	const { plan, draft, app } = state;
	if (!plan.app || !app)
		return t(
			"deploy.frame.restPick",
			"Its mode and its events decide what can run where. Nothing changes on any device until you deploy.",
		);
	const count = plan.targets.length;
	const parts: string[] = [];
	if (stepIndex <= 1 || !count) parts.push(modeSentence(t, state));
	if (draft.entry !== "update" && stepIndex === 0) {
		const all = plan.app.events.length;
		const able = new Set(plan.services.flatMap((row) => row.events)).size;
		if (draft.scope === "app")
			parts.push(
				t(
					"deploy.frame.restEvents",
					"{{count, number}} of its {{total, number}} events can run on a device.",
					{ count: able, total: all },
				),
			);
	}
	const gated = plan.targets.filter((target) => target.gate);
	if (gated.length && count > 1)
		parts.push(
			t("deploy.frame.restGated", "{{devices}} can't take it right now.", {
				devices: gated.map((target) => target.name).join(", "),
			}),
		);
	parts.push(
		count === 1
			? t(
					"deploy.frame.restOne",
					"Nothing changes on the device until you deploy on step 7.",
				)
			: t(
					"deploy.frame.restMany",
					"Nothing changes on any device until you deploy on step 7.",
				),
	);
	return parts.join(" ");
}

function PlanningHeadline({
	state,
	stepIndex,
}: Readonly<{ state: DeployDraftState; stepIndex: number }>) {
	const { t } = useTranslation("devices");
	const { plan, check } = state;
	const [first] = plan.targets;
	if (plan.app && check.issues.some((issue) => issue.code === "local_only_web"))
		return (
			<Headline
				lead={t(
					"deploy.frame.leadLocalWeb",
					"{{app}} can't be deployed from the browser.",
					{ app: plan.app.name },
				)}
				rest={t(
					"deploy.frame.restLocalWeb",
					"It's a local-only app: only the desktop app on the computer that has it can prepare its offline copy. Nothing changes on any device.",
				)}
			/>
		);
	if (plan.targets.length === 1 && first?.gate)
		return (
			<Headline
				lead={t(
					"deploy.frame.leadGated",
					"{{device}} can't take this deploy right now.",
					{ device: first.name },
				)}
				rest={t(
					"deploy.frame.restGatedOne",
					"Your choices are kept, and nothing changes on the device.",
				)}
			/>
		);
	return (
		<Headline
			lead={leadSentence(t, state)}
			rest={restSentences(t, state, stepIndex)}
		/>
	);
}

function Crumbs({
	route,
	scope,
	state,
}: Readonly<{
	route: DeployRoute;
	scope: DevicesScope;
	state: DeployDraftState;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const [deviceId] = route.deviceIds;
	const device = state.devices.find((row) => row.id === deviceId);
	// In an app the top bar already reads "Devices › Deploy"; only the way back to Events is said here.
	if (scope.kind === "app" && route.from !== "events") return null;
	const crumbs: { label: ReactNode; props: { href: string } }[] =
		scope.kind === "app"
			? [
					{
						label: t("deploy.frame.crumbEvents", "Events"),
						props: { href: deployExitHref(route, scope) },
					},
				]
			: [
					{
						label: t("deploy.frame.crumbDevices", "Devices"),
						props: link({ screen: "fleet", view: "devices" }),
					},
					...(device && route.deviceIds.length === 1
						? [
								{
									label: <Mono>{device.name}</Mono>,
									props: link({
										screen: "device",
										deviceId: device.id,
										tab: "services",
									}),
								},
							]
						: []),
					...(device && route.serviceId
						? [
								{
									label: <Mono>{route.serviceId}</Mono>,
									props: link({
										screen: "service",
										deviceId: device.id,
										serviceId: route.serviceId,
									}),
								},
							]
						: []),
				];
	return (
		<Breadcrumb aria-label={t("deploy.frame.breadcrumb", "Breadcrumb")}>
			<BreadcrumbList className="gap-1 text-xs sm:gap-1">
				{crumbs.map((crumb, index) => (
					// biome-ignore lint/suspicious/noArrayIndexKey: crumbs are positional
					<Fragment key={index}>
						<BreadcrumbItem>
							<BreadcrumbLink {...crumb.props} className="hover:underline">
								{crumb.label}
							</BreadcrumbLink>
						</BreadcrumbItem>
						<BreadcrumbSeparator className="[&>svg]:size-3" />
					</Fragment>
				))}
				<BreadcrumbItem>
					<BreadcrumbPage className="text-ink-2">
						{t("deploy.frame.crumbDeploy", "Deploy")}
					</BreadcrumbPage>
				</BreadcrumbItem>
			</BreadcrumbList>
		</Breadcrumb>
	);
}

/** One notice per gated target on every step but Where, where the cards say it themselves. */
function TargetGates({
	state,
	step,
	goTo,
}: Readonly<{
	state: DeployDraftState;
	step: DeployStepId;
	goTo(step: DeployStepId): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { plan, devices } = state;
	const gated = plan.targets.flatMap((target) => {
		const device = devices.find((row) => row.id === target.deviceId);
		return device?.gate ? [{ device, gate: device.gate }] : [];
	});
	const staged = gated.filter(({ gate }) => gate.code === "staged");
	const others = gated.filter(({ gate }) => gate.code !== "staged");
	const appName = plan.app?.name ?? "";
	return (
		<>
			{step === "what" || step === "where"
				? staged.map(({ device, gate }) => (
						<StagedBanner
							key={device.id}
							device={device}
							service={gate.serviceId ?? ""}
							by={gate.by}
							appName={appName}
						/>
					))
				: null}
			{step === "where"
				? null
				: others.map(({ device, gate }) => (
						<GateNotice
							key={device.id}
							kind={whereGateKind(gate)}
							title={t(
								"deploy.frame.gateTitle",
								"You can't deploy to {{device}} right now.",
								{ device: device.name },
							)}
							text={
								<>
									{whereGateText(t, gate, device.name, time)}{" "}
									{plan.targets.length > 1
										? t(
												"deploy.frame.gateOthers",
												"The other devices aren't affected.",
											)
										: t(
												"deploy.frame.gateLook",
												"You can look through the steps, but Deploy stays unavailable until this is fixed.",
											)}
								</>
							}
							actions={
								<>
									<GateFixes
										device={device}
										gate={gate}
										appName={appName}
										size="sm"
									/>
									<DvButton
										size="sm"
										variant="ghost"
										icon={Server}
										onClick={() => goTo("where")}
									>
										{t("deploy.frame.changeDevices", "Change devices")}
									</DvButton>
								</>
							}
						/>
					))}
		</>
	);
}

function StagedBanner({
	device,
	service,
	by,
	appName,
}: Readonly<{
	device: DeployDevice;
	service: string;
	by?: number;
	appName: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { gate } = device;
	return (
		<Banner
			tone="warning"
			icon={Hourglass}
			title={t(
				"deploy.frame.stagedTitle",
				"An update is already staged for {{service}} on {{device}}.",
				{ service, device: device.name },
			)}
			actions={
				gate ? (
					<GateFixes device={device} gate={gate} appName={appName} size="sm" />
				) : undefined
			}
		>
			{t(
				"deploy.frame.stagedText",
				"The device holds one update per service at a time, so activate or discard it before you deploy.",
			)}{" "}
			{by === undefined
				? null
				: t(
						"deploy.frame.stagedBy",
						"If nobody activates it, the device discards it at {{time}}.",
						{ time: time.abs(by) },
					)}
		</Banner>
	);
}

interface Blocking {
	text: string;
	busy?: boolean;
}

interface BlockingContext {
	t: DevicesT;
	time: ReturnType<typeof useAreaTime>;
	state: DeployDraftState;
	prepare: DeployPrepareState;
}

/** A gated target: the device's own reason, as its card says it. */
function gatedBlocking(
	{ t, time, state }: BlockingContext,
	issue: PlanIssue,
): Blocking | null {
	const device = state.devices.find((row) => row.id === issue.deviceId);
	return issue.code === "target_gated" && device?.gate
		? {
				text: t("deploy.frame.gatedReason", "{{device}}: {{reason}}", {
					device: device.name,
					reason: whereGateText(t, device.gate, device.name, time),
				}),
			}
		: null;
}

/** What: a role that can't read the app's flows can't deploy it. */
function whatBlocking({ t, state }: BlockingContext): Blocking | null {
	return state.app && !state.canReadFlows
		? { text: noFlowsText(t, state.app.name) }
		: null;
}

/** How it runs: the preparation runs, or stopped at a check. */
function howBlocking({ t, state, prepare }: BlockingContext): Blocking | null {
	if (prepare.state === "running")
		return {
			text: t("deploy.frame.preparing", "Preparing on this computer…"),
			busy: true,
		};
	return prepare.failure
		? { text: prepareFailureText(t, prepare.failure, state.plan) }
		: null;
}

/** Settings: stored values of an update that need a decision. */
function settingsBlocking({ t, state }: BlockingContext): Blocking | null {
	const count = state.unresolved.length;
	if (!count) return null;
	return {
		text: t("deploy.frame.unresolved", {
			count,
			defaultValue_one:
				"A stored value no longer fits this update. Replace or remove it.",
			defaultValue_other:
				"{{count, number}} stored values no longer fit this update. Replace or remove them.",
		}),
	};
}

/** Where: every picked device has to be connected live before Continue. */
function whereBlocking({ t, state }: BlockingContext): Blocking | null {
	const { plan, devices } = state;
	const waiting = devices.find(
		(device) =>
			!device.isLive &&
			plan.targets.some((target) => target.deviceId === device.id),
	);
	if (!waiting) return null;
	const gaveUp =
		waiting.live.kind === "failed" || waiting.live.kind === "unreachable";
	return gaveUp
		? {
				text: t(
					"deploy.frame.cantConnect",
					"Couldn't connect to {{device}}. Deploying needs a live connection.",
					{ device: waiting.name },
				),
			}
		: {
				text: t("deploy.frame.connecting", "Connecting to {{device}}…", {
					device: waiting.name,
				}),
				busy: true,
			};
}

/** Endpoint & limits: sandbox limits the device would refuse when the service is created. */
function endpointBlocking({ t, state }: BlockingContext): Blocking | null {
	return limitsOutOfRange(state.plan)
		? {
				text: t(
					"deploy.frame.limits",
					"Set limits the device accepts: CPU above 0, memory of at least 64 MiB, 16 to 65,536 processes and at least 16 MiB of disk.",
				),
			}
		: null;
}

const STEP_BLOCKING: Partial<
	Record<DeployStepId, (context: BlockingContext) => Blocking | null>
> = {
	what: whatBlocking,
	how: howBlocking,
	settings: settingsBlocking,
	where: whereBlocking,
	endpoint: endpointBlocking,
};

/** Why Continue is off on this step: the first failing check, then what only the frame knows (preparation, stored values, live connection, limits). */
function stepBlocking(
	step: DeployStepId,
	context: BlockingContext,
): Blocking | null {
	const { t, state } = context;
	const issue = state.check.issues.find(
		(row) => row.step === step && row.severity === "error",
	);
	if (issue)
		return (
			gatedBlocking(context, issue) ?? {
				text: issueText(t, issue, planNames(t, state.plan)),
			}
		);
	return STEP_BLOCKING[step]?.(context) ?? null;
}

const HOLDING: readonly DeployStepId[] = [
	"what",
	"how",
	"settings",
	"endpoint",
];

/** What the plan's own checks can't see and Review would deploy over: a settled frame blocker of an earlier step. */
function reviewHold(
	context: BlockingContext,
): { step: DeployStepId; text: string } | null {
	for (const step of HOLDING) {
		const blocking = STEP_BLOCKING[step]?.(context);
		if (blocking && !blocking.busy) return { step, text: blocking.text };
	}
	return null;
}

function ReviewHold({
	step,
	text,
	onOpen,
}: Readonly<{ step: string; text: string; onOpen(): void }>) {
	const { t } = useTranslation("devices");
	return (
		<GateNotice
			kind="policy"
			title={t(
				"deploy.frame.holdTitle",
				"{{step}} needs your attention first.",
				{
					step,
				},
			)}
			text={text}
			actions={
				<DvButton size="sm" onClick={onOpen}>
					{t("deploy.frame.holdOpen", "Go to {{step}}", { step })}
				</DvButton>
			}
		/>
	);
}

/** The stepper spans both columns, each step as wide as its label; where eight labels don't fit on one line they wrap instead of ending in "…". */
const STEPPER =
	"max-w-[1124px] [&_li]:basis-auto @max-[1100px]/dwiz:[&_li>span:last-child]:leading-[15px] @max-[1100px]/dwiz:[&_li>span:last-child]:whitespace-normal";

/** Phone foot (APP §3.17): the primary on its own row, Back and Discard… side by side under it. */
const PHONE_BACK = "@max-[480px]/wfoot:order-2 @max-[480px]/wfoot:flex-1";
const PHONE_DISCARD = "@max-[480px]/wfoot:order-3 @max-[480px]/wfoot:flex-1";

function FootButtons({
	blocking,
	statusId,
	primarySlot,
	onBack,
	onNext,
}: Readonly<{
	blocking: Blocking | null;
	statusId: string;
	/** Review renders the page's one primary itself and places it here, after Back. */
	primarySlot: boolean;
	onBack?: () => void;
	onNext?: () => void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div className="flex flex-wrap items-center gap-2 @max-[480px]/wfoot:contents">
			{onBack ? (
				<DvButton icon={ChevronLeft} onClick={onBack} className={PHONE_BACK}>
					{t("deploy.frame.back", "Back")}
				</DvButton>
			) : null}
			{primarySlot ? (
				<span
					data-deploy-foot-slot=""
					className="contents @max-[480px]/wfoot:order-1 @max-[480px]/wfoot:flex @max-[480px]/wfoot:basis-full @max-[480px]/wfoot:flex-col"
				/>
			) : null}
			{onNext ? (
				<DvButton
					variant="primary"
					aria-disabled={blocking ? true : undefined}
					aria-describedby={blocking ? statusId : undefined}
					onClick={onNext}
					className="@max-[480px]/wfoot:order-1 @max-[480px]/wfoot:basis-full"
				>
					{t("deploy.frame.continue", "Continue")}
					<ChevronRight aria-hidden className="size-4" />
				</DvButton>
			) : null}
		</div>
	);
}

function FootStatus({
	blocking,
	id,
}: Readonly<{ blocking: Blocking; id?: string }>) {
	const Icon = blocking.busy ? LoaderCircle : Ban;
	return (
		<span
			id={id}
			data-foot-blocking={id ? "" : undefined}
			className={
				blocking.busy
					? "inline-flex items-start gap-1.5"
					: "inline-flex items-start gap-1.5 text-critical"
			}
		>
			<Icon
				aria-hidden
				className={
					blocking.busy
						? "mt-px size-3.5 shrink-0 animate-spin motion-reduce:animate-none"
						: "mt-px size-3.5 shrink-0"
				}
			/>
			<span>{blocking.text}</span>
		</span>
	);
}

function StepBody({
	step,
	props,
}: Readonly<{ step: DeployStepId; props: PlanStepProps }>) {
	const { route: _route, state: _state, prepare: _prepare, ...shared } = props;
	const later: DeployStepProps = shared;
	switch (step) {
		case "what":
			return <WhatStep {...props} />;
		case "how":
			return <HowItRunsStep {...props} />;
		case "where":
			return <WhereStep {...props} />;
		case "settings":
			return <SettingsStep {...props} />;
		case "endpoint":
			return <EndpointLimitsStep {...props} />;
		case "access_cost":
			return <AccessCostStep {...later} />;
		case "copy_upload":
			return <CopyUploadStep {...later} />;
		case "review":
			return <ReviewStep {...later} />;
		default:
			return <RolloutStep {...later} />;
	}
}

/** "Discard…": asks first, then forgets the saved progress and starts over at What. */
function useDiscard(
	state: DeployDraftState,
	goTo: (step: DeployStepId) => void,
): () => void {
	const { t } = useTranslation("devices");
	const confirm = useConfirm();
	const { plan, draft, discard } = state;
	return () => {
		void confirm({
			icon: Trash2,
			title: t("deploy.discard.title", "Discard this deploy?"),
			sub: plan.app?.name ?? t("deploy.discard.nothing", "Nothing chosen yet"),
			rows: {
				what: t(
					"deploy.discard.what",
					"Your choices in this window are cleared.",
				),
				who:
					draft.entry === "update"
						? t(
								"deploy.discard.whoUpdate",
								"Nobody else. The services keep their current settings.",
							)
						: t(
								"deploy.discard.who",
								"Nobody else. Nothing was created on any device.",
							),
				when: t("deploy.discard.when", "Immediately."),
				undo: {
					reversible: false,
					text: t("deploy.discard.undo", "Start a new deploy."),
				},
			},
			confirmLabel: t("deploy.discard.confirm", "Discard deploy"),
			tone: "danger",
			strength: "none",
		}).then((result) => {
			if (!result.ok) return;
			discard();
			goTo("what");
		});
	};
}

function ResumedBanner({
	step,
	onStartOver,
	onDismiss,
}: Readonly<{ step: string; onStartOver(): void; onDismiss(): void }>) {
	const { t } = useTranslation("devices");
	return (
		<Banner
			tone="info"
			icon={History}
			title={t(
				"deploy.frame.resumed",
				"Picked up where you left off: {{step}}.",
				{ step },
			)}
			actions={
				<>
					<DvButton size="sm" variant="ghost" onClick={onStartOver}>
						{t("deploy.frame.startOver", "Start over…")}
					</DvButton>
					<DvButton size="sm" variant="ghost" onClick={onDismiss}>
						{t("deploy.frame.dismiss", "Dismiss")}
					</DvButton>
				</>
			}
		>
			{t(
				"deploy.frame.resumedText",
				"Your choices were kept in this window. Secrets and access tokens are never kept, so enter them again.",
			)}
		</Banner>
	);
}

/** A local-only app on web: said on every step but How it runs, which shows the gate itself. */
function LocalWebNotice({ app }: Readonly<{ app: string }>) {
	const { t } = useTranslation("devices");
	return (
		<GateNotice
			kind="platform"
			title={t(
				"deploy.frame.localWebTitle",
				"Local-only apps deploy from the desktop app.",
			)}
			text={t(
				"deploy.frame.localWeb",
				"{{app}} exists only on the computer that created it, so the browser can't prepare its offline copy. You can look through the steps; Deploy stays unavailable here.",
				{ app },
			)}
		/>
	);
}

interface FooterProps {
	summary: DeploySummaryInput;
	blocking: Blocking | null;
	/** "Step 3 of 8 · Where it runs · next: Settings" pieces. */
	stepLabel: string;
	nextStepLabel?: string;
	onDiscard(): void;
	onBack?: () => void;
	onNext?: () => void;
}

/** The sticky foot: the summary line below 960 px, Discard, the step note or why Continue is off, Back and Continue. */
function WizardFooter({
	summary,
	blocking,
	stepLabel: label,
	nextStepLabel,
	onDiscard,
	onBack,
	onNext,
}: Readonly<FooterProps>) {
	const { t } = useTranslation("devices");
	const statusId = useId();
	return (
		<div className="sticky bottom-2 z-5 flex flex-col gap-2">
			<DeploySummaryBar
				{...summary}
				className="hidden @max-[960px]/dwiz:flex"
			/>
			<WizardFoot
				className="static"
				step={summary.steps.indexOf(summary.current) + 1}
				total={summary.steps.length}
				stepLabel={label}
				nextStepLabel={nextStepLabel}
				status={
					blocking ? (
						<FootStatus blocking={blocking} id={statusId} />
					) : undefined
				}
				cancel={
					<>
						{blocking ? (
							// The foot hides its note below 720 px; the reason stays visible (R7).
							<span
								aria-hidden
								className="hidden basis-full text-xs text-muted-foreground @max-[720px]/wfoot:block"
							>
								<FootStatus blocking={blocking} />
							</span>
						) : null}
						<DvButton
							variant="danger-ghost"
							icon={Trash2}
							onClick={onDiscard}
							className={PHONE_DISCARD}
						>
							{t("deploy.frame.discard", "Discard…")}
						</DvButton>
						<span className="hidden flex-1 @max-[720px]/wfoot:block @max-[480px]/wfoot:hidden" />
					</>
				}
			>
				<FootButtons
					blocking={blocking}
					statusId={statusId}
					primarySlot={summary.current === "review"}
					onBack={onBack}
					onNext={onNext}
				/>
			</WizardFoot>
		</div>
	);
}

function Wizard({
	route,
	scope,
}: Readonly<{ route: DeployRoute; scope: DevicesScope }>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const body = useRef<HTMLDivElement>(null);
	const state = useDeployDraft(route, scope);
	const { plan, check, mode } = state;
	const steps = useMemo(() => deploySteps(mode), [mode]);
	const step = currentStep(route, steps, state);
	const index = steps.indexOf(step);
	const prepare = useDeployPrepare(plan, { enabled: PREPARING.includes(step) });
	const limitsOnly =
		plan.services.length > 0 && !plan.services.some((row) => row.hosted);
	const prefilled = useMemo(() => prefilledSteps(route, scope), [route, scope]);

	const { setCatalog, setReached, loading } = state;
	const approved = prepare.prepared?.approved?.catalog ?? null;
	useEffect(() => setCatalog(approved), [setCatalog, approved]);
	useEffect(() => {
		if (!loading) setReached(index);
	}, [loading, setReached, index]);
	// The URL always carries the step, so a reload or a shared link lands on it.
	useEffect(() => {
		if (!loading && route.step !== step)
			navigate({ ...route, step }, { replace: true });
	}, [loading, route, step, navigate]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: focus moves when the step changes, not on every render
	useEffect(() => {
		body.current?.querySelector<HTMLElement>("h2[tabindex]")?.focus();
	}, [step]);

	const goTo = useCallback(
		(next: DeployStepId) =>
			navigate({ ...route, step: next }, { replace: true }),
		[navigate, route],
	);
	const { markDeployed } = state;
	const onFinished = useCallback(
		(result: DeployResult) => markDeployed(result.outcome),
		[markDeployed],
	);
	const runState = useDeployRunState(state.draft.deploymentId);
	const time = useAreaTime();
	const blockingContext: BlockingContext = { t, time, state, prepare };
	const blocking = stepBlocking(step, blockingContext);
	const hold = step === "review" ? reviewHold(blockingContext) : null;
	const blockingText = check.firstBlocking
		? (gatedBlocking(blockingContext, check.firstBlocking)?.text ??
			issueText(t, check.firstBlocking, planNames(t, plan)))
		: undefined;
	const next = index < REVIEW_INDEX ? steps[index + 1] : undefined;
	const localWeb = check.issues.some(
		(issue) => issue.code === "local_only_web",
	);

	const discard = useDiscard(state, goTo);

	const summary: DeploySummaryInput = {
		plan,
		check,
		steps,
		current: step,
		reached: state.reached,
		prefilled,
		limitsOnly,
		...(state.versionLabel ? { versionLabel: state.versionLabel } : {}),
		prepared: prepare.prepared,
		deployed: state.deployed,
		run: runState,
		...(state.outcome ? { outcome: state.outcome } : {}),
		goTo,
	};
	const running = step === "rollout";
	const stepProps: PlanStepProps = {
		scope,
		route,
		draft: state.draft,
		plan,
		check,
		update: state.update,
		goTo,
		onFinished,
		prepared: prepare.prepared,
		...(blockingText ? { blockingText } : {}),
		reportDeviceCheck: state.reportDeviceCheck,
		startOver: discard,
		// Rollout has its own foot: the summary's one-line form goes there below 960 px.
		...(running
			? {
					summaryBar: (
						<DeploySummaryBar
							{...summary}
							className="hidden @max-[960px]/dwiz:flex"
						/>
					),
				}
			: {}),
		state,
		prepare,
	};
	const exit = t("deploy.frame.exit", "Exit deploy");
	const exitTitle = t(
		"deploy.frame.exitTitle",
		"Your choices stay in this window, so you can come back and continue.",
	);

	return (
		<div
			data-deploy-wizard={state.key}
			data-step={step}
			className="@container/dwiz flex min-w-0 flex-col gap-4"
		>
			<header className="flex flex-col gap-3">
				<Crumbs route={route} scope={scope} state={state} />
				<WizardTitleRow
					exitLabel={exit}
					exitTitle={exitTitle}
					{...(route.from === "events"
						? { exitHref: deployExitHref(route, scope) }
						: { onExit: () => navigate(exitRoute(route, scope)) })}
					title={wizardTitle(t, state)}
					sub={wizardSub(t, state)}
					className="@max-[720px]/dwiz:flex-col @max-[720px]/dwiz:items-start @max-[720px]/dwiz:gap-2"
				/>
			</header>
			{running ? (
				<div data-deploy-headline-slot="" className="contents" />
			) : (
				<PlanningHeadline state={state} stepIndex={index} />
			)}
			<WizardStepper
				label={t("deploy.frame.steps", "Deploy steps")}
				steps={steps.map((id) => stepLabel(t, id, limitsOnly))}
				current={index}
				className={STEPPER}
			/>
			<div className="grid max-w-[1124px] grid-cols-[minmax(0,760px)_minmax(280px,340px)] items-start gap-x-6 gap-y-4 @max-[960px]/dwiz:grid-cols-1">
				<div className="flex min-w-0 flex-col gap-4">
					{state.resumed && !running ? (
						<ResumedBanner
							step={t(
								"deploy.frame.resumedStep",
								"step {{n, number}} of {{total, number}} · {{step}}",
								{
									n: index + 1,
									total: steps.length,
									step: stepTitle(t, step, limitsOnly),
								},
							)}
							onStartOver={discard}
							onDismiss={state.dismissResumed}
						/>
					) : null}
					{state.loading ? (
						<StateView kind="loading" rows={4} />
					) : state.error ? (
						<StateView
							kind="error"
							title={t("deploy.frame.appError", "Couldn't load this app")}
							text={state.error.message}
						/>
					) : (
						<div ref={body} className="flex min-w-0 flex-col gap-4">
							{running ? null : (
								<>
									{plan.app && step !== "how" && localWeb ? (
										<LocalWebNotice app={plan.app.name} />
									) : null}
									<TargetGates state={state} step={step} goTo={goTo} />
								</>
							)}
							{hold ? (
								<ReviewHold
									step={stepTitle(t, hold.step, limitsOnly)}
									text={hold.text}
									onOpen={() => goTo(hold.step)}
								/>
							) : (
								<StepBody step={step} props={stepProps} />
							)}
						</div>
					)}
					{running ? null : (
						<WizardFooter
							summary={summary}
							blocking={blocking}
							stepLabel={stepTitle(t, step, limitsOnly)}
							nextStepLabel={next ? stepTitle(t, next, limitsOnly) : undefined}
							onDiscard={discard}
							onBack={index > 0 ? () => goTo(steps[index - 1]) : undefined}
							onNext={
								next
									? () => {
											if (!blocking) goTo(next);
										}
									: undefined
							}
						/>
					)}
				</div>
				<aside className="sticky top-4 flex min-w-0 flex-col gap-3 @max-[960px]/dwiz:hidden">
					<DeploySummary {...summary} />
				</aside>
			</div>
		</div>
	);
}

export function DeployWizard({ route, scope }: Readonly<ScreenProps>) {
	return route.screen === "deploy" ? (
		<Wizard route={route} scope={scope} />
	) : null;
}
