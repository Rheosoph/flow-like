"use client";

import {
	useEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import { readArtifactUsage } from "../../../../lib/device-management/agent-reads";
import {
	type ArtifactProgress,
	type ArtifactTransferStatus,
	ArtifactUploadError,
	type PendingArtifactTransfer,
	type PreparedProjectArtifact,
	forgetArtifactTransfer,
	pendingArtifactTransfers,
	readArtifactTransfer,
	rememberArtifactTransfer,
	uploadProjectArtifact,
} from "../../../../lib/device-management/artifacts";
import {
	type DeploymentCatalog,
	type DeploymentEvent,
	type DeploymentPlan,
	DeploymentPublicationFailedError,
	DeploymentRejectedError,
	DeploymentReviewRequiredError,
	DeploymentRolloutEndedError,
	type DeploymentRolloutStatus,
	type DeploymentVariable,
	type InstalledProject,
	type PlacementConfiguration,
	StaleDeploymentRevisionError,
	assertOfflineQueuesDrained,
	createDeploymentPlan,
	discoverOfflineEvents,
	discoverOfflineVariables,
	discoverPreviousOfflineVariables,
	discoverPreviousOnlineVariables,
	executeDeploymentPlan,
	readExistingDeployment,
	removesOfflineBuffering,
} from "../../../../lib/device-management/deployment";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import {
	type DeployFailure,
	type DeployPhase,
	type DeployPlan,
	type DeployTargetDraft,
	type PlanTarget,
	type PlanTargetService,
	type WireFacts,
	approvalRequest,
	wirePlan,
} from "../../../../lib/device-management/model/deploy-plan";
import {
	type DeployRunEvent,
	type DeployRunRow,
	type DeployRunState,
	type SharedPhase,
	activeTargets,
	createDeployRun,
	reduceDeployRun,
} from "../../../../lib/device-management/model/deploy-run";
import type {
	CopyRef,
	DeployRoute,
} from "../../../../lib/device-management/model/types";
import { prepareOnlineDependencies } from "../../../../lib/device-management/online-dependencies";
import { prepareOnlineMetadata } from "../../../../lib/device-management/online-metadata";
import {
	desktopExportCommands,
	prepareDesktopProject,
} from "../../../../lib/device-management/project-export";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import type {
	ActivityItem,
	ActivityKind,
	ActivityRun,
	ActivityTracker,
	DeviceWorkspace,
	ResumeHandle,
} from "../../../../lib/device-management/workspace/types";
import {
	type BillingGrant,
	type ResourceGrant,
	approveDeviceBilling,
	createDeviceResourceGrant,
	isActiveGrant,
	loadDeviceResources,
} from "../../../../lib/device-resources";
import {
	type IBackendState,
	useBackend,
} from "../../../../state/backend-state";
import type { DevicesT } from "../primitives/area-context";
import {
	DeviceRejectedError,
	deviceCall,
	requestOrReject,
	useDeviceWorkspace,
} from "../workspace";
import type { DeployPrepared } from "./step-props";
import {
	type CopyEventCheck,
	type CopyRefusedEvent,
	type CopyRow,
	type CopyUploadEntries,
	type CopyUploadEntry,
	DeployRunFailure,
	type DeviceCopyFacts,
	type SafeUpdateTimings,
	type StrategyReason,
	type UpdateStrategy,
	blocksNewUpdate,
	deployTarget,
	installedFromExisting,
	installedProject,
	planCatalog,
	secretCount,
	startsAfter,
	targetPhases,
	updateStrategy,
	uploadsNothing,
} from "./update-path";

export { DeployRunFailure, deployTarget };

/* One deploy across devices (APP §3.13): `reduceDeployRun` + the shared phase + per-target effects + tray items. */

export interface DeployRunOptions {
	title: CopyRef<string>;
	oneAtATime: boolean;
	stopOnFail: boolean;
	resumeId?: string;
}

export interface DeployRunHandle {
	state: DeployRunState;
	start(): void;
	retry(target: string): void;
	skip(target: string): void;
	resume(): void;
	stop(): void;
}

/** What `useDeployRun(plan, opts)` has no slot for; set before `start()`. */
export interface DeployRunExtras {
	/** The bundle step 2 prepared; without it the run prepares by itself. */
	prepared?: DeployPrepared | null;
	timings?: SafeUpdateTimings;
	/** Start a service that is stopped now once it is updated. */
	startStopped?: boolean;
	/** The entry's deploy route; tray items open it on the Rollout step. */
	route?: DeployRoute;
	/** Called when the run ends while no Rollout step shows it. */
	announce?(state: DeployRunState): void;
}

/** What the result shows beyond the run state; tokens stay in memory only. */
export interface DeployTargetDetail {
	target: string;
	deviceId: string;
	deviceName: string;
	serviceId: string;
	kind: PlanTargetService["kind"];
	strategy?: UpdateStrategy;
	reasons?: StrategyReason[];
	/** The access token of a new endpoint, shown once. */
	token?: string;
	fromSettings?: number;
	toSettings?: number;
	wasRunning?: boolean;
	/** What a failed target left on the hub. */
	kept?: "approval" | "approval_spending";
	/** Created or updated, but not running. */
	stopped?: "as_asked" | "start_refused";
	operationId?: string;
	rolloutId?: string;
	transferId?: string;
	rollout?: DeploymentRolloutStatus;
}

export type DeployRunDetails = Readonly<Record<string, DeployTargetDetail>>;

const MAX_REMEMBERED = 16;
const PROGRESS_EVERY_MS = 400;
const extrasById = new Map<string, DeployRunExtras>();

export function setDeployRunExtras(
	deploymentId: string,
	extras: DeployRunExtras,
): void {
	const merged = { ...extrasById.get(deploymentId), ...extras };
	extrasById.delete(deploymentId);
	extrasById.set(deploymentId, merged);
	for (const id of [...extrasById.keys()].slice(0, -MAX_REMEMBERED))
		extrasById.delete(id);
}

export function deployRunExtras(deploymentId: string): DeployRunExtras {
	return extrasById.get(deploymentId) ?? {};
}

/**
 * The wizard's copy lives as long as the wizard: its files are read from a
 * snapshot the wizard gives back when it closes. Rollout hands the copy over
 * while it is shown and takes it back when it goes; from then on the run
 * prepares its own.
 */
export function useHandedBundle(
	deploymentId: string,
	bundle: DeployPrepared | null | undefined,
) {
	useEffect(() => {
		if (!bundle) return;
		setDeployRunExtras(deploymentId, { prepared: bundle });
		return () => {
			if (deployRunExtras(deploymentId).prepared === bundle)
				setDeployRunExtras(deploymentId, { prepared: null });
		};
	}, [deploymentId, bundle]);
}

/** What is deployed, for titles: the event of a one-event deploy, else the app. */
export function deployWhat(plan: DeployPlan): string {
	const { draft, app } = plan;
	const single = draft.scope === "event" && draft.events.length === 1;
	const event = single
		? app?.events.find((value) => value.id === draft.events[0])
		: undefined;
	return event?.name ?? app?.name ?? "";
}

/** The run's title as the tray keeps it (APP §7.8): code `deploy` or `update` with `what`, `count` and, for one device, `device`. */
export function deployRunTitleRef(plan: DeployPlan): CopyRef<string> {
	const [only] = plan.targets;
	const one = plan.targets.length === 1 && only;
	return {
		code: plan.draft.entry === "update" ? "update" : "deploy",
		params: {
			what: deployWhat(plan).slice(0, 256),
			count: plan.targets.length,
			...(one ? { device: one.name.slice(0, 256) } : {}),
		},
	};
}

export function deployRunTitle(t: DevicesT, title: CopyRef<string>): string {
	const { what = "", device } = title.params ?? {};
	const count = Number(title.params?.count ?? 0);
	if (title.code === "update")
		return device
			? t("devices:deployShip.run.updateOne", "Update {{what}} on {{device}}", {
					what,
					device,
				})
			: t(
					"devices:deployShip.run.updateMany",
					"Update {{what}} on {{count, number}} devices",
					{ what, count },
				);
	return device
		? t("devices:deployShip.run.deployOne", "Deploy {{what}} to {{device}}", {
				what,
				device,
			})
		: t(
				"devices:deployShip.run.deployMany",
				"Deploy {{what}} to {{count, number}} devices",
				{ what, count },
			);
}

/* Failures. */

/** The device's keys locked: the target waits instead of failing. */
class Blocked extends Error {}
/** The workspace was disposed or the run abandoned: nothing more is sent or reported. */
class Halted extends Error {}

const ROLLOUT_END_CODE: Partial<Record<string, string>> = {
	rolled_back: "rolled_back",
	cancelled: "rollout_cancelled",
	failed: "rollout_failed",
};

type Classified = Pick<DeployFailure, "code" | "detail" | "rolledBack">;

const CLASSIFIERS: ((error: unknown) => Classified | undefined)[] = [
	(error) =>
		error instanceof DeployRunFailure
			? { code: error.code, detail: error.message }
			: undefined,
	(error) =>
		error instanceof DeploymentRolloutEndedError
			? {
					code: ROLLOUT_END_CODE[error.status.state] ?? "rollout_failed",
					detail: error.status.failure_code ?? undefined,
					rolledBack: error.status.state === "rolled_back",
				}
			: undefined,
	(error) =>
		error instanceof DeploymentPublicationFailedError
			? { code: "secret_publication", detail: error.message }
			: undefined,
	(error) =>
		error instanceof StaleDeploymentRevisionError
			? { code: "stale", detail: error.message }
			: undefined,
	(error) =>
		error instanceof DeploymentReviewRequiredError
			? { code: "review_required", detail: error.message }
			: undefined,
	(error) =>
		error instanceof DeploymentRejectedError
			? { code: error.rejection.code, detail: error.rejection.error }
			: undefined,
	(error) =>
		error instanceof DeviceRejectedError
			? {
					code: error.rejection.code || "rejected",
					detail: error.rejection.error,
				}
			: undefined,
	(error) =>
		error instanceof ArtifactUploadError
			? {
					code: error.rejection?.code ?? "upload_unconfirmed",
					detail: error.rejection?.error ?? error.message,
				}
			: undefined,
];

function classify(error: unknown): Classified {
	for (const classifier of CLASSIFIERS) {
		const known = classifier(error);
		if (known) return known;
	}
	return {
		code: "unconfirmed",
		detail: error instanceof Error ? error.message : String(error),
	};
}

/** Errors after which the prepared commands can't be sent again: a retry reads the service anew. */
function dropsPlan(error: unknown): boolean {
	return [
		DeploymentRolloutEndedError,
		DeploymentPublicationFailedError,
		StaleDeploymentRevisionError,
		DeploymentReviewRequiredError,
		DeploymentRejectedError,
	].some((kind) => error instanceof kind);
}

/* Per-target memory: what a retry reuses. Never persisted (the plan carries secrets). */

interface TargetMemo {
	existing?: PlacementConfiguration;
	grant?: ResourceGrant;
	billing?: BillingGrant;
	installed?: InstalledProject;
	catalog?: DeploymentCatalog;
	deployment?: DeploymentPlan;
	applied?: boolean;
	startOperation?: string;
	progressKey?: string;
	progressAt?: number;
}

interface Job {
	run: DeployRun;
	target: string;
	plan: DeployPlan;
	device: PlanTarget;
	service: PlanTargetService;
	memo: TargetMemo;
	call: ManagementCall;
	signal: AbortSignal;
}

interface Prepared {
	bundle: DeployPrepared;
	release?: () => Promise<void>;
}

const appIdOf = (plan: DeployPlan): string => {
	const id = plan.app?.id ?? plan.draft.appId;
	if (!id) throw new DeployRunFailure("app_missing");
	return id;
};

const sourceOf = (plan: DeployPlan) =>
	plan.mode === "offline" ? "offline" : "online";

const draftTargetOf = (job: Job): DeployTargetDraft | undefined =>
	job.plan.draft.targets.find(
		(target) => target.deviceId === job.device.deviceId,
	);

const hostedService = (job: Job) =>
	job.plan.services.find((service) => service.key === job.service.key)
		?.hosted === true;

function newToken(): string {
	const bytes = crypto.getRandomValues(new Uint8Array(32));
	return btoa(String.fromCharCode(...bytes))
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replaceAll("=", "");
}

/* Preparing the bundle (the shared phase) when step 2 handed none over. */

function bundleOf(
	artifact: PreparedProjectArtifact,
	approved?: DeployPrepared["approved"],
): DeployPrepared {
	return {
		artifact,
		...(approved ? { approved } : {}),
		preparedAt: Date.now(),
	};
}

async function prepareOnline(
	run: DeployRun,
	appId: string,
	signal: AbortSignal,
): Promise<Prepared> {
	const { deps } = run.workspace;
	const backend = run.requireBackend();
	const approved = await prepareOnlineMetadata(
		appId,
		backend,
		deps.profile,
		signal,
	);
	const { app } = approved;
	const native =
		deps.platform === "desktop" &&
		(app.bits.length > 0 || Object.keys(app.packages ?? {}).length > 0);
	if (!native) {
		const exported = await prepareOnlineDependencies(
			app,
			backend,
			deps.profile,
			signal,
			approved,
		);
		return { bundle: bundleOf(exported.artifact, approved) };
	}
	const exported = await prepareDesktopProject(
		appId,
		await desktopExportCommands(app),
		signal,
		approved,
	);
	return {
		bundle: bundleOf(exported.artifact, approved),
		release: exported.release,
	};
}

async function prepareOffline(
	run: DeployRun,
	appId: string,
	signal: AbortSignal,
): Promise<Prepared> {
	const { deps } = run.workspace;
	if (deps.platform !== "desktop") throw new DeployRunFailure("local_only_web");
	const exported = await prepareDesktopProject(
		appId,
		await desktopExportCommands(undefined, deps.scope.account),
		signal,
	);
	return { bundle: bundleOf(exported.artifact), release: exported.release };
}

/* Shipping the version: upload and install. */

export interface CopyUploadInput {
	workspace: DeviceWorkspace;
	deviceId: string;
	bundle: DeployPrepared;
	call: ManagementCall;
	signal?: AbortSignal;
	onProgress?(progress: ArtifactProgress): void;
}

function transferHint(input: CopyUploadInput, id: string, confirmed: boolean) {
	const { descriptor } = input.bundle.artifact;
	rememberArtifactTransfer(
		input.deviceId,
		{
			transfer_id: id,
			project_id: descriptor.project_id,
			manifest_sha256: descriptor.manifest_sha256,
			confirmed,
		},
		input.workspace.deps.scope,
	);
}

/** A begin whose reply was lost still holds staging space on the device until it is resumed or aborted. */
function rememberLostBegin(
	input: CopyUploadInput,
	error: unknown,
	resumed: string | undefined,
) {
	if (!(error instanceof ArtifactUploadError)) return;
	if (error.transferId) transferHint(input, error.transferId, false);
	else if (resumed)
		forgetArtifactTransfer(input.deviceId, resumed, input.workspace.deps.scope);
}

/** Sends the bundle to one device, resuming an upload this browser began earlier for the same version. */
export async function uploadPreparedCopy(
	input: CopyUploadInput,
): Promise<ArtifactTransferStatus> {
	const { deviceId, bundle } = input;
	const { scope } = input.workspace.deps;
	const { descriptor } = bundle.artifact;
	const pending = pendingArtifactTransfers(
		deviceId,
		descriptor.project_id,
		scope,
	).find((entry) => entry.manifest_sha256 === descriptor.manifest_sha256);
	let confirmed = "";
	const onProgress = (progress: ArtifactProgress) => {
		if (confirmed !== progress.transferId) {
			confirmed = progress.transferId;
			transferHint(input, confirmed, true);
		}
		input.onProgress?.(progress);
	};
	try {
		const status = await uploadProjectArtifact({
			prepared: bundle.artifact,
			request: input.call,
			transferId: pending?.transfer_id,
			confirmed: pending ? pending.confirmed === true : undefined,
			signal: input.signal,
			onProgress,
		});
		forgetArtifactTransfer(deviceId, status.transfer_id, scope);
		return status;
	} catch (error) {
		rememberLostBegin(input, error, pending?.transfer_id);
		throw error;
	}
}

/** The device's own check of the copy: its events and, per served event, its settings. */
export async function discoverCopy(
	call: ManagementCall,
	installed: InstalledProject,
	eventIds: readonly string[],
): Promise<DeploymentCatalog> {
	const events = await discoverOfflineEvents(call, installed);
	const variables: Record<string, DeploymentVariable[]> = {};
	for (const id of eventIds) {
		const event = events.find((value) => value.id === id);
		if (!event?.eligible)
			throw new DeployRunFailure("event_refused", event?.ineligible_reason);
		variables[id] = await discoverOfflineVariables(call, installed, id);
	}
	return { events, variables };
}

/* Uploads the Copy & upload step runs before the rollout; they go on while the step isn't shown. */

export interface CopyUploads {
	subscribe(listener: () => void): () => void;
	snapshot(): CopyUploadEntries;
	set(key: string, entry: CopyUploadEntry | undefined): void;
	patch(key: string, change: Partial<CopyUploadEntry>): void;
}

function createCopyUploads(): CopyUploads {
	let entries: CopyUploadEntries = {};
	const listeners = new Set<() => void>();
	const set = (key: string, entry: CopyUploadEntry | undefined) => {
		const { [key]: _previous, ...rest } = entries;
		entries = entry ? { ...rest, [key]: entry } : rest;
		for (const listener of listeners) listener();
	};
	return {
		subscribe: (listener) => {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
		snapshot: () => entries,
		set,
		patch: (key, change) => {
			const current = entries[key];
			if (current) set(key, { ...current, ...change });
		},
	};
}

const copyStores = new WeakMap<DeviceWorkspace, CopyUploads>();

export function copyUploadsOf(workspace: DeviceWorkspace): CopyUploads {
	const known = copyStores.get(workspace);
	if (known) return known;
	const store = createCopyUploads();
	copyStores.set(workspace, store);
	return store;
}

function pendingCopy(
	workspace: DeviceWorkspace,
	deviceId: string,
	bundle: DeployPrepared,
): PendingArtifactTransfer | undefined {
	const { descriptor } = bundle.artifact;
	return pendingArtifactTransfers(
		deviceId,
		descriptor.project_id,
		workspace.deps.scope,
	).find((entry) => entry.manifest_sha256 === descriptor.manifest_sha256);
}

/** One on-demand read per device: its storage (when the agent reports it) and how far an earlier upload got. */
export async function readDeviceCopyFacts(
	workspace: DeviceWorkspace,
	deviceId: string,
	bundle: DeployPrepared,
): Promise<DeviceCopyFacts> {
	const call = deviceCall(workspace, deviceId, "user");
	const features = workspace.live.inspection(deviceId)?.value.features;
	const projectId = bundle.artifact.descriptor.project_id;
	const read = await readArtifactUsage(call, features, { projectId }).catch(
		() => undefined,
	);
	const pending = pendingCopy(workspace, deviceId, bundle);
	const transfer = pending
		? await readArtifactTransfer(call, pending).catch(() => undefined)
		: undefined;
	return {
		...(read?.kind === "ok" ? { usage: read.data } : {}),
		...(pending ? { pending } : {}),
		...(transfer === undefined ? {} : { transfer }),
	};
}

const BUSY_RETRIES = 5;
const BUSY_WAIT_MS = 2_000;

export interface CopySend {
	workspace: DeviceWorkspace;
	bundle: DeployPrepared;
	row: CopyRow;
	call: ManagementCall;
	/** Updates the tray item of this upload. */
	track(patch: Partial<ActivityItem>): void;
}

function copyProgress(input: CopySend) {
	const store = copyUploadsOf(input.workspace);
	const { descriptor } = input.bundle.artifact;
	const last = { at: 0, files: -1 };
	return (progress: ArtifactProgress) => {
		const now = Date.now();
		const finished = progress.completedFiles >= progress.totalFiles;
		const stale = now - last.at >= PROGRESS_EVERY_MS;
		if (progress.completedFiles === last.files || (!stale && !finished)) return;
		last.at = now;
		last.files = progress.completedFiles;
		const count = { done: progress.completedFiles, total: progress.totalFiles };
		store.patch(input.row.key, { phase: "sending", ...count });
		input.track({
			progress: { ...count, unit: "files" },
			detail: { code: "files_progress", params: count },
			resume: {
				type: "transfer",
				transferId: progress.transferId,
				projectId: descriptor.project_id,
				manifestSha256: descriptor.manifest_sha256,
				expiresAt: Math.floor(now / 1000) + 86_400,
				confirmed: true,
			},
		});
	};
}

const pause = (ms: number) =>
	new Promise<void>((resolve) => setTimeout(resolve, ms));

/** A busy device is asked again a few times before the upload counts as failed; it resumes where it stopped. */
export async function sendCopy(
	input: CopySend,
): Promise<ArtifactTransferStatus> {
	const onProgress = copyProgress(input);
	for (let attempt = 0; ; attempt++) {
		try {
			return await uploadPreparedCopy({
				workspace: input.workspace,
				deviceId: input.row.target.deviceId,
				bundle: input.bundle,
				call: input.call,
				onProgress,
			});
		} catch (error) {
			const busy =
				error instanceof ArtifactUploadError &&
				error.rejection?.code === "busy";
			if (!busy || attempt >= BUSY_RETRIES) throw error;
			copyUploadsOf(input.workspace).patch(input.row.key, { phase: "busy" });
			await pause(BUSY_WAIT_MS);
		}
	}
}

/** The device checks the events against the copy it now holds. */
export async function checkCopyEvents(
	input: Pick<CopySend, "workspace" | "bundle" | "row">,
	status: ArtifactTransferStatus,
): Promise<CopyEventCheck | undefined> {
	if (!status.project_path) return undefined;
	const { workspace, bundle, row } = input;
	const call = deviceCall(workspace, row.target.deviceId, "user");
	const installed = installedProject(bundle, status.project_path);
	try {
		const events = await discoverOfflineEvents(call, installed);
		const wanted = new Set(
			row.target.services.flatMap((service) => service.events),
		);
		const mine = events.filter((event) => wanted.has(event.id));
		const accepted = mine.filter((event) => event.eligible);
		const variables: CopyEventCheck["variables"] = {};
		for (const event of accepted)
			variables[event.id] = await discoverOfflineVariables(
				call,
				installed,
				event.id,
			);
		return {
			runs: accepted.map((event) => event.name || event.id),
			refused: mine.filter((event) => !event.eligible).map(refusedEvent),
			variables,
		};
	} catch {
		return undefined;
	}
}

function refusedEvent(event: DeploymentEvent): CopyRefusedEvent {
	const reason = event.ineligible_reason;
	return {
		id: event.id,
		name: event.name || event.id,
		...(reason ? { reason } : {}),
	};
}

/* The per-target chain. Each step is skipped when an earlier attempt finished it. */

function guard(job: Job) {
	job.run.guard(job.device.deviceId);
}

async function loadExisting(job: Job) {
	const { memo, service, plan } = job;
	if (service.kind === "new" || memo.existing) return;
	guard(job);
	const existing = await readExistingDeployment(
		job.call,
		service.serviceId,
		appIdOf(plan),
	);
	if (existing.config.source !== sourceOf(plan))
		throw new DeployRunFailure("source_changed");
	if (blocksNewUpdate(existing.rollout?.state))
		throw new DeployRunFailure("update_in_progress");
	memo.existing = existing;
	job.run.note(job.target, {
		fromSettings: existing.config_revision,
		toSettings: existing.config_revision + 1,
		wasRunning: existing.desired_state === "running",
	});
}

function matchingGrant(job: Job, grants: readonly ResourceGrant[]) {
	return grants.find(
		(grant) =>
			grant.placement_id === job.service.serviceId &&
			grant.deployment_id === job.plan.draft.deploymentId &&
			isActiveGrant(grant, job.run.at()),
	);
}

async function setSpending(job: Job, held: readonly BillingGrant[]) {
	const { memo, plan } = job;
	const { spending, approval } = plan.draft;
	const grant = memo.grant;
	const wanted = job.run.phasesOf(job.target).includes("spending");
	if (!grant || !spending || !wanted) return;
	job.run.alive();
	job.run.phase(job.target, "spending");
	const { api, profile } = job.run.workspace.deps;
	const kept = held.find(
		(billing) =>
			billing.grant_id === grant.grant_id &&
			isActiveGrant(billing, job.run.at()),
	);
	memo.billing ??=
		kept ??
		(await approveDeviceBilling(
			api,
			profile,
			job.device.deviceId,
			grant.grant_id,
			{
				limit_micros:
					draftTargetOf(job)?.over.spendingLimitMicros ?? spending.limitMicros,
				expires_at: Math.min(spending.expiresAt, approval.expiresAt),
			},
		));
	job.run.note(job.target, { kept: "approval_spending" });
}

function createApproval(job: Job): Promise<ResourceGrant> {
	const { plan, service } = job;
	const { api, profile } = job.run.workspace.deps;
	const appId = appIdOf(plan);
	return createDeviceResourceGrant(
		api,
		profile,
		job.device.deviceId,
		approvalRequest(plan.draft.approval, {
			placementId: service.serviceId,
			deploymentId: plan.draft.deploymentId,
			projectId: appId,
			appId: plan.mode === "online" ? appId : null,
		}),
	);
}

/** A failed earlier attempt leaves its approval on the hub; the retry reuses it instead of creating a second one. */
async function approveAccess(job: Job) {
	const { memo } = job;
	const wanted = job.run.phasesOf(job.target).includes("approve");
	if (!wanted || (memo.grant && memo.billing)) return;
	const { api, profile } = job.run.workspace.deps;
	job.run.phase(job.target, "approve");
	const held = await loadDeviceResources(api, profile, job.device.deviceId);
	job.run.alive();
	memo.grant ??= matchingGrant(job, held.grants) ?? (await createApproval(job));
	job.run.note(job.target, { kept: "approval" });
	await setSpending(job, held.billing);
}

async function shipVersion(job: Job) {
	const { memo, plan, service } = job;
	if (memo.installed) return;
	if (uploadsNothing(plan, service)) {
		if (!memo.existing) throw new DeployRunFailure("update_needs_service");
		memo.installed = installedFromExisting(memo.existing);
		return;
	}
	const shipped = await uploadVersion(job, await job.run.bundle());
	const { status } = shipped;
	if (!status.project_path) throw new DeployRunFailure("upload_unconfirmed");
	memo.installed = installedProject(shipped.bundle, status.project_path);
}

function uploadBundle(job: Job, bundle: DeployPrepared) {
	guard(job);
	job.run.phase(job.target, "upload");
	return uploadPreparedCopy({
		workspace: job.run.workspace,
		deviceId: job.device.deviceId,
		bundle,
		call: job.call,
		signal: job.signal,
		onProgress: (progress) => job.run.uploadProgress(job, bundle, progress),
	});
}

/** The device said no (or the run is gone): sending another copy changes nothing. */
function refusedOrHalted(error: unknown): boolean {
	if (error instanceof Halted || error instanceof DeviceRejectedError)
		return true;
	return error instanceof ArtifactUploadError && error.rejection !== undefined;
}

/**
 * The wizard takes its copy back when it closes, and its files can't be read
 * any more. An upload that was reading them then goes on with a copy the run
 * prepares itself, once.
 */
async function uploadVersion(job: Job, bundle: DeployPrepared) {
	try {
		return { bundle, status: await uploadBundle(job, bundle) };
	} catch (error) {
		if (refusedOrHalted(error) || !job.run.handedBack(bundle)) throw error;
		const own = await job.run.bundle();
		return { bundle: own, status: await uploadBundle(job, own) };
	}
}

async function catalogOf(job: Job): Promise<DeploymentCatalog> {
	const installed = job.memo.installed;
	if (!installed) throw new DeployRunFailure("not_prepared");
	if (installed.online_catalog) return installed.online_catalog;
	if (installed.source === "online")
		return planCatalog(job.plan, job.memo.existing);
	guard(job);
	job.run.phase(job.target, "check_events");
	return discoverCopy(job.call, installed, job.service.events);
}

function previousVariables(
	job: Job,
	installed: InstalledProject,
): Promise<DeploymentVariable[]> {
	const { existing } = job.memo;
	if (!existing) return Promise.resolve([]);
	const { eventState, boardState } = job.run.requireBackend();
	const read =
		installed.source === "offline"
			? discoverPreviousOfflineVariables(job.call, installed, existing)
			: discoverPreviousOnlineVariables(
					eventState,
					boardState,
					installed,
					existing,
				);
	// Unknown earlier definitions leave the type check of kept secrets to the device.
	return read.catch(() => []);
}

function tokenFor(job: Job): string {
	const { endpoint } = job.plan.draft;
	if (!hostedService(job)) return "";
	if (endpoint.token === "keep")
		return job.memo.existing?.config.hosting ? "" : newToken();
	const own = draftTargetOf(job)?.over.token;
	if (own) return own;
	if (endpoint.token === "per_device") return newToken();
	return endpoint.tokenValue || job.run.sharedToken();
}

function grantReference(memo: TargetMemo): Record<string, unknown> | undefined {
	const { grant, billing } = memo;
	if (!grant) return undefined;
	return {
		grant_id: grant.grant_id,
		authz_version: grant.authz_version,
		...(billing
			? {
					billing_grant_id: billing.billing_grant_id,
					billing_authz_version: billing.authz_version,
				}
			: {}),
	};
}

/** Queued writes replay only while their tables stay buffered: an update that drops them waits for an empty queue. */
async function requireDrainedQueues(job: Job) {
	const { existing } = job.memo;
	const next = job.plan.draft.writes;
	if (!existing || next === undefined) return;
	if (!removesOfflineBuffering(existing.config.offline_writes, next)) return;
	guard(job);
	try {
		await assertOfflineQueuesDrained(job.call, existing.placement_id);
	} catch (error) {
		throw new DeployRunFailure(
			"queue_not_empty",
			error instanceof Error ? error.message : undefined,
		);
	}
}

function applyTimings(
	deployment: DeploymentPlan,
	timings: SafeUpdateTimings | undefined,
) {
	const stage = deployment.steps.find(
		(step) => step.command.type === "stage_rollout",
	);
	if (!stage || !timings) return;
	stage.command.stabilization_seconds = timings.stabilizeSeconds;
	stage.command.deadline_seconds = timings.deadlineSeconds;
}

function buildDeployment(job: Job, facts: WireFacts): DeploymentPlan {
	try {
		return createDeploymentPlan(
			wirePlan(
				job.plan,
				{ deviceId: job.device.deviceId, serviceKey: job.service.key },
				facts,
			),
		);
	} catch (error) {
		throw new DeployRunFailure(
			"invalid_plan",
			error instanceof Error ? error.message : undefined,
		);
	}
}

async function wireDeployment(job: Job): Promise<DeploymentPlan> {
	const { memo } = job;
	if (memo.deployment) return memo.deployment;
	const installed = memo.installed;
	if (!installed) throw new DeployRunFailure("not_prepared");
	memo.catalog ??= await catalogOf(job);
	await requireDrainedQueues(job);
	const serviceToken = tokenFor(job);
	const deployment = buildDeployment(job, {
		installed,
		existing: memo.existing,
		events: memo.catalog.events,
		variables: memo.catalog.variables,
		previousVariables: await previousVariables(job, installed),
		serviceToken,
		resourceGrant: grantReference(memo),
		canManageCertificates: true,
	});
	applyTimings(deployment, job.run.extras.timings);
	job.run.alive();
	memo.deployment = deployment;
	job.run.wired(job, deployment, serviceToken);
	return deployment;
}

const SECRET_COMMANDS = ["set_secret", "rollout_secret"];

const COMMAND_PHASE: Partial<Record<string, (job: Job) => DeployPhase>> = {
	apply: (job) => (job.service.kind === "new" ? "create" : "stop"),
	stage_rollout: () => "prepare_update",
	set_secret: () => "secrets",
	rollout_secret: () => "secrets",
	activate_rollout: () => "check_new",
};

async function execute(job: Job, deployment: DeploymentPlan) {
	const call: ManagementCall = (command, operationId) => {
		// A lock between two commands sends nothing further: the remaining secrets wait for the unlock.
		guard(job);
		job.run.sending(job, command, operationId);
		return job.call(command, operationId);
	};
	try {
		await executeDeploymentPlan(call, deployment, job.signal, (status) =>
			job.run.rolloutStatus(job, status),
		);
	} catch (error) {
		if (dropsPlan(error)) {
			job.memo.deployment = undefined;
			job.memo.existing = undefined;
			job.memo.catalog = undefined;
		}
		throw error;
	}
	// The applied plan held secret values; nothing keeps them once the device has them.
	job.memo.deployment = undefined;
	job.memo.applied = true;
}

async function startService(job: Job) {
	if (!job.run.phasesOf(job.target).includes("start")) return;
	guard(job);
	job.run.phase(job.target, "start");
	job.memo.startOperation ??= crypto.randomUUID();
	try {
		await requestOrReject(
			job.call,
			{ type: "start", placement_id: job.service.serviceId },
			job.memo.startOperation,
		);
	} catch (error) {
		// The service exists; a refused start leaves it stopped instead of failing the deploy.
		if (!(error instanceof DeviceRejectedError) || error.rejection.retryable)
			throw error;
		job.run.note(job.target, { stopped: "start_refused" });
	}
}

async function runChain(job: Job) {
	if (!job.memo.applied) {
		await loadExisting(job);
		await approveAccess(job);
		await shipVersion(job);
		const deployment = await wireDeployment(job);
		await execute(job, deployment);
	}
	await startService(job);
}

/* Rows. */

type RowSeed = Pick<
	DeployRunRow,
	"target" | "deviceId" | "serviceId" | "phases"
>;

function assumedPhases(
	plan: DeployPlan,
	target: PlanTarget,
	service: PlanTargetService,
	startStopped: boolean,
): DeployPhase[] {
	return targetPhases(plan, service, {
		secrets: secretCount(plan, target, service) > 0,
		strategy: plan.draft.strategy === "quick" ? "quick" : "safe",
		wasRunning: true,
		startStopped,
	});
}

/** One row per served service on each device; a service left without events has nothing to run. */
export function deployRunRows(
	plan: DeployPlan,
	startStopped = false,
): RowSeed[] {
	return plan.targets.flatMap((target) =>
		target.services
			.filter((service) => service.events.length > 0)
			.map((service) => ({
				target: deployTarget(target.deviceId, service.serviceId),
				deviceId: target.deviceId,
				serviceId: service.serviceId,
				phases: assumedPhases(plan, target, service, startStopped),
			})),
	);
}

function sharedPhaseOf(plan: DeployPlan): SharedPhase | null {
	const ships = plan.targets.some((target) =>
		target.services.some(
			(service) => service.events.length && !uploadsNothing(plan, service),
		),
	);
	if (!ships) return null;
	return plan.mode === "offline" ? "prepare" : "approve_definitions";
}

type RunShape = Pick<DeployRunOptions, "oneAtATime" | "stopOnFail">;

function orderOf(plan: DeployPlan, options: RunShape) {
	if (options.oneAtATime) return "one" as const;
	return plan.draft.order === "first" ? ("first" as const) : ("all" as const);
}

function idleState(plan: DeployPlan, options: RunShape, id = "") {
	return createDeployRun({
		id,
		rows: deployRunRows(
			plan,
			deployRunExtras(plan.draft.deploymentId).startStopped,
		),
		order: orderOf(plan, options),
		stopOnFail: options.stopOnFail,
		shared: sharedPhaseOf(plan),
	});
}

const EMPTY_STATE = createDeployRun({
	id: "",
	rows: [],
	order: "one",
	stopOnFail: true,
});

/* The run's id across reloads: `deploymentId → run id` in this window, else the newest unfinished run with the same targets. */

const RUNS_KEY = "flow-like.deploy-runs.";

function readRunIds(scopeKey: string): Record<string, string> {
	try {
		const value: unknown = JSON.parse(
			globalThis.sessionStorage?.getItem(RUNS_KEY + scopeKey) ?? "{}",
		);
		return value && typeof value === "object" && !Array.isArray(value)
			? (value as Record<string, string>)
			: {};
	} catch {
		return {};
	}
}

function rememberRunId(scopeKey: string, deploymentId: string, runId: string) {
	try {
		const entries = Object.entries({
			...readRunIds(scopeKey),
			[deploymentId]: runId,
		}).slice(-MAX_REMEMBERED);
		globalThis.sessionStorage?.setItem(
			RUNS_KEY + scopeKey,
			JSON.stringify(Object.fromEntries(entries)),
		);
	} catch {
		// Without session storage a reload finds the run by its targets.
	}
}

const OPEN_ITEM = new Set(["active", "paused", "waiting"]);

function sameTargets(items: readonly ActivityItem[], plan: DeployPlan) {
	const wanted = new Set(deployRunRows(plan).map((row) => row.target));
	const held = new Set(
		items.map((item) =>
			deployTarget(item.target.deviceId, item.target.serviceId ?? ""),
		),
	);
	return wanted.size === held.size && [...wanted].every((key) => held.has(key));
}

function unfinishedRun(
	activity: ActivityTracker,
	plan: DeployPlan,
): ActivityRun | undefined {
	return [...activity.runs()]
		.sort((a, b) => b.startedAt - a.startedAt)
		.find((run) => {
			const items = activity.runItems(run.id);
			return (
				items.some((item) => OPEN_ITEM.has(item.state)) &&
				sameTargets(items, plan)
			);
		});
}

/** The tray run of this plan, if one was started in this window or is still unfinished. */
export function recallDeployRun(
	workspace: DeviceWorkspace,
	plan: DeployPlan,
): string | undefined {
	const known = readRunIds(workspace.scopeKey)[plan.draft.deploymentId];
	if (known && workspace.activity.run(known)) return known;
	return unfinishedRun(workspace.activity, plan)?.id;
}

/* Tray items. */

const PHASE_KIND: Partial<Record<DeployPhase, ActivityKind>> = {
	upload: "upload",
	install: "upload",
	check_events: "upload",
	prepare_update: "safe_update",
	check_new: "safe_update",
	switch: "safe_update",
	secrets: "secret_write",
};

const PHASE_COMMAND: Partial<Record<DeployPhase, string>> = {
	start: "start",
	stop: "stop",
};

const currentPhase = (row: DeployRunRow): DeployPhase | undefined =>
	row.phases[Math.min(row.phase, row.phases.length - 1)];

function kindOf(row: DeployRunRow): ActivityKind {
	const phase = currentPhase(row);
	return (phase && PHASE_KIND[phase]) ?? "command";
}

function labelOf(row: DeployRunRow): ActivityItem["label"] {
	const phase = currentPhase(row);
	const command = phase && PHASE_COMMAND[phase];
	return { code: kindOf(row), ...(command ? { params: { command } } : {}) };
}

type TrayEnd = { state: "done" | "failed"; detail: ActivityItem["detail"] };

function trayEnd(row: DeployRunRow): TrayEnd | undefined {
	if (row.state === "done") return { state: "done", detail: { code: "done" } };
	if (row.state === "skipped" || row.state === "not_started")
		return { state: "failed", detail: { code: "cancelled" } };
	if (row.state !== "failed") return undefined;
	return {
		state: "failed",
		detail: { code: row.error?.rolledBack ? "rolled_back" : "failed" },
	};
}

function trayProgress(
	row: DeployRunRow,
): Pick<ActivityItem, "progress" | "detail"> {
	const { progress } = row;
	if (row.state !== "active" || !progress)
		return { progress: undefined, detail: undefined };
	return {
		progress: { ...progress, unit: "files" },
		detail: { code: "files_progress", params: { ...progress } },
	};
}

/* Rows of a run this window did not start (reload): each follows its tray item. */

type Adopt = (row: DeployRunRow, item: ActivityItem) => DeployRunRow;

const seconds = (ms: number) => Math.floor(ms / 1000);

const interrupted = (
	row: DeployRunRow,
	item: ActivityItem,
	code: string,
): DeployRunRow => ({
	...row,
	state: "failed",
	at: seconds(item.finishedAt ?? item.updatedAt),
	error: { phase: row.phases[row.phase] ?? "upload", code },
});

/**
 * The page that ran the deploy clears the item's handle when a device is done.
 * A finished item that still has one was settled by the tray's own re-check:
 * a healthy update is complete, but an applied command or a committed upload
 * says nothing about the steps that should have followed.
 */
function adoptDone(row: DeployRunRow, item: ActivityItem): DeployRunRow {
	const handle = item.resume?.type;
	if (handle === "operation") return interrupted(row, item, "applied_only");
	if (handle === "transfer") return interrupted(row, item, "interrupted");
	return {
		...row,
		state: "done",
		phase: row.phases.length,
		finishedAt: seconds(item.finishedAt ?? item.updatedAt),
		at: seconds(item.finishedAt ?? item.updatedAt),
	};
}

const ADOPTED_STATE: Partial<Record<ActivityItem["state"], Adopt>> = {
	done: adoptDone,
	failed: (row, item) => ({
		...row,
		state: "failed",
		at: seconds(item.finishedAt ?? item.updatedAt),
		error: {
			phase: row.phases[row.phase] ?? "upload",
			code: item.detail?.code === "cancelled" ? "interrupted" : "tray_failed",
			rolledBack: item.detail?.code === "rolled_back",
		},
	}),
	unknown: (row, item) => ({
		...row,
		state: "failed",
		at: seconds(item.updatedAt),
		error: { phase: row.phases[row.phase] ?? "upload", code: "no_reply" },
	}),
};

const OPEN_ROW = ["held", "active", "blocked", "waiting"];

/* The controller: lives with the workspace, so the run goes on when the Rollout step is not mounted. */

type Listener = () => void;

class DeployRun {
	state: DeployRunState | null = null;
	details: DeployRunDetails = {};
	backend: IBackendState | null = null;
	plan: DeployPlan | null = null;
	options: DeployRunOptions | null = null;
	/** Rollout steps showing this run. */
	watchers = 0;
	private readonly listeners = new Set<Listener>();
	private readonly memos = new Map<string, TargetMemo>();
	private readonly inFlight = new Set<string>();
	private readonly items = new Map<string, string>();
	private readonly handles = new Map<string, ResumeHandle>();
	private abort = new AbortController();
	private bundlePromise: Promise<DeployPrepared> | null = null;
	/** The copy this run prepared itself. */
	private own: DeployPrepared | null = null;
	private release: (() => Promise<void>) | undefined;
	private token = "";
	private unwatchKeys: (() => void) | undefined;
	private unwatchTray: (() => void) | undefined;
	private announced = false;
	private adopted = false;

	constructor(
		readonly deploymentId: string,
		readonly workspace: DeviceWorkspace,
	) {}

	get extras(): DeployRunExtras {
		return deployRunExtras(this.deploymentId);
	}

	subscribe = (listener: Listener) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	getState = () => this.state;
	getDetails = () => this.details;
	isAdopted = () => this.adopted;

	private emit() {
		for (const listener of this.listeners) listener();
	}

	/** Unix seconds on the area's (hub-corrected) clock. */
	at(): number {
		return seconds(this.workspace.clock.now());
	}

	requireBackend(): IBackendState {
		if (!this.backend) throw new DeployRunFailure("not_prepared");
		return this.backend;
	}

	/** The workspace was disposed (another account, a closed page) or the run abandoned. */
	private gone(): boolean {
		const disposed = (this.workspace as { disposed?: boolean }).disposed;
		return disposed === true || this.abort.signal.aborted;
	}

	/** Throws once nothing may be sent or reported any more. */
	alive() {
		if (this.gone()) throw new Halted();
	}

	private locked(deviceId: string) {
		return this.workspace.keys.snapshot(deviceId).state !== "unlocked";
	}

	guard(deviceId: string) {
		this.alive();
		if (this.locked(deviceId)) throw new Blocked();
	}

	sharedToken(): string {
		this.token ||= newToken();
		return this.token;
	}

	private row(target: string): DeployRunRow | undefined {
		return this.state?.rows.find((value) => value.target === target);
	}

	phasesOf(target: string): DeployPhase[] {
		return this.row(target)?.phases ?? [];
	}

	/* State changes. */

	private commit(next: DeployRunState) {
		const previous = this.state;
		this.state = next;
		this.syncTray(previous);
		this.emit();
		this.drive();
	}

	dispatch(event: DeployRunEvent) {
		if (!this.state) return;
		const next = reduceDeployRun(this.state, event);
		if (next !== this.state) this.commit(next);
	}

	phase(
		target: string,
		phase: DeployPhase,
		progress?: { done: number; total: number },
	) {
		this.dispatch({ type: "phase", target, phase, at: this.at(), progress });
	}

	note(target: string, patch: Partial<DeployTargetDetail>) {
		const current = this.details[target];
		if (!current) return;
		this.details = { ...this.details, [target]: { ...current, ...patch } };
		this.emit();
	}

	private setPhases(target: string, phases: DeployPhase[]) {
		if (!this.state) return;
		const rows = this.state.rows.map((row) => {
			if (row.target !== target) return row;
			const current = row.phases[row.phase];
			const index = current ? phases.indexOf(current) : -1;
			const phase = index < 0 ? Math.min(row.phase, phases.length - 1) : index;
			return { ...row, phases, phase: Math.max(0, phase) };
		});
		this.commit({ ...this.state, rows });
	}

	/* What the chain reports. */

	uploadProgress(job: Job, bundle: DeployPrepared, progress: ArtifactProgress) {
		const phase = progress.phase === "commit" ? "install" : "upload";
		const key = `${phase}:${progress.completedFiles}`;
		const now = Date.now();
		const last = progress.completedFiles >= progress.totalFiles;
		const soon = now - (job.memo.progressAt ?? 0) < PROGRESS_EVERY_MS;
		if (job.memo.progressKey === key || (soon && !last && phase === "upload"))
			return;
		job.memo.progressKey = key;
		job.memo.progressAt = now;
		const { descriptor } = bundle.artifact;
		this.handles.set(job.target, {
			type: "transfer",
			transferId: progress.transferId,
			projectId: descriptor.project_id,
			manifestSha256: descriptor.manifest_sha256,
			expiresAt: this.at() + 86_400,
			confirmed: true,
		});
		this.note(job.target, { transferId: progress.transferId });
		this.phase(job.target, phase, {
			done: progress.completedFiles,
			total: progress.totalFiles,
		});
	}

	/** Strategy, phases and identifiers are final once the commands exist. */
	wired(job: Job, deployment: DeploymentPlan, token: string) {
		const safe = Boolean(deployment.rollout_id);
		const wasRunning = job.memo.existing?.desired_state === "running";
		const choice = {
			strategy: safe ? ("safe" as const) : ("quick" as const),
			wasRunning,
			startStopped: this.extras.startStopped === true,
		};
		this.note(job.target, {
			...this.strategyNote(job, safe),
			...(token && hostedService(job) ? { token } : {}),
			toSettings: deployment.expected_revision + 1,
			...(deployment.rollout_id ? { rolloutId: deployment.rollout_id } : {}),
			...this.stoppedNote(job, choice),
		});
		this.setPhases(
			job.target,
			targetPhases(job.plan, job.service, {
				...choice,
				secrets: deployment.steps.some((step) =>
					SECRET_COMMANDS.includes(String(step.command.type)),
				),
			}),
		);
	}

	private strategyNote(job: Job, safe: boolean): Partial<DeployTargetDetail> {
		if (job.service.kind === "new") return {};
		const events = (job.memo.catalog?.events ?? []).filter((event) =>
			job.service.events.includes(event.id),
		);
		const info = updateStrategy(job.plan.draft.strategy, {
			source: sourceOf(job.plan),
			existing: job.memo.existing,
			events,
		});
		return { strategy: safe ? "safe" : "quick", reasons: info.reasons };
	}

	private stoppedNote(
		job: Job,
		choice: Pick<
			Parameters<typeof startsAfter>[0],
			"strategy" | "wasRunning" | "startStopped"
		>,
	): Partial<DeployTargetDetail> {
		const starts = startsAfter({
			...choice,
			kind: job.service.kind,
			start: job.plan.draft.start,
		});
		return starts ? {} : { stopped: "as_asked" };
	}

	sending(job: Job, command: Record<string, unknown>, operationId?: string) {
		const type = String(command.type);
		if (type === "apply" && operationId) {
			this.handles.set(job.target, {
				type: "operation",
				operationId,
				command: "apply",
				issuedAt: this.at(),
			});
			this.note(job.target, { operationId });
		}
		if (type === "stage_rollout" && operationId)
			this.handles.set(job.target, {
				type: "rollout",
				rolloutId: operationId,
				placementId: job.service.serviceId,
				projectId: appIdOf(job.plan),
			});
		const phase = COMMAND_PHASE[type]?.(job);
		if (phase) this.phase(job.target, phase);
	}

	rolloutStatus(job: Job, status: DeploymentRolloutStatus) {
		this.note(job.target, { rollout: status });
		if (status.state === "validating") this.phase(job.target, "check_new");
		if (status.state === "activating" || status.state === "healthy")
			this.phase(job.target, "switch");
	}

	/* The bundle. */

	bundle(): Promise<DeployPrepared> {
		const handed = this.extras.prepared;
		if (handed) return Promise.resolve(handed);
		this.bundlePromise ??= this.prepare().catch((error: unknown) => {
			this.bundlePromise = null;
			throw error;
		});
		return this.bundlePromise;
	}

	/** True for the wizard's copy once the wizard took it back (it closed). */
	handedBack(bundle: DeployPrepared): boolean {
		return bundle !== this.own && this.extras.prepared !== bundle;
	}

	private async prepare(): Promise<DeployPrepared> {
		const plan = this.plan;
		if (!plan) throw new DeployRunFailure("app_missing");
		const appId = appIdOf(plan);
		const result = await (plan.mode === "offline"
			? prepareOffline(this, appId, this.abort.signal)
			: prepareOnline(this, appId, this.abort.signal));
		this.release = result.release;
		this.own = result.bundle;
		return result.bundle;
	}

	private async runShared() {
		try {
			await this.bundle();
			this.alive();
			this.dispatch({ type: "shared_done", at: this.at() });
		} catch (error) {
			if (error instanceof Halted) return;
			this.dispatch({
				type: "shared_fail",
				at: this.at(),
				error: { phase: "upload", ...classify(error) },
			});
		}
	}

	/* Driving the rows. */

	private drive() {
		const state = this.state;
		// A halted chain leaves its row active; nothing may start it again.
		if (!state || this.gone()) return;
		if (state.shared?.state === "active" && !this.inFlight.has("")) {
			this.inFlight.add("");
			void this.runShared().finally(() => this.inFlight.delete(""));
		}
		for (const target of activeTargets(state))
			if (!this.inFlight.has(target)) void this.runTarget(target);
		this.settleFinish();
	}

	private jobOf(target: string): Job | undefined {
		const plan = this.plan;
		const row = this.row(target);
		const device = plan?.targets.find(
			(value) => value.deviceId === row?.deviceId,
		);
		const service = device?.services.find(
			(value) => value.serviceId === row?.serviceId,
		);
		if (!plan || !device || !service) return undefined;
		const memo = this.memos.get(target) ?? {};
		this.memos.set(target, memo);
		return {
			run: this,
			target,
			plan,
			device,
			service,
			memo,
			call: deviceCall(this.workspace, device.deviceId, "operation"),
			signal: this.abort.signal,
		};
	}

	private async runTarget(target: string) {
		const job = this.jobOf(target);
		if (!job) return;
		this.inFlight.add(target);
		const release = this.workspace.live.acquire(
			job.device.deviceId,
			"operation",
		);
		try {
			await runChain(job);
			this.alive();
			this.finishTarget(job);
		} catch (error) {
			this.failTarget(job, error);
		} finally {
			release();
			this.inFlight.delete(target);
			this.drive();
		}
	}

	private finishTarget(job: Job) {
		const { deviceId } = job.device;
		const { queryClient } = this.workspace.deps;
		const scopeKey = this.workspace.scopeKey;
		this.handles.delete(job.target);
		this.dispatch({ type: "done", target: job.target, at: this.at() });
		void this.workspace.live.refreshInspection(deviceId).catch(() => {});
		for (const queryKey of [
			deviceKeys.resources(scopeKey, deviceId),
			deviceKeys.resourceSummary(scopeKey),
			deviceKeys.appPlacements(scopeKey, appIdOf(job.plan)),
		])
			void queryClient.invalidateQueries({ queryKey });
	}

	private failTarget(job: Job, error: unknown) {
		// A disposed workspace drops its sessions: what fails then is not the device's answer.
		if (error instanceof Halted || this.gone()) return;
		const at = this.at();
		const { target } = job;
		if (error instanceof Blocked || this.locked(job.device.deviceId)) {
			this.dispatch({ type: "block", target, at, reason: "locked" });
			return;
		}
		const row = this.row(target);
		const phase = (row && currentPhase(row)) ?? "upload";
		this.dispatch({
			type: "fail",
			target,
			at,
			error: { phase, ...classify(error) },
		});
	}

	/** Rows that wait for an unlock go on by themselves once the keys are open. */
	private onKeys = () => {
		for (const row of this.state?.rows ?? [])
			if (row.state === "blocked" && !this.locked(row.deviceId))
				this.dispatch({ type: "unblock", target: row.target, at: this.at() });
	};

	private settleFinish() {
		const state = this.state;
		if (state?.status !== "finished") {
			this.announced = false;
			return;
		}
		if (this.announced) return;
		this.announced = true;
		// A failed device may be retried: the native export stays readable until every device has it.
		if (state.rows.every((row) => row.state === "done")) this.releaseBundle();
		if (this.watchers === 0) this.extras.announce?.(state);
	}

	private releaseBundle() {
		void this.release?.().catch(() => {});
		this.release = undefined;
		this.bundlePromise = null;
	}

	/* The tray. */

	private hrefOf(row: DeployRunRow): DeployRoute {
		const appId = this.plan?.app?.id;
		const route: DeployRoute = this.extras.route ?? {
			screen: "deploy",
			deviceIds: [row.deviceId],
			...(appId ? { appId } : {}),
		};
		const deviceIds =
			route.deviceIds.length <= 64 ? route.deviceIds : [row.deviceId];
		// The tray stores plain data: no `undefined` fields.
		return JSON.parse(
			JSON.stringify({ ...route, deviceIds, step: "rollout" }),
		) as DeployRoute;
	}

	private itemOf(row: DeployRunRow): Parameters<ActivityTracker["start"]>[0] {
		const name = this.details[row.target]?.deviceName;
		const appId = this.plan?.app?.id;
		return {
			kind: kindOf(row),
			target: {
				deviceId: row.deviceId,
				...(name ? { deviceName: name.slice(0, 256) } : {}),
				...(row.serviceId ? { serviceId: row.serviceId } : {}),
				...(appId ? { projectId: appId } : {}),
			},
			state: "waiting",
			label: labelOf(row),
			startedBy: "you",
			actions: ["open"],
			href: this.hrefOf(row),
		};
	}

	private openTray(rows: readonly DeployRunRow[]): string {
		const options = this.options;
		try {
			if (!options) throw new Error("The run has no options.");
			const run = this.workspace.activity.startRun({
				title: options.title,
				oneAtATime: options.oneAtATime,
				stopOnFail: options.stopOnFail,
				items: rows.map((row) => this.itemOf(row)),
			});
			run.itemIds.forEach((itemId, index) => {
				const row = rows[index];
				if (row) this.items.set(row.target, itemId);
			});
			rememberRunId(this.workspace.scopeKey, this.deploymentId, run.id);
			return run.id;
		} catch {
			// A refused tray entry never stops the deploy; it then lives on this page only.
			return crypto.randomUUID();
		}
	}

	private syncTray(previous: DeployRunState | null) {
		for (const row of this.state?.rows ?? []) {
			const before = previous?.rows.find(
				(value) => value.target === row.target,
			);
			if (before !== row) this.trayRow(row);
		}
	}

	private trayRow(row: DeployRunRow) {
		const itemId = this.items.get(row.target);
		if (!itemId) return;
		const { activity } = this.workspace;
		const end = trayEnd(row);
		const active = row.state === "active";
		try {
			if (end) {
				// No handle is left behind: a later re-check must not settle a device this page already settled.
				activity.update(itemId, {
					state: end.state,
					detail: end.detail,
					resume: undefined,
					progress: undefined,
				});
				return;
			}
			activity.update(itemId, {
				kind: kindOf(row),
				label: labelOf(row),
				state: active ? "active" : "waiting",
				// Kept while the row waits for an unlock: a reload can still ask the device what it accepted.
				resume: this.handles.get(row.target),
				actions: ["open"],
				...trayProgress(row),
			});
		} catch {
			// The tray is a mirror; the run itself is the record on this page.
		}
	}

	/* Starting, adopting a run after a reload, and the handle's controls. */

	private buildDetails(plan: DeployPlan) {
		const details: Record<string, DeployTargetDetail> = {};
		for (const target of plan.targets)
			for (const service of target.services) {
				const key = deployTarget(target.deviceId, service.serviceId);
				details[key] = {
					target: key,
					deviceId: target.deviceId,
					deviceName: target.name,
					serviceId: service.serviceId,
					kind: service.kind,
				};
			}
		this.details = details;
	}

	start() {
		const { plan, options } = this;
		if (!plan || !options || this.state) return;
		this.abort = new AbortController();
		this.memos.clear();
		this.items.clear();
		this.handles.clear();
		this.token = "";
		this.adopted = false;
		this.buildDetails(plan);
		const idle = idleState(plan, options);
		this.state = { ...idle, id: this.openTray(idle.rows) };
		this.unwatchKeys ??= this.workspace.keys.subscribe(this.onKeys);
		this.dispatch({ type: "start", at: this.at() });
	}

	/** Shows a run this window did not start (reload, another tab's tray): its items say how far each device got. */
	adopt(runId: string) {
		const { plan, options } = this;
		const run =
			plan && options ? this.workspace.activity.run(runId) : undefined;
		if (!plan || !options || !run || this.state) return;
		this.buildDetails(plan);
		const items = this.workspace.activity.runItems(runId);
		const idle = idleState(plan, options, runId);
		const rows = idle.rows.map((row) => this.adoptRow(row, items));
		const open = rows.some((row) => row.state === "held");
		this.adopted = true;
		this.announced = !open;
		this.state = {
			...idle,
			status: open ? "held" : "finished",
			shared: idle.shared ? { ...idle.shared, state: "done" } : null,
			rows,
			startedAt: seconds(run.startedAt),
			...(open ? {} : { finishedAt: seconds(run.updatedAt) }),
		};
		this.unwatchKeys ??= this.workspace.keys.subscribe(this.onKeys);
		this.unwatchTray ??= this.workspace.activity.subscribe(this.onTray);
		void this.workspace.activity.resume().catch(() => {});
		this.emit();
	}

	private adoptRow(row: DeployRunRow, items: readonly ActivityItem[]) {
		const item = items.find(
			(value) =>
				value.target.deviceId === row.deviceId &&
				value.target.serviceId === row.serviceId,
		);
		if (!item) return { ...row, state: "not_started" as const };
		this.items.set(row.target, item.id);
		const map = ADOPTED_STATE[item.state];
		return map ? map(row, item) : { ...row, state: "held" as const };
	}

	/** After a reload the tray re-reads each device; a row that nothing drives follows its item. */
	private onTray = () => {
		const state = this.state;
		if (!state || !this.adopted) return;
		const items = this.workspace.activity.runItems(state.id);
		const rows = state.rows.map((row) => {
			if (row.state !== "held" || this.inFlight.has(row.target)) return row;
			const itemId = this.items.get(row.target);
			const item = items.find((value) => value.id === itemId);
			const map = item && ADOPTED_STATE[item.state];
			return item && map ? map(row, item) : row;
		});
		if (rows.every((row, index) => row === state.rows[index])) return;
		const open = rows.some((row) => OPEN_ROW.includes(row.state));
		this.state = open
			? { ...state, rows }
			: { ...state, rows, status: "finished", finishedAt: this.at() };
		this.emit();
		this.settleFinish();
	};

	resume() {
		if (this.state?.shared?.state === "failed") {
			this.state = null;
			this.start();
			return;
		}
		this.dispatch({ type: "continue", at: this.at() });
	}

	/** Stops following: nothing more is sent, the devices keep what they have. */
	dispose() {
		this.abort.abort();
		this.unwatchKeys?.();
		this.unwatchTray?.();
		this.releaseBundle();
		this.listeners.clear();
	}
}

/* One controller per deploy and workspace. */

const runsByWorkspace = new WeakMap<DeviceWorkspace, Map<string, DeployRun>>();

function runFor(workspace: DeviceWorkspace, deploymentId: string): DeployRun {
	const runs = runsByWorkspace.get(workspace) ?? new Map<string, DeployRun>();
	runsByWorkspace.set(workspace, runs);
	const known = runs.get(deploymentId);
	if (known) return known;
	const run = new DeployRun(deploymentId, workspace);
	runs.set(deploymentId, run);
	for (const id of [...runs.keys()].slice(0, -MAX_REMEMBERED)) {
		const old = runs.get(id);
		if (old?.state && old.state.status !== "finished") continue;
		old?.dispose();
		runs.delete(id);
	}
	return run;
}

const NO_DETAILS: DeployRunDetails = {};
const noSubscription = () => () => {};
const noDetails = () => NO_DETAILS;
const noState = () => null;

/** The result details of a run (tokens, versions, what stayed on the hub); empty before it starts. */
export function useDeployRunDetails(
	deploymentId: string | undefined,
): DeployRunDetails {
	const workspace = useDeviceWorkspace();
	const run = deploymentId ? runFor(workspace, deploymentId) : null;
	return useSyncExternalStore(
		run?.subscribe ?? noSubscription,
		run?.getDetails ?? noDetails,
		noDetails,
	);
}

/**
 * A place the wizard frame may offer outside the step's own column: `foot` next
 * to Back, `headline` above the stepper. Without it (`slot` null) the step
 * renders the content in place. Put `anchor` on an element inside the step.
 */
export function useWizardSlot(name: "foot" | "headline") {
	const anchor = useRef<HTMLSpanElement>(null);
	const [slot, setSlot] = useState<Element | null>(null);
	useEffect(() => {
		const wizard = anchor.current?.closest("[data-deploy-wizard]");
		setSlot(wizard?.querySelector(`[data-deploy-${name}-slot]`) ?? null);
	});
	return { anchor, slot };
}

/** Keeps a live connection to each unlocked target while the step reads from the devices. */
export function useLiveTargets(targets: readonly PlanTarget[]) {
	const workspace = useDeviceWorkspace();
	const unlocked = targets
		.filter((target) => !target.locked)
		.map((target) => target.deviceId);
	const ids = [...new Set(unlocked)].join("|");
	useEffect(() => {
		const releases = ids
			.split("|")
			.filter(Boolean)
			.map((id) => workspace.live.acquire(id, "view"));
		return () => {
			for (const release of releases) release();
		};
	}, [workspace, ids]);
}

const notAdopted = () => false;

/** True for a run this window only follows (after a reload): the plan it shows may lack what the run began with. */
export function useDeployRunAdopted(deploymentId: string | undefined): boolean {
	const workspace = useDeviceWorkspace();
	const run = deploymentId ? runFor(workspace, deploymentId) : null;
	return useSyncExternalStore(
		run?.subscribe ?? noSubscription,
		run?.isAdopted ?? notAdopted,
		notAdopted,
	);
}

/** Marks the run as shown on this page, so its end isn't announced with a toast. */
export function useDeployRunWatch(deploymentId: string | undefined) {
	const workspace = useDeviceWorkspace();
	useEffect(() => {
		if (!deploymentId) return;
		const run = runFor(workspace, deploymentId);
		run.watchers += 1;
		return () => {
			run.watchers -= 1;
		};
	}, [workspace, deploymentId]);
}

export function useDeployRun(
	plan: DeployPlan | null,
	opts: DeployRunOptions,
): DeployRunHandle {
	const workspace = useDeviceWorkspace();
	const backend = useBackend();
	const deploymentId = plan?.draft.deploymentId;
	const run = deploymentId ? runFor(workspace, deploymentId) : null;
	const { oneAtATime, stopOnFail, resumeId, title } = opts;
	// Until the run starts it follows the plan as the draft changes.
	useEffect(() => {
		if (!run) return;
		run.backend = backend;
		if (run.state || !plan) return;
		run.plan = plan;
		run.options = { title, oneAtATime, stopOnFail, resumeId };
		if (resumeId) run.adopt(resumeId);
	});
	const live = useSyncExternalStore(
		run?.subscribe ?? noSubscription,
		run?.getState ?? noState,
		noState,
	);
	const idle = useMemo(
		() =>
			plan
				? idleState(plan, { oneAtATime, stopOnFail }, resumeId)
				: EMPTY_STATE,
		[plan, oneAtATime, stopOnFail, resumeId],
	);
	const state = live ?? idle;
	return useMemo(
		() => ({
			state,
			start: () => run?.start(),
			retry: (target) => run?.dispatch({ type: "retry", target, at: run.at() }),
			skip: (target) => run?.dispatch({ type: "skip", target, at: run.at() }),
			resume: () => run?.resume(),
			stop: () => run?.dispatch({ type: "stop", at: run.at() }),
		}),
		[state, run],
	);
}
