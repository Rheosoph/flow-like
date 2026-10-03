"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Activity,
	ArrowRight,
	ChevronRight,
	CircleCheck,
	CircleSlash,
	ClipboardList,
	Copy,
	KeyRound,
	LoaderCircle,
	Lock,
	LockOpen,
	type LucideIcon,
	OctagonX,
	Pencil,
	Plus,
	Rocket,
	RotateCcw,
	RotateCw,
	Server,
	TriangleAlert,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import type {
	DeployFailure,
	DeployPhase,
	DeployPlan,
	DeployResult,
} from "../../../../../lib/device-management/model/deploy-plan";
import {
	type DeployRunRow,
	type DeployRunState,
	deployRunResult,
} from "../../../../../lib/device-management/model/deploy-run";
import type {
	DeployRoute,
	DevicesRoute,
	DevicesScope,
} from "../../../../../lib/device-management/model/types";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { Checklist, type ChecklistItem } from "../../primitives/checklist";
import { DvButton } from "../../primitives/dv-button";
import {
	FleetRollout,
	type FleetRolloutRow,
	type FleetRowState,
	type FleetSharedStep,
	fleetRolloutChip,
} from "../../primitives/fleet-rollout";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { Headline } from "../../primitives/headline";
import { IdRef } from "../../primitives/id-ref";
import { InlineResult, type ResultTone } from "../../primitives/inline-result";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { ProgressBar } from "../../primitives/meter";
import { StateView } from "../../primitives/state-view";
import { StatusChip } from "../../primitives/status-chip";
import type { ChipTone } from "../../primitives/tone";
import { useCopy } from "../../primitives/use-copy";
import { WizardFoot, WizardStepHeader } from "../../primitives/wizard";
import { deployExitHref } from "../../routing/devices-href";
import {
	useDevicesRoute,
	useHostLink,
	useRouteLink,
} from "../../routing/use-devices-route";
import { useActivityTray } from "../../shell/activity-tray";
import { useDeviceWorkspace, useOverlay } from "../../workspace";
import { RUN_REFUSALS } from "../deploy-copy";
import type { DeployStepProps } from "../step-props";
import { secretCount } from "../update-path";
import {
	type DeployRunDetails,
	type DeployRunHandle,
	type DeployTargetDetail,
	deployRunExtras,
	deployRunTitle,
	deployRunTitleRef,
	deployWhat,
	recallDeployRun,
	useDeployRun,
	useDeployRunAdopted,
	useDeployRunDetails,
	useDeployRunWatch,
	useHandedBundle,
	useShownTokens,
	useWizardSlot,
} from "../use-deploy-run";
import { PlanSummary } from "./review-step";

/* Copy (APP §3.13, §7.8). */

interface PhaseContext {
	t: DevicesT;
	plan: DeployPlan;
	service: string;
	files: number;
	secrets: number;
	running: boolean;
	/** What the `schedules` phase hands to the service: schedules, bots or both. */
	claims: { schedules: number; bots: number };
}

function claimsLabel({ t, claims }: PhaseContext): string {
	if (!claims.bots)
		return t(
			"devices:deployShip.phase.schedules",
			"Moving its schedules off the hub",
		);
	return claims.schedules
		? t(
				"devices:deployShip.phase.schedulesBots",
				"Moving its schedules and bots to this service",
			)
		: t("devices:deployShip.phase.bots", "Moving its bots to this service");
}

const PHASE_LABEL: Record<DeployPhase, (c: PhaseContext) => string> = {
	approve: ({ t, plan }) =>
		plan.mode === "offline"
			? t("devices:deployShip.phase.modelAccess", "Creating model access")
			: t("devices:deployShip.phase.cloudAccess", "Creating cloud access"),
	spending: ({ t }) =>
		t("devices:deployShip.phase.spending", "Setting the spending limit"),
	upload: ({ t, plan, files }) => {
		if (plan.mode !== "offline")
			return t(
				"devices:deployShip.phase.uploadDefinitions",
				"Uploading definitions and packages",
			);
		return files
			? t("devices:deployShip.phase.uploadFiles", {
					count: files,
					defaultValue_one: "Uploading {{count, number}} file",
					defaultValue_other: "Uploading {{count, number}} files",
				})
			: t("devices:deployShip.phase.uploadCopy", "Uploading the copy");
	},
	check_events: ({ t }) =>
		t("devices:deployShip.phase.checkEvents", "Checking events on the device"),
	install: ({ t }) =>
		t("devices:deployShip.phase.install", "Installing the app version"),
	schedules: claimsLabel,
	create: ({ t, service }) =>
		t(
			"devices:deployShip.phase.create",
			"Creating {{service}} with settings v1",
			{ service },
		),
	secrets: ({ t, secrets }) =>
		secrets
			? t("devices:deployShip.phase.secretsCount", {
					count: secrets,
					defaultValue_one: "Saving {{count, number}} secret",
					defaultValue_other: "Saving {{count, number}} secrets",
				})
			: t("devices:deployShip.phase.secrets", "Saving secrets"),
	start: ({ t, service }) =>
		t("devices:deployShip.phase.start", "Starting {{service}}", { service }),
	prepare_update: ({ t }) =>
		t("devices:deployShip.phase.prepareUpdate", "Preparing the update"),
	check_new: ({ t }) =>
		t("devices:deployShip.phase.checkNew", "Checking new version"),
	switch: ({ t }) => t("devices:deployShip.phase.switch", "Switching over"),
	stop: ({ t, service, running }) =>
		running
			? t("devices:deployShip.phase.stop", "Stopping {{service}}", { service })
			: t("devices:deployShip.phase.apply", "Applying the new settings"),
};

/** A failure's code names the kind it is about: a bot's refusals start with `bot_`. */
type FailedCode = Pick<DeployFailure, "code"> | null | undefined;

const FAILED_WHILE: Record<
	DeployPhase,
	(t: DevicesT, error?: FailedCode) => string
> = {
	approve: (t) =>
		t("devices:deployShip.while.approve", "creating its cloud access"),
	spending: (t) =>
		t("devices:deployShip.while.spending", "setting the spending limit"),
	upload: (t) => t("devices:deployShip.while.upload", "uploading"),
	check_events: (t) =>
		t("devices:deployShip.while.checkEvents", "checking the events"),
	install: (t) => t("devices:deployShip.while.install", "installing"),
	schedules: (t, error) =>
		error?.code.startsWith("bot_")
			? t("devices:deployShip.while.bots", "moving its bots to this service")
			: t(
					"devices:deployShip.while.schedules",
					"moving its schedules off the hub",
				),
	create: (t) => t("devices:deployShip.while.create", "creating the service"),
	secrets: (t) => t("devices:deployShip.while.secrets", "saving secrets"),
	start: (t) => t("devices:deployShip.while.start", "starting"),
	prepare_update: (t) =>
		t("devices:deployShip.while.prepareUpdate", "preparing the update"),
	check_new: (t) =>
		t("devices:deployShip.while.checkNew", "checking the new version"),
	switch: (t) => t("devices:deployShip.while.switch", "switching over"),
	stop: (t) => t("devices:deployShip.while.apply", "applying the new settings"),
};

const REASON: Record<string, (t: DevicesT, detail: string) => string> = {
	...RUN_REFUSALS,
	unauthorized: (t) =>
		t(
			"devices:deployShip.fail.unauthorized",
			"your access to this device doesn't allow it",
		),
	revision_conflict: (t) =>
		t(
			"devices:deployShip.fail.changed",
			"the service changed on the device in the meantime",
		),
	stale: (t) =>
		t(
			"devices:deployShip.fail.changed",
			"the service changed on the device in the meantime",
		),
	invalid: (t) =>
		t("devices:deployShip.fail.invalid", "the device refused these settings"),
	invalid_plan: (t) =>
		t(
			"devices:deployShip.fail.invalidPlan",
			"these settings can't be applied as they are",
		),
	host_policy: (t) =>
		t(
			"devices:deployShip.fail.hostPolicy",
			"the device's isolation rules don't allow these limits",
		),
	unsupported: (t) =>
		t(
			"devices:deployShip.fail.unsupported",
			"the device agent doesn't support this yet",
		),
	limit: (t) =>
		t("devices:deployShip.fail.limit", "a limit on the device was reached"),
	busy: (t) => t("devices:deployShip.fail.busy", "the device was busy"),
	failed: (t) =>
		t("devices:deployShip.fail.failed", "the device couldn't complete it"),
	upload_unconfirmed: (t) =>
		t(
			"devices:deployShip.fail.uploadUnconfirmed",
			"the upload wasn't confirmed; it can resume",
		),
	unconfirmed: (t) =>
		t("devices:deployShip.fail.unconfirmed", "the device stopped answering"),
	hub_refused: (t) =>
		t("devices:deployShip.fail.hubRefused", "the hub refused it"),
	service_exists: (t) =>
		t(
			"devices:deployShip.fail.serviceExists",
			"a service with this ID already runs on the device",
		),
	device_unread: (t) =>
		t(
			"devices:deployShip.fail.deviceUnread",
			"the device's services couldn't be read",
		),
	secret_publication: (t) =>
		t(
			"devices:deployShip.fail.secret",
			"a secret couldn't be saved on the device",
		),
	review_required: (t) =>
		t(
			"devices:deployShip.fail.review",
			"the service was stopped or is being updated by someone else",
		),
	update_in_progress: (t) =>
		t(
			"devices:deployShip.fail.updateInProgress",
			"another update of this service is still in progress",
		),
	rolled_back: (t) =>
		t(
			"devices:deployShip.fail.rolledBack",
			"the new version didn't become healthy in time",
		),
	rollout_failed: (t) =>
		t(
			"devices:deployShip.fail.rolloutFailed",
			"the update failed on the device",
		),
	rollout_cancelled: (t) =>
		t("devices:deployShip.fail.rolloutCancelled", "the update was discarded"),
	source_changed: (t) =>
		t(
			"devices:deployShip.fail.sourceChanged",
			"the service runs the app another way than this deploy",
		),
	queue_not_empty: (t) =>
		t(
			"devices:deployShip.fail.queue",
			"buffered changes are still waiting to reach the cloud",
		),
	event_refused: (t) =>
		t(
			"devices:deployShip.fail.eventRefused",
			"the device can't run one of the events",
		),
	not_prepared: (t) =>
		t(
			"devices:deployShip.fail.notPrepared",
			"the app version wasn't prepared on this computer",
		),
	local_only_web: (t) =>
		t(
			"devices:deployShip.fail.localOnlyWeb",
			"local-only apps deploy from the desktop app",
		),
	no_reply: (t) =>
		t("devices:deployShip.fail.noReply", "no reply was received"),
	interrupted: (t) =>
		t(
			"devices:deployShip.fail.interrupted",
			"it was stopped before it finished",
		),
	tray_failed: (t) =>
		t("devices:deployShip.fail.trayFailed", "it didn't finish"),
	applied_only: (t) =>
		t(
			"devices:deployShip.fail.appliedOnly",
			"the device applied it, but this page closed before the steps after it; open the service to check it",
		),
};

const HUB_PHASES: readonly DeployPhase[] = ["approve", "spending"];

function unknownReason(t: DevicesT): string {
	return t(
		"devices:deployShip.fail.unknown",
		"it couldn't be confirmed; the device may have stopped answering",
	);
}

function failureReason(t: DevicesT, error: DeployFailure): string {
	const hub = HUB_PHASES.includes(error.phase);
	if (hub && error.code === "unconfirmed")
		return t("devices:deployShip.fail.hub", "the hub didn't confirm it");
	const known = Object.hasOwn(REASON, error.code)
		? REASON[error.code]
		: undefined;
	return (known ?? unknownReason)(t, error.detail ?? "");
}

function listOf(time: AreaTime, values: readonly string[]): string {
	return new Intl.ListFormat(time.locale, {
		style: "long",
		type: "conjunction",
	}).format(values);
}

/* Rows. */

interface RowContext {
	t: DevicesT;
	time: AreaTime;
	plan: DeployPlan;
	files: number;
	details: DeployRunDetails;
}

function phaseLabels(c: RowContext, row: DeployRunRow): string[] {
	const detail = c.details[row.target];
	const target = c.plan.targets.find(
		(value) => value.deviceId === row.deviceId,
	);
	const service = target?.services.find(
		(value) => value.serviceId === row.serviceId,
	);
	const context: PhaseContext = {
		t: c.t,
		plan: c.plan,
		service: row.serviceId ?? "",
		files: c.files,
		secrets: target && service ? secretCount(c.plan, target, service) : 0,
		running: detail?.wasRunning !== false,
		claims: {
			schedules: service?.addedSchedules.length ?? 0,
			bots: service?.addedBots.length ?? 0,
		},
	};
	return row.phases.map((phase) => PHASE_LABEL[phase](context));
}

/** After an approval exists, a failed device keeps it on the hub; the row says so instead of "Nothing changed". */
function keptText(c: RowContext, row: DeployRunRow): string | undefined {
	const kept = c.details[row.target]?.kept;
	const error = row.error;
	if (row.state !== "failed" || !kept || !error || error.rolledBack)
		return undefined;
	const params = {
		phase: FAILED_WHILE[error.phase](c.t, error),
		reason: failureReason(c.t, error),
		device: c.details[row.target]?.deviceName ?? "",
	};
	if (kept === "approval_spending")
		return c.t(
			"devices:deployShip.row.keptBoth",
			"Failed while {{phase}}: {{reason}}. Its access and spending limit stay on the hub, so a retry reuses them. Nothing else changed on {{device}}.",
			params,
		);
	return c.t(
		"devices:deployShip.row.keptAccess",
		"Failed while {{phase}}: {{reason}}. Its access stays on the hub, so a retry reuses it. Nothing else changed on {{device}}.",
		params,
	);
}

function doneText(c: RowContext, row: DeployRunRow): string | undefined {
	const detail = c.details[row.target];
	if (row.state !== "done" || !detail) return undefined;
	const params = {
		time: row.at === undefined ? "" : c.time.clock(row.at),
		service: detail.serviceId,
		settings: detail.toSettings ?? 1,
	};
	if (detail.kind !== "new")
		return detail.stopped
			? c.t(
					"devices:deployShip.row.updatedStopped",
					"Updated at {{time}} · settings v{{settings}} · stays stopped",
					params,
				)
			: c.t(
					"devices:deployShip.row.updated",
					"Updated at {{time}} · settings v{{settings}}",
					params,
				);
	return detail.stopped
		? c.t(
				"devices:deployShip.row.deployedStopped",
				"Deployed at {{time}} · {{service}} stays stopped",
				params,
			)
		: c.t(
				"devices:deployShip.row.deployed",
				"Deployed at {{time}} · {{service}} running",
				params,
			);
}

function rowState(row: DeployRunRow): FleetRowState {
	return row.state === "failed" && row.error?.rolledBack
		? "rolled_back"
		: row.state;
}

function fleetRow(
	c: RowContext,
	row: DeployRunRow,
	actions: ReactNode,
): FleetRolloutRow {
	const detail = c.details[row.target];
	const error = row.error;
	return {
		id: row.target,
		device: detail?.deviceName ?? row.deviceId,
		service: row.serviceId,
		phases: phaseLabels(c, row),
		step: row.phase,
		state: rowState(row),
		text: keptText(c, row) ?? doneText(c, row),
		at: row.at,
		progress: row.progress,
		actions,
		...(error
			? {
					reason: failureReason(c.t, error),
					failedWhile: FAILED_WHILE[error.phase](c.t, error),
				}
			: {}),
	};
}

/* Row actions. */

interface Navigation {
	scope: DevicesScope;
	route: DeployRoute;
	appScope: DevicesScope;
}

function serviceRoute(row: DeployRunRow): DevicesRoute {
	return {
		screen: "service",
		deviceId: row.deviceId,
		serviceId: row.serviceId ?? "",
		tab: "status",
	};
}

function RowActions({
	row,
	run,
	first,
	gated,
	navigation,
}: Readonly<{
	row: DeployRunRow;
	run: DeployRunHandle;
	/** The first held row carries the run's Continue / Stop. */
	first: boolean;
	gated: boolean;
	navigation: Navigation;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { openUnlock } = useOverlay();
	if (row.state === "done")
		return (
			<DvButton size="xs" variant="ghost" asChild>
				<a {...link(serviceRoute(row), { scope: navigation.scope })}>
					{t("deployShip.rollout.openService", "Open service")}
				</a>
			</DvButton>
		);
	if (row.state === "failed")
		return (
			<>
				<DvButton
					size="xs"
					icon={RotateCw}
					disabled={gated}
					onClick={() => run.retry(row.target)}
				>
					{t("deployShip.rollout.retryDevice", "Retry this device")}
				</DvButton>
				<DvButton
					size="xs"
					variant="ghost"
					onClick={() => run.skip(row.target)}
				>
					{t("deployShip.rollout.skip", "Skip")}
				</DvButton>
			</>
		);
	if (row.state === "blocked")
		return (
			<DvButton
				size="xs"
				icon={LockOpen}
				onClick={() => openUnlock(row.deviceId, { connectLive: true })}
			>
				{t("deployShip.rollout.unlock", "Unlock…")}
			</DvButton>
		);
	if (row.state !== "held" || !first) return null;
	return (
		<>
			<DvButton size="xs" disabled={gated} onClick={run.resume}>
				{t("deployShip.rollout.continue", "Continue with the rest")}
			</DvButton>
			<DvButton size="xs" variant="ghost" onClick={run.stop}>
				{t("deployShip.rollout.stopHere", "Stop here")}
			</DvButton>
		</>
	);
}

/* Result (APP §7.8). */

interface ResultContext extends RowContext {
	state: DeployRunState;
	result: DeployResult;
	update: boolean;
}

function firstFailure(c: ResultContext): string {
	const [failure] = c.result.failed;
	const shared = c.result.sharedFailure;
	if (!failure)
		return shared
			? c.t(
					"devices:deployShip.result.sharedFailed",
					"Preparing on this computer failed: {{reason}}.",
					{ reason: failureReason(c.t, shared) },
				)
			: c.t(
					"devices:deployShip.result.stopped",
					"You stopped before any device finished.",
				);
	const detail = c.details[failure.target];
	const params = {
		device: detail?.deviceName ?? "",
		service: detail?.serviceId ?? "",
		phase: FAILED_WHILE[failure.phase](c.t, failure),
		reason: failureReason(c.t, failure),
	};
	return failure.rolledBack
		? c.t(
				"devices:deployShip.result.rolledBack",
				"{{service}} rolled back on {{device}}; the previous version is running again there.",
				params,
			)
		: c.t(
				"devices:deployShip.result.failedWhile",
				"{{device}} failed while {{phase}}: {{reason}}.",
				params,
			);
}

function allDoneText(c: ResultContext): string {
	const total = c.state.rows.length;
	const time = c.time.clock(c.result.at);
	const [only] = c.state.rows;
	if (c.update)
		return total === 1
			? c.t(
					"devices:deployShip.result.updatedOne",
					"Updated {{service}} at {{time}}.",
					{
						service: only?.serviceId ?? "",
						time,
					},
				)
			: c.t(
					"devices:deployShip.result.updatedAll",
					"Updated {{count, number}} services at {{time}}.",
					{ count: total, time },
				);
	if (total === 1)
		return c.t(
			"devices:deployShip.result.deployedOne",
			"Deployed to {{device}} at {{time}}.",
			{
				device: (only && c.details[only.target]?.deviceName) ?? "",
				time,
			},
		);
	return total === 2
		? c.t(
				"devices:deployShip.result.deployedBoth",
				"Deployed to both devices at {{time}}.",
				{ time },
			)
		: c.t(
				"devices:deployShip.result.deployedAll",
				"Deployed to all {{count, number}} devices at {{time}}.",
				{ count: total, time },
			);
}

function partialText(c: ResultContext): string {
	const params = { done: c.result.done.length, total: c.state.rows.length };
	const lead = c.update
		? c.t(
				"devices:deployShip.result.updatedSome",
				"Updated {{done, number}} of {{total, number}} services.",
				params,
			)
		: c.t(
				"devices:deployShip.result.deployedSome",
				"Deployed to {{done, number}} of {{total, number}} devices.",
				params,
			);
	return `${lead} ${firstFailure(c)}`;
}

function noneText(c: ResultContext): string {
	const lead = c.update
		? c.t("devices:deployShip.result.updatedNone", "Nothing was updated.")
		: c.t("devices:deployShip.result.deployedNone", "Nothing was deployed.");
	return `${lead} ${firstFailure(c)}`;
}

const RESULT_TEXT: Record<
	DeployResult["outcome"],
	{ tone: ResultTone; text(c: ResultContext): string }
> = {
	all: { tone: "good", text: allDoneText },
	partial: { tone: "warning", text: partialText },
	none: { tone: "critical", text: noneText },
};

/** "crm-webhook runs on edge-berlin-01 and studio-mac-mini." */
function whereText(c: ResultContext): string | undefined {
	const running = c.result.done.flatMap((target) => {
		const detail = c.details[target];
		return detail && !detail.stopped ? [detail] : [];
	});
	if (!running.length) return undefined;
	const services = [...new Set(running.map((detail) => detail.serviceId))];
	return c.t(
		"devices:deployShip.result.runsOn",
		"{{services}} runs on {{devices}}.",
		{
			services: listOf(c.time, services),
			devices: listOf(
				c.time,
				running.map((detail) => detail.deviceName),
			),
		},
	);
}

/* Lead sentence. */

function runningLead(c: RowContext, state: DeployRunState, what: string) {
	const total = state.rows.length;
	const done = state.rows.filter((row) => row.state === "done").length;
	const active = state.rows.find((row) => row.state === "active");
	const [only] = state.rows;
	const device = (row: DeployRunRow | undefined) =>
		(row && c.details[row.target]?.deviceName) ?? "";
	const lead =
		total === 1
			? c.t(
					"devices:deployShip.lead.runningOne",
					"Deploying {{what}} to {{device}}.",
					{
						what,
						device: device(only),
					},
				)
			: c.t(
					"devices:deployShip.lead.runningMany",
					"Deploying {{what}} on {{count, number}} devices.",
					{ what, count: total },
				);
	const now = active
		? c.t("devices:deployShip.lead.activeNow", "{{device}}: {{phase}}.", {
				device: device(active),
				phase: phaseLabels(c, active)[active.phase] ?? "",
			})
		: "";
	const rest = c.t(
		"devices:deployShip.lead.progress",
		"{{done, number}} of {{total, number}} done. You can leave this page; each device keeps going and Activity tracks it.",
		{ done, total },
	);
	return { lead, rest: `${now} ${rest}`.trim() };
}

/** What the first failed device keeps on the hub; empty when it kept nothing. */
function keptNote(c: ResultContext): string {
	const [failure] = c.result.failed;
	const detail = failure ? c.details[failure.target] : undefined;
	if (!failure || failure.rolledBack || !detail?.kept) return "";
	const device = detail.deviceName;
	return detail.kept === "approval_spending"
		? c.t(
				"devices:deployShip.lead.keptBoth",
				"{{device}} keeps its access and spending limit on the hub; nothing else changed there.",
				{ device },
			)
		: c.t(
				"devices:deployShip.lead.keptAccess",
				"{{device}} keeps its access on the hub; nothing else changed there.",
				{ device },
			);
}

/** Under a lead that already says how many devices have it: only why, and what stays. */
function failedRest(c: ResultContext): string {
	return [firstFailure(c), keptNote(c)].filter(Boolean).join(" ");
}

function doneRest(c: ResultContext): string {
	const text = allDoneText(c);
	if (c.state.rows.length < 2) return text;
	const pointer = c.t(
		"devices:deployShip.lead.openInDevices",
		"Open in Devices shows where it runs and at which version.",
	);
	return `${text} ${pointer}`;
}

function finishedLead(c: ResultContext, what: string) {
	const total = c.state.rows.length;
	const done = c.result.done.length;
	if (c.result.outcome === "none")
		return {
			lead: c.t("devices:deployShip.lead.none", "{{what}} wasn't deployed.", {
				what,
			}),
			rest: failedRest(c),
		};
	if (c.result.outcome === "partial")
		return {
			lead: c.t(
				"devices:deployShip.lead.some",
				"{{what}} is on {{done, number}} of {{total, number}} devices.",
				{ what, done, total },
			),
			rest: failedRest(c),
		};
	return { lead: allLead(c, what, total), rest: doneRest(c) };
}

function allLead(c: ResultContext, what: string, total: number): string {
	if (total === 1)
		return c.t("devices:deployShip.lead.allOne", "{{what}} is deployed.", {
			what,
		});
	return total === 2
		? c.t("devices:deployShip.lead.allBoth", "{{what}} is on both devices.", {
				what,
			})
		: c.t(
				"devices:deployShip.lead.allMany",
				"{{what}} is on all {{count, number}} devices.",
				{ what, count: total },
			);
}

/* Blocks. */

function TokenBlock({
	deploymentId,
	state,
	details,
}: Readonly<{
	deploymentId: string;
	state: DeployRunState;
	details: DeployRunDetails;
}>) {
	const { t } = useTranslation("devices");
	const tokens = state.rows.flatMap((row) => {
		const detail = details[row.target];
		const held = detail?.token || detail?.tokenGone;
		return row.state === "done" && detail && held ? [detail] : [];
	});
	useShownTokens(
		deploymentId,
		tokens.filter((detail) => detail.token).map((detail) => detail.target),
	);
	if (!tokens.length) return null;
	return (
		<Block
			id="dp-tokens"
			icon={KeyRound}
			title={t("deployShip.tokens.title", "Access tokens · shown once")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t("deployShip.tokens.stamp", "generated on this computer")}
				/>
			}
			foot={t(
				"deployShip.tokens.foot",
				"Clients send the token to call the service. After you leave this page it can't be shown again; set a new one with Change settings.",
			)}
		>
			<ul className="flex flex-col divide-y divide-hairline">
				{tokens.map((detail) => (
					<TokenRow key={detail.target} detail={detail} />
				))}
			</ul>
		</Block>
	);
}

interface TokenRowProps {
	detail: DeployTargetDetail;
}

function TokenRow({ detail }: Readonly<TokenRowProps>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const token = detail.token ?? "";
	const label = t(
		"deployShip.tokens.copyLabel",
		"Copy the access token of {{service}} on {{device}}",
		{ service: detail.serviceId, device: detail.deviceName },
	);
	function copyToken() {
		void copy(token);
	}
	return (
		<li className="flex min-w-0 flex-wrap items-center gap-2 py-2 text-ui first:pt-0 last:pb-0">
			<span
				title={`${detail.deviceName} › ${detail.serviceId}`}
				className="min-w-0 flex-1 truncate font-mono"
			>
				{detail.deviceName}
				<span className="text-muted-foreground"> › {detail.serviceId}</span>
			</span>
			{token ? (
				<>
					<code className="rounded bg-surface-sunken px-1.5 py-0.5 font-mono text-xs">
						{token.slice(0, 10)}…
					</code>
					<DvButton
						size="xs"
						icon={Copy}
						aria-label={copied ? undefined : label}
						onClick={copyToken}
					>
						{copied
							? t("deployShip.tokens.copied", "Copied")
							: t("deployShip.tokens.copy", "Copy")}
					</DvButton>
				</>
			) : (
				<span className="text-xs text-muted-foreground" data-token-gone="">
					{t(
						"deployShip.tokens.gone",
						"Shown once · not kept after you left this page",
					)}
				</span>
			)}
		</li>
	);
}

/* One device: its own block instead of the board (the prototype's single-device rollout). */

function stepState(row: DeployRunRow, index: number): ChecklistItem["state"] {
	if (row.state === "done" || index < row.phase) return "pass";
	if (index > row.phase) return "pending";
	if (row.state === "failed") return "fail";
	return row.state === "active" ? "active" : "pending";
}

function checkStates(row: DeployRunRow, labels: readonly string[]) {
	return labels.map(function item(label, index): ChecklistItem {
		return { id: `${index}`, label, state: stepState(row, index) };
	});
}

const SHARED_CHECK: Record<FleetSharedStep["state"], ChecklistItem["state"]> = {
	waiting: "pending",
	active: "active",
	done: "pass",
	failed: "fail",
};

/** What runs once before the device's steps, as the list's first item. */
function sharedItems(shared: FleetSharedStep | undefined): ChecklistItem[] {
	if (!shared) return [];
	const { label, source } = shared;
	return [{ id: "shared", label, source, state: SHARED_CHECK[shared.state] }];
}

interface SingleCopy {
	t: DevicesT;
	/** The current step's label. */
	phase: string;
	device: string;
	reason: string;
	failedWhile: string;
}

const starting = ({ t }: SingleCopy) =>
	t("devices:deployShip.single.starting", "Starting…");

const SINGLE_SENTENCE: Record<FleetRowState, (c: SingleCopy) => string> = {
	waiting: starting,
	held: starting,
	not_started: starting,
	done: starting,
	active: ({ t, phase }) =>
		t("devices:deployShip.single.active", "{{phase}}…", { phase }),
	failed: ({ t, failedWhile, reason, device }) =>
		t(
			"devices:deployShip.single.failedWhile",
			"Failed while {{phase}}: {{reason}}. Nothing changed on {{device}}.",
			{ phase: failedWhile, reason, device },
		),
	rolled_back: ({ t, reason }) =>
		t(
			"devices:deployShip.single.rolledBack",
			"Rolled back: {{reason}}. The previous version is running again.",
			{ reason },
		),
	blocked: ({ t, device }) =>
		t(
			"devices:deployShip.single.blocked",
			"Waiting: unlock {{device}} to continue.",
			{ device },
		),
	skipped: ({ t, device }) =>
		t(
			"devices:deployShip.single.skipped",
			"Skipped by you. Nothing changed on {{device}}.",
			{ device },
		),
};

function sharedFailedText(t: DevicesT, error: DeployFailure): string {
	return t(
		"devices:deployShip.result.sharedFailed",
		"Preparing on this computer failed: {{reason}}.",
		{ reason: failureReason(t, error) },
	);
}

/** The block's one sentence: where the device is, or how it ended and what that leaves. */
function singleSentence(
	c: RowContext,
	state: DeployRunState,
	row: DeployRunRow,
	phase: string,
): string {
	const shared = state.shared;
	if (shared?.state === "failed" && shared.error)
		return sharedFailedText(c.t, shared.error);
	const ended = keptText(c, row) ?? doneText(c, row);
	if (ended) return ended;
	const error = row.error;
	return SINGLE_SENTENCE[rowState(row)]({
		t: c.t,
		phase,
		device: c.details[row.target]?.deviceName ?? "",
		reason: error ? failureReason(c.t, error) : "",
		failedWhile: error ? FAILED_WHILE[error.phase](c.t, error) : "",
	});
}

interface SingleChip {
	tone: ChipTone;
	text: string;
	icon: LucideIcon;
	spin?: boolean;
}

function singleChip(t: DevicesT, view: FleetRowState, phase: string) {
	const chips: Partial<Record<FleetRowState, SingleChip>> = {
		done: {
			tone: "good",
			text: t("devices:deployShip.single.done", "Done"),
			icon: CircleCheck,
		},
		failed: {
			tone: "critical",
			text: t("devices:deployShip.single.failed", "Failed"),
			icon: OctagonX,
		},
		rolled_back: {
			tone: "critical",
			text: t("devices:deployShip.single.rolledBackChip", "Rolled back"),
			icon: RotateCcw,
		},
		blocked: {
			tone: "locked",
			text: t("devices:deployShip.single.waiting", "Waiting"),
			icon: Lock,
		},
		skipped: {
			tone: "unknown",
			text: t("devices:deployShip.single.skippedChip", "Skipped"),
			icon: CircleSlash,
		},
	};
	const running: SingleChip = {
		tone: "info",
		text: phase,
		icon: LoaderCircle,
		spin: true,
	};
	return chips[view] ?? running;
}

const SINGLE_ICON: Partial<Record<FleetRowState, LucideIcon>> = {
	done: CircleCheck,
	failed: OctagonX,
	rolled_back: OctagonX,
};

const ENDED_BADLY: readonly FleetRowState[] = ["failed", "rolled_back"];

function singleFoot(t: DevicesT, view: FleetRowState): string {
	if (view === "done")
		return t(
			"devices:deployShip.single.footDone",
			"The service page shows each step in its update history.",
		);
	if (ENDED_BADLY.includes(view))
		return t(
			"devices:deployShip.single.foot",
			"Retrying doesn't resend steps the device already accepted.",
		);
	return t(
		"devices:deployShip.single.footRunning",
		"You can leave this page. Progress continues on the device and stays in Activity.",
	);
}

/** English and machine-oriented on purpose: this text is for support, not for the page. */
function diagnostics(
	row: DeployRunRow,
	detail: DeployTargetDetail | undefined,
) {
	return [
		"Flow-Like deploy diagnostics",
		`Device: ${detail?.deviceName ?? ""} (${row.deviceId})`,
		`Service: ${row.serviceId ?? ""}`,
		`Step: ${row.error?.phase ?? ""} · ${row.error?.code ?? ""}`,
		`Device says: ${row.error?.detail ?? ""}`,
		`Operation: ${detail?.operationId ?? ""}`,
		`Update: ${detail?.rolloutId ?? ""}`,
		`Upload: ${detail?.transferId ?? ""}`,
	].join("\n");
}

function ifItFails(t: DevicesT, detail: DeployTargetDetail | undefined) {
	if (detail?.kind === "new")
		return t("devices:deployShip.single.failsNew", "No service is created.");
	return detail?.strategy === "safe"
		? t(
				"devices:deployShip.single.failsSafe",
				"The device restores the current settings and keeps the service running.",
			)
		: t(
				"devices:deployShip.single.failsQuick",
				"Nothing is applied: the service keeps its current settings.",
			);
}

function progressOf(row: DeployRunRow): number {
	if (row.state === "done") return 100;
	const total = Math.max(1, row.phases.length);
	return Math.round((Math.min(row.phase, total) / total) * 100);
}

function progressTone(view: FleetRowState) {
	if (view === "done") return "good" as const;
	return ENDED_BADLY.includes(view) ? ("critical" as const) : ("info" as const);
}

/** "13:59:47 · took 17 s" once the device finished. */
function finishedText(c: RowContext, row: DeployRunRow): string | undefined {
	const { startedAt, finishedAt } = row;
	if (row.state !== "done" || finishedAt === undefined) return undefined;
	const time = c.time.clock(finishedAt);
	if (startedAt === undefined) return time;
	return c.t(
		"devices:deployShip.single.finishedTook",
		"{{time}} · took {{seconds, number}} s",
		{ time, seconds: Math.max(0, Math.round(finishedAt - startedAt)) },
	);
}

interface SingleRunProps {
	c: RowContext;
	row: DeployRunRow;
	labels: readonly string[];
	shared: FleetSharedStep | undefined;
}

/** One device: its steps as a checklist, the command id and what a failure leaves behind. */
function SingleRun({ c, row, labels, shared }: Readonly<SingleRunProps>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const detail = c.details[row.target];
	const view = rowState(row);
	const finished = finishedText(c, row);
	function copyDiagnostics() {
		void copy(diagnostics(row, detail));
	}
	return (
		<>
			<ProgressBar
				value={progressOf(row)}
				tone={progressTone(view)}
				label={t("deployShip.single.progress", "Deploy progress")}
			/>
			<Checklist
				items={[...sharedItems(shared), ...checkStates(row, labels)]}
				label={t("deployShip.single.steps", "Steps on the device")}
			/>
			<KeyValueList>
				{finished ? (
					<KvRow label={t("deployShip.single.finished", "Finished")}>
						{finished}
					</KvRow>
				) : null}
				{detail?.operationId ? (
					<KvRow label={t("deployShip.single.operation", "Command ID")}>
						<IdRef id={detail.operationId} />
					</KvRow>
				) : null}
				{/* What a failure would leave is said while it can still fail; afterwards the sentence above says what it left. */}
				{view === "done" || ENDED_BADLY.includes(view) ? null : (
					<KvRow label={t("deployShip.single.ifFails", "If it fails")}>
						{ifItFails(t, detail)}
					</KvRow>
				)}
			</KeyValueList>
			{ENDED_BADLY.includes(view) ? (
				<DvButton
					size="sm"
					icon={Copy}
					className="self-start"
					onClick={copyDiagnostics}
				>
					{copied
						? t("deployShip.single.copied", "Copied")
						: t("deployShip.single.copyDiagnostics", "Copy diagnostics")}
				</DvButton>
			) : null}
		</>
	);
}

interface DeployedProps {
	plan: DeployPlan;
	prepared: DeployStepProps["prepared"];
}

/** The plan's summary again once it is on its way; closed until asked for. */
function DeployedDisclosure({ plan, prepared }: Readonly<DeployedProps>) {
	const { t } = useTranslation("devices");
	return (
		<details className="group text-ui" data-deployed="">
			<summary className="flex cursor-pointer list-none items-center gap-1.5 font-medium [&::-webkit-details-marker]:hidden">
				<ChevronRight
					aria-hidden
					className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90"
				/>
				{t("deployShip.single.whatDeployed", "What you deployed")}
			</summary>
			<div className="mt-2 rounded-lg border border-border bg-card px-4 py-3">
				<PlanSummary plan={plan} prepared={prepared} />
			</div>
		</details>
	);
}

/* Foot buttons by outcome. */

interface FootProps {
	/** The run's title ("Deploy Check-in page to 2 devices"). */
	title: string;
	state: DeployRunState;
	result: DeployResult | null;
	run: DeployRunHandle;
	details: DeployRunDetails;
	navigation: Navigation;
	gated: boolean;
	goTo: DeployStepProps["goTo"];
	startOver: DeployStepProps["startOver"];
	eventId: string | undefined;
}

/** Phone (APP §3.17): the primary on its own row first, the others full width under it. */
const PHONE_PRIMARY =
	"@max-[480px]/wfoot:order-1 @max-[480px]/wfoot:basis-full";
const PHONE_OTHER = "@max-[480px]/wfoot:order-2 @max-[480px]/wfoot:flex-1";

function OpenInDevices({
	navigation,
	eventId,
	primary,
}: Readonly<{ navigation: Navigation; eventId?: string; primary: boolean }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const route: DevicesRoute = eventId
		? { screen: "app-devices", by: "event", eventId }
		: { screen: "app-devices", by: "device" };
	return (
		<DvButton
			variant={primary ? "primary" : "default"}
			icon={Server}
			className={primary ? PHONE_PRIMARY : PHONE_OTHER}
			asChild
		>
			<a {...link(route, { scope: navigation.appScope })}>
				{t("deployShip.rollout.openInDevices", "Open in Devices")}
			</a>
		</DvButton>
	);
}

function DoneButtons(props: Readonly<FootProps>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const hostLink = useHostLink();
	const { navigate } = useDevicesRoute();
	const { navigation, state } = props;
	const [only] = state.rows;
	const single = state.rows.length === 1 && only;
	const { appId, eventId, from } = navigation.route;
	const more: DeployRoute = {
		screen: "deploy",
		deviceIds: [],
		...(appId ? { appId } : {}),
		...(eventId ? { eventId } : {}),
		...(from ? { from } : {}),
		mode: "new",
		step: "where",
	};
	return (
		<>
			<DvButton
				icon={Plus}
				className={PHONE_OTHER}
				onClick={() => navigate(more)}
			>
				{t("deployShip.rollout.deployMore", "Deploy to more devices…")}
			</DvButton>
			<DvButton className={PHONE_OTHER} asChild>
				<a {...hostLink(deployExitHref(navigation.route, navigation.scope))}>
					{t("deployShip.rollout.exit", "Exit deploy")}
				</a>
			</DvButton>
			{single ? (
				<DvButton variant="primary" className={PHONE_PRIMARY} asChild>
					<a {...link(serviceRoute(single), { scope: navigation.scope })}>
						{t("deployShip.rollout.openNamed", "Open {{service}}", {
							service: single.serviceId ?? "",
						})}
						<ArrowRight aria-hidden className="size-4" />
					</a>
				</DvButton>
			) : (
				<OpenInDevices
					navigation={navigation}
					eventId={props.eventId}
					primary
				/>
			)}
		</>
	);
}

function PartialButtons(props: Readonly<FootProps>) {
	const { t } = useTranslation("devices");
	const failed = props.state.rows.filter((row) => row.state === "failed");
	return (
		<>
			<OpenInDevices
				navigation={props.navigation}
				eventId={props.eventId}
				primary={failed.length === 0}
			/>
			{failed.map((row, index) => (
				<DvButton
					key={row.target}
					variant={index === 0 ? "primary" : "default"}
					icon={RotateCw}
					className={index === 0 ? PHONE_PRIMARY : PHONE_OTHER}
					disabled={props.gated}
					onClick={() => props.run.retry(row.target)}
				>
					{t("deployShip.rollout.retryNamed", "Retry {{device}}", {
						device: props.details[row.target]?.deviceName ?? "",
					})}
				</DvButton>
			))}
		</>
	);
}

function NoneButtons(props: Readonly<FootProps>) {
	const { t } = useTranslation("devices");
	const { run, state, goTo } = props;
	const failed = state.rows.filter((row) => row.state === "failed");
	const retry = () => {
		if (state.shared?.state === "failed") run.resume();
		for (const row of failed) run.retry(row.target);
	};
	const canRetry = failed.length > 0 || state.shared?.state === "failed";
	// The frame asks first and resets the choices; on its own the step can only open What.
	const startOver = props.startOver ?? (() => goTo("what"));
	return (
		<>
			<DvButton
				variant="danger-ghost"
				icon={RotateCcw}
				className={PHONE_OTHER}
				onClick={startOver}
			>
				{t("deployShip.rollout.startOver", "Start over…")}
			</DvButton>
			<DvButton
				icon={Pencil}
				className={PHONE_OTHER}
				onClick={() => goTo("review")}
			>
				{t("deployShip.rollout.changeAgain", "Change and deploy again")}
			</DvButton>
			{canRetry ? (
				<DvButton
					variant="primary"
					icon={RotateCw}
					className={PHONE_PRIMARY}
					disabled={props.gated}
					onClick={retry}
				>
					{t("deployShip.rollout.retry", "Retry")}
				</DvButton>
			) : null}
		</>
	);
}

/** While it runs: the run's title and how far it is, like its tray item. */
function runningStatus(t: DevicesT, props: FootProps): string {
	const rows = props.state.rows.map(function chipRow(row) {
		return { state: rowState(row) };
	});
	return t("devices:deployShip.foot.running", "{{title}} · {{progress}}", {
		title: props.title,
		progress: fleetRolloutChip(t, rows).text,
	});
}

function failedStatus(t: DevicesT, props: FootProps, result: DeployResult) {
	const total = props.state.rows.length;
	const text =
		result.outcome === "partial"
			? t(
					"devices:deployShip.foot.partial",
					"{{failed, number}} of {{total, number}} failed.",
					{ failed: total - result.done.length, total },
				)
			: t("devices:deployShip.foot.none", "Nothing was deployed.");
	return (
		<span className="inline-flex items-center gap-1.5 text-critical">
			<TriangleAlert aria-hidden className="size-3.5 shrink-0" />
			{text}
		</span>
	);
}

function footStatus(t: DevicesT, time: AreaTime, props: FootProps) {
	const { result } = props;
	if (!result) return runningStatus(t, props);
	if (result.outcome !== "all") return failedStatus(t, props, result);
	return t("devices:deployShip.foot.done", "Deployed at {{time}}", {
		time: time.clock(result.at),
	});
}

function openActivity() {
	useActivityTray.getState().setOpen(true);
}

function RunningButtons() {
	const { t } = useTranslation("devices");
	return (
		<DvButton icon={Activity} onClick={openActivity}>
			{t("deployShip.rollout.follow", "Follow in activity")}
		</DvButton>
	);
}

function RolloutFoot(props: Readonly<FootProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const outcome = props.result?.outcome;
	const status = footStatus(t, time, props);
	return (
		<WizardFoot
			className="static"
			step={8}
			total={8}
			stepLabel={t("deployShip.rollout.title", "Rollout")}
			status={status}
			cancel={
				// The foot hides its note below 720 px; how the run stands stays visible there.
				<output
					data-run-foot-status=""
					className="hidden basis-full text-xs text-muted-foreground @max-[720px]/wfoot:block"
				>
					{status}
				</output>
			}
		>
			{outcome === undefined ? <RunningButtons /> : null}
			{outcome === "all" ? <DoneButtons {...props} /> : null}
			{outcome === "partial" ? <PartialButtons {...props} /> : null}
			{outcome === "none" ? <NoneButtons {...props} /> : null}
		</WizardFoot>
	);
}

/* The step. */

function useNavigation(
	scope: DevicesScope,
	plan: DeployPlan,
	sourceRoute?: DeployRoute,
): Navigation {
	const { route } = useDevicesRoute();
	const appId = plan.app?.id ?? plan.draft.appId ?? undefined;
	return useMemo(() => {
		const deploy: DeployRoute =
			route.screen === "deploy"
				? route
				: (sourceRoute ?? {
						screen: "deploy",
						deviceIds: plan.targets.map((target) => target.deviceId),
						...(appId ? { appId } : {}),
					});
		return {
			scope,
			route: deploy,
			appScope: appId ? { kind: "app", appId } : scope,
		};
	}, [route, sourceRoute, scope, plan.targets, appId]);
}

function useFinished(
	state: DeployRunState,
	onFinished: DeployStepProps["onFinished"],
) {
	const result = useMemo(() => deployRunResult(state), [state]);
	// Once per end: a retry runs again and ends again, even within the same second.
	const reported = useRef(false);
	useEffect(() => {
		if (!result) {
			reported.current = false;
			return;
		}
		if (reported.current) return;
		reported.current = true;
		onFinished(result);
	}, [result, onFinished]);
	return result;
}

function subtitleOf(t: DevicesT, plan: DeployPlan, state: DeployRunState) {
	const order = {
		one: t("devices:deployShip.order.oneLower", "one device at a time"),
		all: t("devices:deployShip.order.allLower", "all at once"),
		first: t(
			"devices:deployShip.order.firstLower",
			"first device, then the rest",
		),
	}[state.order];
	const mode =
		plan.mode === "offline"
			? t("devices:deployShip.mode.offline", "offline copy")
			: t("devices:deployShip.mode.online", "runs online");
	return [plan.app?.name ?? "", mode, order].filter(Boolean).join(" · ");
}

function sharedStep(t: DevicesT, plan: DeployPlan, state: DeployRunState) {
	if (!state.shared) return undefined;
	const offline = state.shared.phase === "prepare";
	return {
		label: offline
			? t(
					"devices:deployShip.shared.prepare",
					"Preparing the copy on this computer",
				)
			: t("devices:deployShip.shared.approve", "Approving definitions"),
		state: state.shared.state,
		source: offline ? ("local" as const) : ("hub" as const),
	};
}

function NotStarted({ goTo }: Readonly<Pick<DeployStepProps, "goTo">>) {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="notloaded"
			title={t(
				"deployShip.rollout.notStarted",
				"Nothing has been deployed yet",
			)}
			text={t(
				"deployShip.rollout.notStartedText",
				"The rollout starts when you deploy on Review.",
			)}
			actions={
				<DvButton size="sm" icon={ClipboardList} onClick={() => goTo("review")}>
					{t("deployShip.rollout.goReview", "Go to Review")}
				</DvButton>
			}
		/>
	);
}

interface BoardProps {
	c: RowContext;
	state: DeployRunState;
	run: DeployRunHandle;
	result: DeployResult | null;
	navigation: Navigation;
	gated: boolean;
}

function TrackedStamp() {
	const { t } = useTranslation("devices");
	return (
		<FreshnessStamp
			source="local"
			age="current"
			text={t("deployShip.rollout.tracked", "tracked on this computer")}
		/>
	);
}

function RunBoard({ c, state, run, result, navigation, gated }: BoardProps) {
	const { t, plan } = c;
	const firstHeld = state.rows.find((row) => row.state === "held")?.target;
	const update = state.rows.every(
		(row) => c.details[row.target]?.kind !== "new",
	);
	const rows = state.rows.map((row) =>
		fleetRow(
			c,
			row,
			<RowActions
				row={row}
				run={run}
				first={row.target === firstHeld}
				gated={gated}
				navigation={navigation}
			/>,
		),
	);
	const context = result ? { ...c, state, result, update } : null;
	const done = { tone: "good" as const, text: singleChip(t, "done", "").text };
	return (
		<FleetRollout
			title={deployRunTitle(t, deployRunTitleRef(plan))}
			subtitle={subtitleOf(t, plan, state)}
			kind={update ? "update" : "deploy"}
			rows={rows}
			stopOnFail={state.stopOnFail}
			heldBy={state.holdBy ? c.details[state.holdBy]?.deviceName : undefined}
			shared={sharedStep(t, plan, state)}
			chip={result?.outcome === "all" ? done : undefined}
			stamp={<TrackedStamp />}
			result={
				context ? (
					<InlineResult
						tone={RESULT_TEXT[context.result.outcome].tone}
						className="max-w-none"
					>
						{RESULT_TEXT[context.result.outcome].text(context)}
					</InlineResult>
				) : undefined
			}
		/>
	);
}

const MONO = { 1: <span className="font-mono" /> };

interface SingleBlockProps extends BoardProps {
	row: DeployRunRow;
}

function SingleBlock(props: Readonly<SingleBlockProps>) {
	const { c, state, row } = props;
	const { t } = useTranslation("devices");
	const labels = phaseLabels(c, row);
	const phase = labels[Math.min(row.phase, labels.length - 1)] ?? "";
	const view = rowState(row);
	const chip = singleChip(c.t, view, phase);
	const bad = ENDED_BADLY.includes(view) || state.shared?.state === "failed";
	const device = c.details[row.target]?.deviceName ?? "";
	const title = (
		<Trans
			t={t}
			i18nKey="deployShip.single.title"
			defaults="Deploy to <1>{{device}}</1>"
			values={{ device }}
			components={MONO}
		/>
	);
	const status = (
		<StatusChip tone={chip.tone} icon={chip.icon} spin={chip.spin}>
			{chip.text}
		</StatusChip>
	);
	return (
		<Block
			id="dp-run"
			icon={SINGLE_ICON[view] ?? Rocket}
			title={title}
			summary={status}
			stamp={<TrackedStamp />}
			danger={bad}
			foot={singleFoot(c.t, view)}
		>
			<p
				role={bad ? "alert" : "status"}
				className="max-w-[72ch] text-ui font-semibold"
				data-run-sentence=""
			>
				{singleSentence(c, state, row, phase)}
			</p>
			{view === "blocked" ? (
				<div className="flex flex-wrap gap-2">
					<RowActions
						row={row}
						run={props.run}
						first={false}
						gated={props.gated}
						navigation={props.navigation}
					/>
				</div>
			) : null}
			<SingleRun
				c={c}
				row={row}
				labels={labels}
				shared={sharedStep(c.t, c.plan, state)}
			/>
		</Block>
	);
}

function useRollout(props: Readonly<DeployStepProps>) {
	const { plan, draft, scope } = props;
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const deploymentId = draft.deploymentId;
	// biome-ignore lint/correctness/useExhaustiveDependencies: the run of one deploy is looked up once per deploy id
	const resumeId = useMemo(
		() => recallDeployRun(workspace, plan),
		[workspace, deploymentId],
	);
	const title = useMemo(() => deployRunTitleRef(plan), [plan]);
	const run = useDeployRun(plan, {
		title,
		oneAtATime: draft.order === "one",
		stopOnFail: draft.stopOnFail,
		resumeId,
	});
	const details = useDeployRunDetails(deploymentId);
	useDeployRunWatch(deploymentId);
	useHandedBundle(deploymentId, props.prepared);
	const adopted = useDeployRunAdopted(deploymentId);
	const navigation = useNavigation(scope, plan, props.route);
	const result = useFinished(run.state, props.onFinished);
	const bundle = props.prepared ?? deployRunExtras(deploymentId).prepared;
	const files = bundle?.artifact.descriptor.file_count ?? 0;
	const c: RowContext = { t, time, plan, files, details };
	// After a reload the draft has no secret values: a retry would deploy without them.
	const gated = adopted && !props.check.ok;
	return { run, details, navigation, result, c, gated };
}

function firstDeviceName(c: RowContext, state: DeployRunState): string {
	const [row] = state.rows;
	const detail = row ? c.details[row.target] : undefined;
	return detail ? detail.deviceName : "";
}

function doneLede(c: RowContext, state: DeployRunState): string {
	const { t } = c;
	const total = state.rows.length;
	if (total === 1)
		return t("devices:deployShip.rollout.ledeOne", "Done on {{device}}.", {
			device: firstDeviceName(c, state),
		});
	if (total === 2)
		return t("devices:deployShip.rollout.ledeBoth", "Done on both devices.");
	return t(
		"devices:deployShip.rollout.ledeAll",
		"Done on all {{count, number}} devices.",
		{ count: total },
	);
}

function partialLede(c: ResultContext): string {
	const counts = { done: c.result.done.length, total: c.state.rows.length };
	return c.update
		? c.t(
				"devices:deployShip.rollout.ledePartialUpdate",
				"Updated {{done, number}} of {{total, number}} services.",
				counts,
			)
		: c.t(
				"devices:deployShip.rollout.ledePartial",
				"Deployed to {{done, number}} of {{total, number}} devices.",
				counts,
			);
}

/** The step's lede (APP §3.13) once the run ended. */
function finishedLede(c: ResultContext): string {
	if (c.result.outcome === "none")
		return c.t("devices:deployShip.rollout.ledeNone", "Nothing was deployed.");
	if (c.result.outcome === "partial") return partialLede(c);
	return doneLede(c, c.state);
}

function runningLede(t: DevicesT): string {
	return t(
		"devices:deployShip.rollout.ledeRunning",
		"Each device runs its own steps. You can leave this page; Activity keeps tracking it.",
	);
}

/** What runs where now, and what a retry of each failed device reuses. */
function AfterNotes({
	c,
	state,
	result,
}: Readonly<{ c: RowContext; state: DeployRunState; result: DeployResult }>) {
	const { t } = c;
	const where = whereText({ ...c, state, result, update: false });
	const failed = state.rows.filter(
		(row) => row.state === "failed" && !row.error?.rolledBack,
	);
	return (
		<div className="flex flex-col gap-2 text-ui" data-run-notes="">
			{where ? <p className="text-ui">{where}</p> : null}
			{failed.map((row) => {
				const detail = c.details[row.target];
				const device = detail?.deviceName ?? "";
				return (
					<p key={row.target} className="text-ui">
						{detail?.kept
							? t(
									"devices:deployShip.rollout.retryReuses",
									"Retrying {{device}} resumes from the step that failed and reuses its access on the hub.",
									{ device },
								)
							: t(
									"devices:deployShip.rollout.retryResumes",
									"Retrying {{device}} resumes from the step that failed.",
									{ device },
								)}
					</p>
				);
			})}
		</div>
	);
}

function RolloutBody({
	props,
	model,
}: Readonly<{
	props: DeployStepProps;
	model: ReturnType<typeof useRollout>;
}>) {
	const { run, details, navigation, result, c, gated } = model;
	const { state } = run;
	const { draft, plan } = props;
	const what = deployWhat(plan);
	const update = state.rows.every((row) => details[row.target]?.kind !== "new");
	const ended = result ? { ...c, state, result, update } : null;
	const lead = ended ? finishedLead(ended, what) : runningLead(c, state, what);
	const lede = ended ? finishedLede(ended) : runningLede(c.t);
	// Above the stepper when the frame offers the place its planning headline has on the other steps.
	const { anchor, slot } = useWizardSlot("headline");
	const headline = <Headline lead={lead.lead} rest={lead.rest} />;
	const board = { c, state, run, result, navigation, gated };
	const [only] = state.rows.length === 1 ? state.rows : [];
	return (
		<div className="flex min-w-0 flex-col gap-4">
			<span ref={anchor} hidden />
			{slot ? createPortal(headline, slot) : headline}
			<StepHeader lede={lede} />
			{only ? <SingleBlock {...board} row={only} /> : <RunBoard {...board} />}
			{result && !only ? (
				<AfterNotes c={c} state={state} result={result} />
			) : null}
			<TokenBlock
				deploymentId={draft.deploymentId}
				state={state}
				details={details}
			/>
			{only ? (
				<DeployedDisclosure plan={plan} prepared={props.prepared} />
			) : null}
			<div className="sticky bottom-2 z-5 flex flex-col gap-2">
				{props.summaryBar}
				<RolloutFoot
					title={deployRunTitle(c.t, deployRunTitleRef(plan))}
					state={state}
					result={result}
					run={run}
					details={details}
					navigation={navigation}
					gated={gated}
					goTo={props.goTo}
					startOver={props.startOver}
					eventId={draft.scope === "event" ? draft.events[0] : undefined}
				/>
			</div>
		</div>
	);
}

function StepHeader({ lede }: Readonly<{ lede?: string }>) {
	const { t } = useTranslation("devices");
	return (
		<WizardStepHeader
			step={8}
			total={8}
			title={t("deployShip.rollout.title", "Rollout")}
			lede={lede}
		/>
	);
}

export function RolloutStep(props: Readonly<DeployStepProps>) {
	const model = useRollout(props);
	if (model.run.state.status === "idle")
		return (
			<div className="flex flex-col gap-4">
				<StepHeader />
				<NotStarted goTo={props.goTo} />
			</div>
		);
	return <RolloutBody props={props} model={model} />;
}
