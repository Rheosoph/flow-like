"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toHubError } from "../../../../lib/device-management/hub/endpoints";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type {
	GateExtra,
	GateFailure,
} from "../../../../lib/device-management/model/types";
import type {
	ReleaseConfig,
	ReleaseTarget,
	VerifiedRelease,
} from "../../../../lib/device-management/package";
import type {
	AccountBackupOutcome,
	DeviceSetupInput,
	prepareDevicePackage,
} from "../../../../lib/device-management/setup";
import type { DeviceStoragePersistence } from "../../../../lib/device-management/storage";
import type { OnboardingManifest } from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { IApiState } from "../../../../state/backend-state/api-state";
import { useDeviceAction, useDeviceWorkspace } from "../workspace";
import type { BackupOutcome, SetupMode } from "./setup-state";

/** The one call that makes the keys, registers the device and builds the package. */
export type PrepareSetup = typeof prepareDevicePackage;

export const CREATE_PHASES = [
	"keys",
	"register",
	"download",
	"build",
	"backup",
] as const;
export type CreatePhase = (typeof CREATE_PHASES)[number];

export interface Registration {
	enrollmentId: string;
	deviceId: string;
	/** Unix seconds, as the hub signed them. */
	createdAt: number;
	expiresAt: number;
}

export type SetupFailureKind =
	/** The hub's limits: devices, unused setup packages or packages per day. */
	| "limit"
	| "hub_refused"
	| "hub_unreachable"
	/** The agent could not be fetched or did not match the signed release. */
	| "download"
	/** Keys, storage or packaging failed on this computer. */
	| "local"
	/** It failed and the hub kept the reservation. */
	| "cancel_failed"
	| "gated";

export interface SetupFailure {
	kind: SetupFailureKind;
	phase: CreatePhase;
	/** The hub's own sentence (refusals) or the client's sentence, for "Details". */
	detail?: string;
	/** Seconds the hub asked to wait. */
	retryAfterS?: number;
	gate?: GateFailure;
}

export interface CreateRun {
	status: "idle" | "running" | "failed" | "done";
	/** The phase that is running, or the one that failed. */
	phase: CreatePhase;
	/** Epoch milliseconds each finished phase ended. */
	doneAt: Partial<Record<CreatePhase, number>>;
	registration?: Registration;
	failure?: SetupFailure;
}

export interface BuiltPackage {
	packageUrl: string;
	packageBytes: number;
	backupUrl: string;
	backupBytes: number;
	storage: DeviceStoragePersistence;
}

export interface CreateRequest {
	name: string;
	password: string;
	target: ReleaseTarget;
	mode: SetupMode;
	backup: boolean;
	/** The agent is packed here (not deferred to the device, not Docker only). */
	packsAgent: boolean;
	release: ReleaseConfig;
	verified: VerifiedRelease;
	gateExtra: GateExtra;
}

export interface CreatedResult {
	registration: Registration | undefined;
	deviceId: string;
	outcome: BackupOutcome;
	/** Unix seconds. */
	finishedAt: number;
}

export interface SetupCreate {
	run: CreateRun;
	/** The files of this window; gone after a reload or once the wizard closes. */
	built: BuiltPackage | undefined;
	/** A password is held for a retry. */
	canRetry: boolean;
	start(request: CreateRequest): Promise<CreatedResult | undefined>;
	/** Runs the held request again, with the release as it is verified now. */
	retry(
		current: Pick<CreateRequest, "release" | "verified">,
	): Promise<CreatedResult | undefined>;
	/** Stops a running creation; the hub reservation is released by the run itself. */
	abort(): void;
	/** The stopped run's setup could not be cancelled at the hub: nothing runs any more, and the hub still holds it. */
	cancelFailed(): void;
	/** What the hub registered in the current run, read at call time (state may lag behind an abort). */
	registered(): Registration | undefined;
	/** Forgets the run, its password and its files. */
	reset(): void;
	/** Resolves once no creation is running. */
	settled(): Promise<void>;
}

const IDLE: CreateRun = { status: "idle", phase: "keys", doneAt: {} };
export const SETUP_CREATE_KEY = "setup:create";

interface ApiObserver {
	registering(): void;
	registered(manifest: OnboardingManifest | undefined): void;
	backingUp(): void;
}

/** The same hub client, reporting where the run is: the lib has no progress callback. */
function observedApi(api: IApiState, observer: ApiObserver): IApiState {
	const fetch: IApiState["fetch"] = async (profile, path, options) => {
		const enroll = path === "devices/enrollments" && options?.method === "POST";
		if (enroll) observer.registering();
		else if (path.startsWith("devices/controller-vaults")) observer.backingUp();
		const result = await api.fetch(profile, path, options);
		if (enroll)
			observer.registered(
				(result as { manifest?: OnboardingManifest } | null)?.manifest,
			);
		return result as never;
	};
	return new Proxy(api, {
		get(target, key) {
			if (key === "fetch") return fetch;
			const value = Reflect.get(target, key);
			return typeof value === "function" ? value.bind(target) : value;
		},
	});
}

const HUB_PHASES: ReadonlySet<CreatePhase> = new Set([
	"keys",
	"register",
	"backup",
]);
const UNREACHABLE = new Set(["network", "timeout", "server_error"]);

function failureOf(error: unknown, phase: CreatePhase): SetupFailure {
	if (error === undefined) return { kind: "local", phase };
	const message = error instanceof Error ? error.message : String(error);
	if (/could not be cancelled/i.test(message))
		return { kind: "cancel_failed", phase };
	const hub = toHubError(error);
	if (hub.code === "rate_limited")
		return {
			kind: "limit",
			phase,
			detail: hub.message,
			...(hub.retryAfterS === undefined
				? {}
				: { retryAfterS: hub.retryAfterS }),
		};
	if (!HUB_PHASES.has(phase))
		return { kind: "download", phase, detail: message };
	if (UNREACHABLE.has(hub.code)) return { kind: "hub_unreachable", phase };
	return hub.status === undefined
		? { kind: "local", phase, detail: message }
		: { kind: "hub_refused", phase, detail: hub.message };
}

function registrationOf(
	manifest: OnboardingManifest | undefined,
): Registration | undefined {
	if (!manifest?.enrollment_id || !manifest.device_id) return undefined;
	return {
		enrollmentId: manifest.enrollment_id,
		deviceId: manifest.device_id,
		createdAt: manifest.issued_at,
		expiresAt: manifest.expires_at,
	};
}

/**
 * The tray item of a setup: its handle is also this computer's record of the
 * pending setup on older hubs. Its deadline is the time the package has to be
 * started by: the setup's own end, or the earlier end of the agent release.
 */
function trackSetup(
	workspace: DeviceWorkspace,
	registration: Registration,
	name: string,
	releaseEndsAt: number,
): string {
	const { enrollmentId, deviceId, expiresAt } = registration;
	return workspace.activity.start({
		kind: "setup",
		target: { deviceId, deviceName: name },
		state: "active",
		label: { code: "setup" },
		progress: "indeterminate",
		deadlineAt: Math.min(expiresAt, releaseEndsAt) * 1000,
		startedBy: "you",
		resume: { type: "setup", enrollmentId, deviceId },
		actions: [],
		href: { screen: "setup", enrollmentId },
	});
}

/** The tray item that tracks a setup, also one made by an earlier window. */
export function setupActivityId(
	workspace: DeviceWorkspace,
	enrollmentId: string | undefined,
): string | undefined {
	if (!enrollmentId) return undefined;
	return workspace.activity
		.list()
		.find(
			(item) =>
				item.resume?.type === "setup" &&
				item.resume.enrollmentId === enrollmentId,
		)?.id;
}

/**
 * A setup that was cancelled leaves no tray entry: the wizard states the
 * outcome, and without its handle nothing keeps asking the hub about it.
 */
export function dropSetupActivity(workspace: DeviceWorkspace, itemId: string) {
	workspace.activity.update(itemId, {
		resume: undefined,
		deadlineAt: undefined,
	});
	workspace.activity.dismiss(itemId);
}

/** The device checked in: the tray entry is done and now leads to the device. */
export function finishSetupActivity(
	workspace: DeviceWorkspace,
	itemId: string,
	deviceId: string,
) {
	if (deviceId)
		workspace.activity.update(itemId, {
			href: { screen: "device", deviceId, tab: "overview" },
		});
	workspace.activity.finish(itemId, "done", { code: "done" });
}

interface Active {
	controller: AbortController;
	done: Promise<unknown>;
}

/**
 * Step 4 of the setup. Leaving the wizard aborts a running creation and drops
 * its files: a completion that arrives afterwards changes nothing here.
 */
export function useSetupCreate(prepare: PrepareSetup): SetupCreate {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const [run, setRun] = useState<CreateRun>(IDLE);
	const [built, setBuilt] = useState<BuiltPackage>();
	const [canRetry, setCanRetry] = useState(false);
	const alive = useRef(true);
	const active = useRef<Active | undefined>(undefined);
	const urls = useRef<string[]>([]);
	const held = useRef<CreateRequest | undefined>(undefined);
	const registration = useRef<Registration | undefined>(undefined);

	const dropFiles = useCallback(() => {
		for (const url of urls.current) URL.revokeObjectURL(url);
		urls.current = [];
	}, []);

	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
			active.current?.controller.abort();
			held.current = undefined;
			dropFiles();
		};
	}, [dropFiles]);

	const execute = useCallback(
		async (request: CreateRequest): Promise<CreatedResult | undefined> => {
			if (active.current) return undefined;
			const controller = new AbortController();
			const current = () => alive.current && !controller.signal.aborted;
			const state: CreateRun = { status: "running", phase: "keys", doneAt: {} };
			let itemId: string | undefined;
			const publish = () => {
				if (current()) setRun({ ...state, doneAt: { ...state.doneAt } });
			};
			const advance = (next: CreatePhase, ...ended: CreatePhase[]) => {
				const now = Date.now();
				for (const phase of ended) state.doneAt[phase] ??= now;
				state.phase = next;
				publish();
			};
			const observer: ApiObserver = {
				registering: () => advance("register", "keys"),
				registered: (manifest) => {
					state.registration = registrationOf(manifest);
					registration.current = state.registration;
					// Tracked even after the wizard closed: the hub now holds a setup that an older hub lists nowhere.
					if (state.registration)
						itemId = trackSetup(
							workspace,
							state.registration,
							request.name,
							request.verified.manifest.expires_at,
						);
					advance(request.packsAgent ? "download" : "build", "register");
				},
				backingUp: () => {
					if (state.phase !== "backup") advance("backup", "download", "build");
				},
			};
			held.current = request;
			registration.current = undefined;
			setCanRetry(false);
			setBuilt(undefined);
			dropFiles();
			publish();

			const scope = workspace.scopeKey;
			const work = actions.run({
				action: "setup_device",
				label: t("setup.create.action", "Create setup package"),
				target: { extra: request.gateExtra },
				resultKey: SETUP_CREATE_KEY,
				call: ({ workspace: target }) => {
					const input: DeviceSetupInput = {
						api: observedApi(target.deps.api, observer),
						profile: target.deps.profile,
						scope: target.deps.scope,
						name: request.name,
						password: request.password,
						target: request.target,
						mode: request.mode,
						release: request.release,
						verifiedRelease: request.verified,
						signal: controller.signal,
						backupToAccount: request.backup,
					};
					return prepare(input);
				},
				invalidate: [
					deviceKeys.list(scope),
					deviceKeys.enrollments(scope, "open"),
					deviceKeys.enrollments(scope, "recent"),
					deviceKeys.usage(scope),
					deviceKeys.accountBackups(scope),
				],
			});
			active.current = { controller, done: work.catch(() => undefined) };
			try {
				const outcome = await work;
				if (outcome.status === "busy") {
					// An earlier run still holds the control: nothing started, so nothing may read as running.
					if (current()) setRun(IDLE);
					return undefined;
				}
				if (outcome.status !== "done") {
					const failure: SetupFailure =
						outcome.status === "gated"
							? { kind: "gated", phase: state.phase, gate: outcome.gate }
							: failureOf(
									"error" in outcome ? outcome.error : undefined,
									state.phase,
								);
					if (itemId && controller.signal.aborted)
						dropSetupActivity(workspace, itemId);
					else if (itemId)
						workspace.activity.finish(itemId, "failed", { code: "failed" });
					if (current()) {
						setCanRetry(true);
						setRun({
							status: "failed",
							phase: state.phase,
							doneAt: { ...state.doneAt },
							...(failure.kind === "cancel_failed" && state.registration
								? { registration: state.registration }
								: {}),
							failure,
						});
					}
					return undefined;
				}
				const prepared = outcome.result;
				const now = Date.now();
				for (const phase of CREATE_PHASES) state.doneAt[phase] ??= now;
				const backup: AccountBackupOutcome | undefined = prepared.accountBackup;
				const result: CreatedResult = {
					registration: state.registration,
					deviceId: prepared.deviceId,
					outcome: request.backup ? (backup ?? "local_only") : "off",
					finishedAt: Math.floor(now / 1000),
				};
				if (itemId)
					workspace.activity.update(itemId, {
						state: "waiting",
						detail: { code: "waiting_for_heartbeat" },
						actions: ["check_again"],
					});
				void workspace.local.reload().catch(() => undefined);
				if (!current()) return undefined;
				const packageUrl = URL.createObjectURL(prepared.package);
				const backupUrl = URL.createObjectURL(prepared.backup);
				urls.current.push(packageUrl, backupUrl);
				held.current = undefined;
				setBuilt({
					packageUrl,
					packageBytes: prepared.package.size,
					backupUrl,
					backupBytes: prepared.backup.size,
					storage: prepared.storage,
				});
				setRun({
					status: "done",
					phase: "backup",
					doneAt: { ...state.doneAt },
					...(state.registration ? { registration: state.registration } : {}),
				});
				return result;
			} finally {
				active.current = undefined;
			}
		},
		[actions, dropFiles, prepare, t, workspace],
	);

	const retry = useCallback(
		(current: Pick<CreateRequest, "release" | "verified">) =>
			held.current
				? execute({ ...held.current, ...current })
				: Promise.resolve(undefined),
		[execute],
	);

	const abort = useCallback(() => active.current?.controller.abort(), []);

	const cancelFailed = useCallback(() => {
		if (active.current || !alive.current) return;
		const kept = registration.current;
		// An aborted run publishes nothing, so its last state still says "running".
		setRun((current) =>
			current.status === "running"
				? {
						status: "failed",
						phase: current.phase,
						doneAt: current.doneAt,
						...(kept ? { registration: kept } : {}),
						failure: { kind: "cancel_failed", phase: current.phase },
					}
				: current,
		);
	}, []);

	const reset = useCallback(() => {
		active.current?.controller.abort();
		held.current = undefined;
		registration.current = undefined;
		dropFiles();
		setCanRetry(false);
		setBuilt(undefined);
		setRun(IDLE);
	}, [dropFiles]);

	const settled = useCallback(async () => {
		await active.current?.done;
	}, []);

	const registered = useCallback(() => registration.current, []);

	return useMemo(
		() => ({
			run,
			built,
			canRetry,
			start: execute,
			retry,
			abort,
			cancelFailed,
			registered,
			reset,
			settled,
		}),
		[
			run,
			built,
			canRetry,
			execute,
			retry,
			abort,
			cancelFailed,
			registered,
			reset,
			settled,
		],
	);
}
