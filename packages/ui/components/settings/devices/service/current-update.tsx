"use client";

import { useTranslation } from "@flow-like/locales";
import { Activity, Play, Rocket, ScrollText } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	agentSupports,
	readRolloutHistory,
} from "../../../../lib/device-management/agent-reads";
import {
	DeploymentRolloutEndedError,
	type DeploymentRolloutStatus,
	cancelDeploymentRollout,
	readDeploymentRollout,
	readExistingDeployment,
	waitForDeploymentRollout,
} from "../../../../lib/device-management/deployment";
import type {
	DevicesRoute,
	GateResult,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { enumExplain, enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import {
	type RolloutPhase,
	RolloutProgress,
	type RolloutTimes,
} from "../primitives/rollout-progress";
import { useRouteLink } from "../routing/use-devices-route";
import { useActivityTray } from "../shell/activity-tray";
import { stampOf } from "../shell/attention-popover";
import {
	DeviceRejectedError,
	deviceCall,
	serviceGateExtra,
	useAttentionState,
	useDeviceAction,
	useDeviceWorkspace,
	useGates,
	useInlineResults,
} from "../workspace";

const DAY_S = 86_400;
const FOLLOW_MS = 1_000;
const HISTORY_PAGE = 8;
const FOLLOWED = new Set<DeploymentRolloutStatus["state"]>([
	"validating",
	"activating",
	"rolling_back",
]);
const SERVICE_STOPPED = new Set(["rollback_timeout", "rollback_failed"]);

type RolloutScope = Pick<
	DeploymentRolloutStatus,
	"rollout_id" | "placement_id" | "project_id"
>;

/** The display phase of a device rollout; a cancelled one is only history. */
export function rolloutPhase(
	rollout: Pick<DeploymentRolloutStatus, "state" | "failure_code">,
): RolloutPhase | null {
	if (rollout.state === "cancelled") return null;
	if (rollout.state !== "failed") return rollout.state;
	return SERVICE_STOPPED.has(rollout.failure_code ?? "")
		? "failed_stopped"
		: "not_applied";
}

export function rolloutEndedAt(rollout: DeploymentRolloutStatus) {
	return rollout.updated_at ?? rollout.created_at;
}

/** A running update always; one that ended for a day; the failed update that left the service stopped while it stays stopped. */
function staysCurrent(
	service: ServiceView,
	rollout: DeploymentRolloutStatus,
	phase: RolloutPhase,
	nowS: number,
) {
	if (phase === "staged" || FOLLOWED.has(rollout.state)) return true;
	if (phase === "failed_stopped" && service.conv === "failed_stopped")
		return true;
	const endedAt = rolloutEndedAt(rollout);
	return endedAt !== undefined && nowS - endedAt < DAY_S;
}

/** The rollout the page treats as the current update; older results live in the update history only. */
export function currentUpdateOf(
	service: ServiceView | undefined,
	nowS: number,
): DeploymentRolloutStatus | undefined {
	const rollout = service?.rollout;
	const phase = rollout ? rolloutPhase(rollout) : null;
	if (!service || !rollout || !phase) return undefined;
	return staysCurrent(service, rollout, phase, nowS) ? rollout : undefined;
}

/** The settings versions of a rollout: "settings v11" → "v12". */
export function rolloutSettings(
	t: DevicesT,
	rollout: DeploymentRolloutStatus,
	service: ServiceView,
) {
	const from = rollout.base_revision ?? service.settings.applied ?? 0;
	const to = rollout.active_revision ?? service.settings.latest;
	return {
		from,
		to,
		fromLabel: t("devices:service.update.settings", "settings v{{version}}", {
			version: from,
		}),
		toLabel: t("devices:service.update.version", "v{{version}}", {
			version: to,
		}),
	};
}

/** Why an update ended the way it did, in the device's failure vocabulary. */
export function rolloutOutcome(
	t: DevicesT,
	rollout: DeploymentRolloutStatus,
): string | undefined {
	const code = rollout.failure_code;
	if (!code) return undefined;
	const label = enumLabel(t, "failureCode", code as never);
	const explain = enumExplain(t, "failureCode", code as never);
	return explain ? `${label}. ${explain}` : `${label}.`;
}

const followers = new WeakMap<DeviceWorkspace, Set<string>>();

function followersOf(workspace: DeviceWorkspace) {
	let set = followers.get(workspace);
	if (!set) {
		set = new Set();
		followers.set(workspace, set);
	}
	return set;
}

function sameRollout(
	a: DeploymentRolloutStatus,
	b: DeploymentRolloutStatus | undefined,
) {
	return b !== undefined && JSON.stringify(a) === JSON.stringify(b);
}

/** Keeps the device's other rollouts; the attention engine reads them from the facts store. */
function recordRollouts(
	workspace: DeviceWorkspace,
	deviceId: string,
	rows: readonly DeploymentRolloutStatus[],
) {
	if (!rows.length || workspace.keys.snapshot(deviceId).state !== "unlocked")
		return;
	const known = workspace.facts.get(deviceId)?.rollouts ?? [];
	const byId = new Map(known.map((row) => [row.rollout_id, row]));
	if (rows.every((row) => sameRollout(row, byId.get(row.rollout_id)))) return;
	const ids = new Set(rows.map((row) => row.rollout_id));
	workspace.facts.record(deviceId, {
		rollouts: [...known.filter((row) => !ids.has(row.rollout_id)), ...rows],
	});
}

/** Polls one rollout until it ends; the status it reads lands in the facts store. */
async function followRollout(
	workspace: DeviceWorkspace,
	deviceId: string,
	scope: RolloutScope,
): Promise<"done" | "failed"> {
	const following = followersOf(workspace);
	following.add(scope.rollout_id);
	try {
		await waitForDeploymentRollout(
			deviceCall(workspace, deviceId, "poll"),
			scope,
			undefined,
			(status) => recordRollouts(workspace, deviceId, [status]),
		);
		return "done";
	} catch (error) {
		if (!(error instanceof DeploymentRolloutEndedError)) throw error;
		recordRollouts(workspace, deviceId, [error.status]);
		return "failed";
	} finally {
		following.delete(scope.rollout_id);
	}
}

function startedAt(rollout: DeploymentRolloutStatus) {
	return rollout.created_at ?? rollout.updated_at ?? 0;
}

function newestFirst(a: DeploymentRolloutStatus, b: DeploymentRolloutStatus) {
	return startedAt(b) - startedAt(a);
}

export interface ServiceRollouts {
	/** This service's rollouts as far as known, newest first. */
	rollouts: DeploymentRolloutStatus[];
	/** `full`: the device's own list. `latest`: an older agent, only what this computer could read. */
	coverage: "full" | "latest" | "unread";
	/** Older rollouts exist on the device. */
	more: boolean;
	loading: boolean;
	/** The last read failed; the rows are from the read before. */
	failed: boolean;
	/** Unix seconds of the last good read. */
	readAt?: number;
	following: boolean;
	refresh(): void;
	loadOlder(): void;
}

interface ReadTarget {
	workspace: DeviceWorkspace;
	deviceId: string;
	serviceId: string;
	projectId: string;
	/** The viewer may read the service's settings (older agents keep the rollout there). */
	mayReadConfig: boolean;
}

interface ReadResult {
	coverage: "full" | "latest";
	next: string | null;
	readAt: number;
}

/** The update this computer knows about: one already read, else one it started (tray handle). */
function knownScope(target: ReadTarget): RolloutScope | undefined {
	const { workspace, deviceId, serviceId } = target;
	const known = (workspace.facts.get(deviceId)?.rollouts ?? [])
		.filter((row) => row.placement_id === serviceId)
		.sort(newestFirst)[0];
	if (known) return known;
	const handle = workspace.activity
		.list()
		.flatMap((item) =>
			item.resume?.type === "rollout" && item.resume.placementId === serviceId
				? [item.resume]
				: [],
		)[0];
	if (!handle) return undefined;
	return {
		rollout_id: handle.rolloutId,
		placement_id: serviceId,
		project_id: handle.projectId,
	};
}

/** Older agents can't list updates: re-read the known one, else the one its settings name. */
async function readLatestKnown(target: ReadTarget) {
	const { workspace, deviceId, serviceId, projectId } = target;
	const call = deviceCall(workspace, deviceId, "poll");
	const scope = knownScope(target);
	if (scope) return [await readDeploymentRollout(call, scope)];
	if (!target.mayReadConfig) return [];
	const existing = await readExistingDeployment(call, serviceId, projectId);
	return existing.rollout ? [existing.rollout] : [];
}

async function readRollouts(
	target: ReadTarget,
	before?: string,
): Promise<ReadResult & { rows: DeploymentRolloutStatus[] }> {
	const { workspace, deviceId, serviceId } = target;
	const readAt = Math.floor(workspace.clock.now() / 1000);
	const history = await readRolloutHistory(
		deviceCall(workspace, deviceId, "poll"),
		workspace.live.inspection(deviceId)?.value.features,
		{
			placementId: serviceId,
			limit: HISTORY_PAGE,
			...(before ? { before } : {}),
		},
	);
	if (history.kind === "ok")
		return {
			coverage: "full",
			next: history.data.next,
			readAt,
			rows: history.data.rollouts,
		};
	return {
		coverage: "latest",
		next: null,
		readAt,
		rows: await readLatestKnown(target),
	};
}

interface ReadState extends Omit<ReadResult, "coverage"> {
	coverage: ServiceRollouts["coverage"];
	failed: boolean;
}

const NEVER_READ: ReadState = {
	coverage: "unread",
	next: null,
	readAt: 0,
	failed: true,
};

/** Reads the updates when `readKey` changes and keeps the last good rows when a read fails. */
function useRolloutRead(target: ReadTarget | undefined, readKey: string) {
	const [state, setState] = useState<ReadState>();
	const [loading, setLoading] = useState(false);
	const [olderFrom, setOlderFrom] = useState<string>();

	// biome-ignore lint/correctness/useExhaustiveDependencies: `readKey` stands for every input of the read
	useEffect(() => {
		if (!readKey || !target) return;
		let alive = true;
		setLoading(true);
		readRollouts(target)
			.then(({ rows, ...result }) => {
				if (!alive) return;
				recordRollouts(target.workspace, target.deviceId, rows);
				setState({ ...result, failed: false });
			})
			.catch(() => {
				if (alive)
					setState((previous) => ({
						...(previous ?? NEVER_READ),
						failed: true,
					}));
			})
			.finally(() => {
				if (alive) setLoading(false);
			});
		return () => {
			alive = false;
		};
	}, [readKey]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `readKey` stands for the target
	useEffect(() => {
		if (!olderFrom || !target) return;
		let alive = true;
		readRollouts(target, olderFrom)
			.then(({ rows, next }) => {
				if (!alive) return;
				recordRollouts(target.workspace, target.deviceId, rows);
				setState((previous) => (previous ? { ...previous, next } : previous));
			})
			.catch(() => undefined)
			.finally(() => {
				if (alive) setOlderFrom(undefined);
			});
		return () => {
			alive = false;
		};
	}, [olderFrom, readKey]);

	const next = state?.next ?? null;
	const loadOlder = useCallback(() => {
		if (next) setOlderFrom(next);
	}, [next]);
	return { state, loading: loading || !!olderFrom, loadOlder };
}

/** Once a second while an update runs and nothing else follows it; `true` while the last read failed. */
function useRolloutFollow(
	workspace: DeviceWorkspace,
	deviceId: string,
	scope: RolloutScope | undefined,
) {
	const [failed, setFailed] = useState(false);
	const rolloutId = scope?.rollout_id;
	const placementId = scope?.placement_id;
	const projectId = scope?.project_id;
	useEffect(() => {
		if (!rolloutId || !placementId || !projectId) return;
		let alive = true;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const call = deviceCall(workspace, deviceId, "poll");
		const read = async () => {
			const status = await readDeploymentRollout(call, {
				rollout_id: rolloutId,
				placement_id: placementId,
				project_id: projectId,
			});
			if (alive) recordRollouts(workspace, deviceId, [status]);
		};
		const poll = async () => {
			if (!followersOf(workspace).has(rolloutId))
				await read().then(
					() => alive && setFailed(false),
					() => alive && setFailed(true),
				);
			if (alive) timer = setTimeout(poll, FOLLOW_MS);
		};
		timer = setTimeout(poll, FOLLOW_MS);
		return () => {
			alive = false;
			clearTimeout(timer);
			setFailed(false);
		};
	}, [workspace, deviceId, rolloutId, placementId, projectId]);
	return failed;
}

const settingsKey = (service: ServiceView | undefined) =>
	service ? `${service.settings.latest}/${service.settings.applied}` : "";

/**
 * The rollouts of one service: read from the device when the page opens (the
 * update history on current agents, the latest known update on older ones),
 * followed once a second while one runs, and recorded for the attention engine.
 */
export function useServiceRollouts(
	deviceId: string,
	service: ServiceView | undefined,
): ServiceRollouts {
	const subject = useRolloutSubject(deviceId, service);
	const { workspace, liveOpen, target } = subject;
	const followedId = followedIdOf(liveOpen, service?.rollout);
	const [tick, setTick] = useState(0);
	const refresh = useCallback(() => setTick((value) => value + 1), []);
	// An older agent keeps the update in the service's settings, which only some viewers may read.
	const configAccess = subject.supported || !target?.mayReadConfig ? "" : "c";
	const readKey =
		liveOpen && target
			? [
					deviceId,
					target.serviceId,
					subject.supported,
					configAccess,
					settingsKey(service),
					followedId,
					tick,
				].join("|")
			: "";
	const { state, loading, loadOlder } = useRolloutRead(target, readKey);
	const scope = useMemo(
		() => followScope(followedId, target),
		[followedId, target],
	);
	const followFailed = useRolloutFollow(workspace, deviceId, scope);
	const { rollouts } = subject;

	return useMemo(
		() => ({
			...readSummary(state),
			rollouts,
			loading,
			failed: (state?.failed ?? false) || followFailed,
			following: followedId !== "",
			refresh,
			loadOlder,
		}),
		[rollouts, state, loading, followFailed, followedId, refresh, loadOlder],
	);
}

const isOpen = (kind: string | undefined) =>
	kind === "live" || kind === "renewing";

/** The id of an update that runs now and can be followed over the live connection, else "". */
function followedIdOf(
	liveOpen: boolean,
	rollout: DeploymentRolloutStatus | undefined,
) {
	if (!liveOpen || !rollout) return "";
	return FOLLOWED.has(rollout.state) ? rollout.rollout_id : "";
}

function followScope(
	rolloutId: string,
	target: ReadTarget | undefined,
): RolloutScope | undefined {
	if (!rolloutId || !target) return undefined;
	return {
		rollout_id: rolloutId,
		placement_id: target.serviceId,
		project_id: target.projectId,
	};
}

function readSummary(state: ReadState | undefined) {
	if (!state) return { coverage: "unread" as const, more: false };
	return {
		coverage: state.coverage,
		more: state.next !== null,
		...(state.readAt ? { readAt: state.readAt } : {}),
	};
}

/** What is known about the service's updates before any read: the live state, the agent's support and the recorded rows. */
function useRolloutSubject(deviceId: string, service: ServiceView | undefined) {
	const workspace = useDeviceWorkspace();
	const { input } = useAttentionState();
	const serviceId = service?.serviceId;
	const projectId = service?.projectId;
	const liveInput = input.live[deviceId];
	const gates = useGates(["update_service"], deviceId, {
		placementId: serviceId,
		...(projectId ? { projectId } : {}),
	});
	const configGate = gates.update_service;
	const mayReadConfig = configGate.ok || configGate.kind === "busy";
	const known = liveInput?.rollouts;
	const rollouts = useMemo(
		() =>
			(known ?? [])
				.filter((row) => row.placement_id === serviceId)
				.sort(newestFirst),
		[known, serviceId],
	);
	const target = useMemo<ReadTarget | undefined>(
		() =>
			serviceId && projectId
				? { workspace, deviceId, serviceId, projectId, mayReadConfig }
				: undefined,
		[workspace, deviceId, serviceId, projectId, mayReadConfig],
	);
	return {
		workspace,
		liveOpen: isOpen(liveInput?.state.kind),
		supported: agentSupports(
			liveInput?.inspection?.value.features,
			"rollout_history",
		),
		rollouts,
		target,
	};
}

export function updateResultKey(deviceId: string, serviceId: string) {
	return `service-update:${deviceId}/${serviceId}`;
}

interface UpdateContext {
	deviceId: string;
	deviceName: string;
	service: ServiceView;
	rollout: DeploymentRolloutStatus;
}

/** Activate and Discard of one update, through the action layer (gate, confirm, tray, inline result). */
function useUpdateActions({
	deviceId,
	deviceName,
	service,
	rollout,
}: UpdateContext) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const { serviceId, projectId } = service;
	const resultKey = updateResultKey(deviceId, serviceId);
	const target = useMemo(
		() => ({
			placementId: serviceId,
			projectId,
			labels: { service: serviceId },
			extra: serviceGateExtra(service),
		}),
		[serviceId, projectId, service],
	);
	const gates = useGates(
		["activate_staged", "discard_staged"],
		deviceId,
		target,
	);
	const scope: RolloutScope = {
		rollout_id: rollout.rollout_id,
		placement_id: serviceId,
		project_id: projectId,
	};
	const gateOf = (gate: GateResult): Gate | null =>
		gate.ok
			? null
			: { kind: gate.kind, reason: gateCopy(t, gate, time).inline };

	const activate = () =>
		actions.run({
			action: "activate_staged",
			deviceId,
			target,
			label: t(
				"service.update.activateLabel",
				"Activate the update of {{service}}",
				{ service: serviceId },
			),
			resultKey,
			call: async (context) => {
				await context.request({
					type: "activate_rollout",
					rollout_id: scope.rollout_id,
				});
				const status = await readDeploymentRollout(context.call, scope).catch(
					(): DeploymentRolloutStatus => ({ ...rollout, state: "validating" }),
				);
				recordRollouts(workspace, deviceId, [status]);
				return status;
			},
			activity: {
				kind: "safe_update",
				deviceName,
				serviceId,
				projectId,
				href: { screen: "service", deviceId, serviceId, tab: "status" },
				resume: () => ({
					type: "rollout",
					rolloutId: scope.rollout_id,
					placementId: serviceId,
					projectId,
				}),
				settle: () => followRollout(workspace, deviceId, scope),
			},
		});

	const discard = () =>
		actions.run({
			action: "discard_staged",
			deviceId,
			target,
			label: t(
				"service.update.discardLabel",
				"Discard the update of {{service}}",
				{ service: serviceId },
			),
			consequence: {
				what: t(
					"service.update.discardWhat",
					"The staged version is discarded. The current version keeps running.",
				),
				who: t(
					"service.update.discardWho",
					"Nobody: {{service}} keeps answering with {{from}}.",
					{
						service: serviceId,
						from: rolloutSettings(t, rollout, service).fromLabel,
					},
				),
				when: t("service.update.discardWhen", "Immediately."),
				undo: {
					reversible: true,
					text: t(
						"service.update.discardUndo",
						"Stage it again from Configuration.",
					),
				},
			},
			strength: "none",
			confirm: {
				sub: t("service.update.on", "on {{device}}", { device: deviceName }),
				tone: "danger",
			},
			resultKey,
			call: async (context) => {
				const status = await cancelDeploymentRollout(context.call, scope);
				recordRollouts(workspace, deviceId, [status]);
				if (status.state !== "cancelled")
					throw new DeviceRejectedError({
						code: "busy",
						error: t(
							"service.update.discardTooLate",
							"Switching over had already started, so the update can't be discarded any more.",
						),
						retryable: false,
					});
				return status;
			},
		});

	return {
		busy: actions.pending(resultKey),
		activate,
		discard,
		activateGate: gateOf(gates.activate_staged),
		discardGate: gateOf(gates.discard_staged),
	};
}

function FollowButton() {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			size="sm"
			variant="ghost"
			icon={Activity}
			onClick={() => useActivityTray.getState().setOpen(true)}
		>
			{t("service.update.follow", "Follow in activity")}
		</DvButton>
	);
}

/** Staged or checking: the update can still be discarded; a staged one can be activated. */
function PendingControls({
	context,
	staged,
}: Readonly<{ context: UpdateContext; staged: boolean }>) {
	const { t } = useTranslation("devices");
	const update = useUpdateActions(context);
	return (
		<>
			{staged ? (
				<GatedAction gate={update.activateGate}>
					<DvButton
						size="sm"
						icon={Play}
						busy={update.busy}
						onClick={update.activate}
					>
						{t("service.update.activate", "Activate")}
					</DvButton>
				</GatedAction>
			) : null}
			<GatedAction gate={update.discardGate}>
				<DvButton
					size="sm"
					variant="danger-ghost"
					busy={update.busy}
					onClick={update.discard}
				>
					{t("service.update.discard", "Discard update…")}
				</DvButton>
			</GatedAction>
			<FollowButton />
		</>
	);
}

function SwitchingControls() {
	const { t } = useTranslation("devices");
	return (
		<>
			<GatedAction
				gate={{
					kind: "busy",
					reason: t(
						"service.update.discardStarted",
						"Can't discard once switching has started.",
					),
				}}
			>
				<DvButton size="sm">
					{t("service.update.discard", "Discard update…")}
				</DvButton>
			</GatedAction>
			<FollowButton />
		</>
	);
}

/** After a failed update: the crash is in the logs, the next try starts in the deploy wizard. */
function FailedControls({
	deviceId,
	serviceId,
}: Readonly<{ deviceId: string; serviceId: string }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const logsRoute: DevicesRoute = {
		screen: "service",
		deviceId,
		serviceId,
		tab: "activity",
		stream: "errors",
	};
	const deployRoute: DevicesRoute = {
		screen: "deploy",
		deviceIds: [deviceId],
		serviceId,
	};
	return (
		<>
			<DvButton asChild size="sm" icon={ScrollText}>
				<a {...link(logsRoute)}>
					{t("service.update.viewLogs", "View logs (errors only)")}
				</a>
			</DvButton>
			<DvButton asChild size="sm" variant="ghost" icon={Rocket}>
				<a
					{...link(deployRoute)}
					title={t(
						"service.update.updateTitle",
						"Change the app version, events or settings in the deploy wizard",
					)}
				>
					{t("service.update.update", "Update…")}
				</a>
			</DvButton>
		</>
	);
}

/** An update that finished well, or was never switched on, leaves nothing to do here. */
const NO_CONTROLS = new Set<RolloutPhase>(["healthy", "not_applied"]);

function UpdateControls({
	context,
	phase,
}: Readonly<{ context: UpdateContext; phase: RolloutPhase }>) {
	switch (phase) {
		case "staged":
		case "validating":
			return <PendingControls context={context} staged={phase === "staged"} />;
		case "activating":
		case "rolling_back":
			return <SwitchingControls />;
		default:
			return (
				<FailedControls
					deviceId={context.deviceId}
					serviceId={context.service.serviceId}
				/>
			);
	}
}

/** The device reports when an update was staged, when it last changed and when it must be done; steps in between have no time of their own. */
function rolloutTimes(
	rollout: DeploymentRolloutStatus,
	phase: RolloutPhase,
): RolloutTimes {
	const times: RolloutTimes = {};
	if (rollout.created_at !== undefined) times.staged = rollout.created_at;
	const changedAt = rollout.updated_at;
	if (changedAt === undefined || phase === "staged") return times;
	if (!FOLLOWED.has(rollout.state)) times.done = changedAt;
	else if (rollout.state === "activating" && !rollout.stable_since)
		times.validated = changedAt;
	return times;
}

function UpdateStamp({
	service,
	rollouts,
}: Readonly<{ service: ServiceView; rollouts: ServiceRollouts }>) {
	const { t } = useTranslation("devices");
	if (!rollouts.following)
		return <FreshnessStamp {...stampOf(service.freshness)} />;
	if (!rollouts.failed)
		return (
			<FreshnessStamp
				source="live"
				age="live"
				text={t("service.update.following", "following")}
			/>
		);
	return (
		<FreshnessStamp
			source="live"
			age="error"
			{...(rollouts.readAt ? { error: { dataFrom: rollouts.readAt } } : {})}
		/>
	);
}

/** SPEC §5.3 Status › Current update: the safe update's timeline with Activate, Discard and Follow. */
export function CurrentUpdate({
	deviceId,
	deviceName,
	service,
	rollout,
	rollouts,
}: Readonly<UpdateContext & { rollouts: ServiceRollouts }>) {
	const { t } = useTranslation("devices");
	const phase = rolloutPhase(rollout) ?? "not_applied";
	const versions = rolloutSettings(t, rollout, service);
	const { previous_replicas: previous, candidate_replicas: candidate } =
		rollout;
	return (
		<RolloutProgress
			phase={phase}
			from={versions.fromLabel}
			to={versions.toLabel}
			times={rolloutTimes(rollout, phase)}
			{...(rollout.deadline_at ? { deadlineAt: rollout.deadline_at } : {})}
			{...(rollout.stable_since ? { stableSince: rollout.stable_since } : {})}
			{...(rollout.stabilization_seconds
				? { stabilizeSec: rollout.stabilization_seconds }
				: {})}
			{...(previous !== undefined && candidate !== undefined
				? { instances: { previous, candidate } }
				: {})}
			waitingFor={t("service.update.instanceZero", "instance #0")}
			outcome={rolloutOutcome(t, rollout)}
			stamp={<UpdateStamp service={service} rollouts={rollouts} />}
			actions={
				NO_CONTROLS.has(phase) ? undefined : (
					<UpdateControls
						context={{ deviceId, deviceName, service, rollout }}
						phase={phase}
					/>
				)
			}
		/>
	);
}

/** What Activate and Discard did; it outlives the block when the update ends (R9). */
export function UpdateResults({
	deviceId,
	serviceId,
}: Readonly<{ deviceId: string; serviceId: string }>) {
	const results = useInlineResults(updateResultKey(deviceId, serviceId));
	return results.map((result) => (
		<InlineResult key={result.id} tone={result.tone} onDismiss={result.dismiss}>
			{result.text}
		</InlineResult>
	));
}
