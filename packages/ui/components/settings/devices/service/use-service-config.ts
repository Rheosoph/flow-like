"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo } from "react";
import {
	DeploymentRejectedError,
	DeploymentReviewRequiredError,
	DeploymentRolloutEndedError,
	type DeploymentVariable,
	type PlacementConfiguration,
	StaleDeploymentRevisionError,
	discoverOfflineVariables,
	executeDeploymentPlan,
	mergeVariables,
	readExistingDeployment,
	removesOfflineBuffering,
} from "../../../../lib/device-management/deployment";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	Freshness,
	GateExtra,
	GateFailure,
	GateResult,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import { readOfflineQueues } from "../../../../lib/device-management/offline-queue";
import type {
	ManagementRejection,
	ManagementResponse,
} from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { useBackend } from "../../../../state/backend-state";
import {
	type DeviceActionOutcome,
	DeviceRejectedError,
	type GateTarget,
	type ServiceViewRead,
	deviceCall,
	inlineResultsOf,
	requestOrReject,
	serviceGateExtra,
	useActivity,
	useAttentionState,
	useDeviceAction,
	useGate,
	useGates,
	useServiceView,
} from "../workspace";
import {
	type ApplyHow,
	ConfigTooLargeError,
	type PlacementConfig,
	boardDefinitions,
	settingsPlan,
} from "./config-model";

/* The live reads and writes behind Configuration, Endpoint and Write buffering. */

const CONFIG_EVERY_MS = 15_000;

/** The device's code for "this account may not do that". */
export const ACCESS_REFUSED = "unauthorized";

export const serviceConfigKey = (
	scopeKey: string,
	deviceId: string,
	serviceId: string,
) => ["devices", scopeKey, "service-config", deviceId, serviceId] as const;

interface ConfigData {
	configuration: PlacementConfiguration;
	/** Unix seconds, hub-corrected. */
	readAt: number;
}

export interface ServiceConfigRead {
	device: DeviceViewModel | undefined;
	deviceLabel: string;
	service: ServiceView | undefined;
	/** Why the service's row isn't known at all. */
	unavailable: ServiceViewRead["unavailable"];
	/** Why its settings can't be read from the device right now; `null` while a live read is possible. */
	gate: GateFailure | null;
	configuration: PlacementConfiguration | undefined;
	readAt: number | undefined;
	loading: boolean;
	/** The device answered that this account may not read the settings. */
	refused: ManagementRejection | undefined;
	/** The read failed for another reason; settings read earlier stay. */
	failed: boolean;
	/** The device's own sentence when it turned the read down for that other reason. */
	failure?: string;
	freshness: Freshness;
	/** Reads the settings again; resolves with what the device holds now. */
	refresh(): Promise<PlacementConfiguration | undefined>;
}

/**
 * What other screens derive from a service's settings (endpoint, certificate,
 * buffering, approval). The read is complete, so a fact the settings no longer
 * carry goes: an endpoint that was removed, buffering that was turned off.
 */
function reportFacts(
	workspace: DeviceWorkspace,
	deviceId: string,
	serviceId: string,
	{ config }: PlacementConfiguration,
) {
	if (workspace.keys.snapshot(deviceId).state !== "unlocked") return;
	const known = workspace.facts.get(deviceId)?.placements?.[serviceId];
	const writes = config.offline_writes;
	const next = {
		...(config.hosting
			? { host: config.hosting.host, port: config.hosting.port }
			: {}),
		tlsCertificateId: config.tls_certificate_id ?? null,
		...(writes
			? {
					offlineWrites: {
						maxAgeS: writes.max_age_seconds,
						maxBytes: writes.max_queue_bytes,
					},
				}
			: {}),
		resourceGrantId: config.resource_grant?.grant_id ?? null,
	};
	if (JSON.stringify(next) === JSON.stringify(known)) return;
	workspace.facts.record(deviceId, { placements: { [serviceId]: next } });
}

function freshnessOf(
	data: ConfigData | undefined,
	live: boolean,
	failed: boolean,
): Freshness {
	if (!data) return { src: "live", age: "notloaded" };
	if (failed) return { src: "live", age: "error", dataFrom: data.readAt };
	return live
		? { src: "live", age: "live", at: data.readAt, cadenceS: 15 }
		: { src: "live", age: "lastknown", at: data.readAt };
}

/**
 * The service's settings, read live (`placement_configuration`) and re-read
 * every 15 s while a view is open. One read serves all three tabs.
 */
export function useServiceConfig(
	deviceId: string,
	serviceId: string,
): ServiceConfigRead {
	const { workspace } = useAttentionState();
	const queryClient = useQueryClient();
	const { device, service, unavailable } = useServiceView(deviceId, serviceId);
	const projectId = service?.projectId;
	const target = useMemo<GateTarget>(
		() => ({
			placementId: serviceId,
			...(projectId ? { projectId } : {}),
			labels: { service: serviceId },
		}),
		[serviceId, projectId],
	);
	const gate = useGate("set_secret", deviceId, target);
	const unlocked = device?.keys.state === "unlocked";
	const queryKey = serviceConfigKey(workspace.scopeKey, deviceId, serviceId);
	const query = useQuery({
		queryKey,
		queryFn: async (): Promise<ConfigData> => {
			const response = await requestOrReject(
				deviceCall(workspace, deviceId, "poll"),
				{ type: "placement_configuration", placement_id: serviceId },
			);
			const configuration = await readExistingDeployment(
				async () => response,
				serviceId,
				projectId ?? "",
			);
			reportFacts(workspace, deviceId, serviceId, configuration);
			return {
				configuration,
				readAt: Math.floor(workspace.clock.now() / 1000),
			};
		},
		enabled: gate.ok && !!projectId,
		staleTime: CONFIG_EVERY_MS,
		refetchInterval: CONFIG_EVERY_MS,
		// Nothing watches the keys once the last settings tab closed, so the settings go with it.
		gcTime: 0,
		retry: false,
		meta: { persist: false },
	});

	// Decrypted settings never outlive the keys they were read with.
	useEffect(() => {
		if (unlocked) return;
		queryClient.removeQueries({
			queryKey: serviceConfigKey(workspace.scopeKey, deviceId, serviceId),
		});
	}, [unlocked, queryClient, workspace.scopeKey, deviceId, serviceId]);

	const { data, error, isLoading, refetch } = query;
	const refresh = useCallback(
		async () => (await refetch()).data?.configuration,
		[refetch],
	);
	return useMemo(() => {
		const rejection =
			error instanceof DeviceRejectedError ? error.rejection : undefined;
		// Only "you may not" is a matter of access; a busy or failing device is a failed read.
		const refused = rejection?.code === ACCESS_REFUSED ? rejection : undefined;
		const failed = !!error && !refused;
		// Settings read before the device withdrew the access don't stay on screen.
		const shown = unlocked && !refused ? data : undefined;
		return {
			device,
			deviceLabel: device ? deviceName(device.row) : deviceId.slice(0, 8),
			service,
			unavailable,
			gate: gate.ok ? null : gate,
			configuration: shown?.configuration,
			readAt: shown?.readAt,
			loading: isLoading,
			refused,
			failed,
			...(failed && rejection?.error ? { failure: rejection.error } : {}),
			freshness: freshnessOf(shown, gate.ok, failed),
			refresh,
		};
	}, [
		device,
		deviceId,
		service,
		unavailable,
		gate,
		unlocked,
		data,
		error,
		isLoading,
		refresh,
	]);
}

/* Variable names and types of the version the service runs. */

export interface DefinitionsRead {
	/** `undefined` while unknown: names then fall back and nothing is flagged. */
	definitions: DeploymentVariable[] | undefined;
	loading: boolean;
}

type Pins = PlacementConfig["events"];

/**
 * The variables the served events define. An offline copy is described by the
 * device; an online service by the flows its events are pinned to.
 */
export function useVariableDefinitions(
	deviceId: string,
	configuration: PlacementConfiguration | undefined,
	live: boolean,
): DefinitionsRead {
	const { workspace } = useAttentionState();
	const backend = useBackend();
	const config = configuration?.config;
	const source = config?.source;
	const pins = JSON.stringify(config?.events ?? []);
	const query = useQuery({
		queryKey: [
			"devices",
			workspace.scopeKey,
			"service-definitions",
			deviceId,
			config?.id ?? "",
			config?.revision ?? "",
			pins,
		],
		queryFn: async (): Promise<DeploymentVariable[]> => {
			if (!config) return [];
			const events = JSON.parse(pins) as Pins;
			if (config.source === "offline") {
				const call = deviceCall(workspace, deviceId, "poll");
				const installed = {
					project_id: config.project_id,
					project_path: config.project_path,
					revision: config.revision,
					source: "offline" as const,
				};
				const groups: DeploymentVariable[][] = [];
				for (const event of events)
					groups.push(
						await discoverOfflineVariables(call, installed, event.event_id),
					);
				return mergeVariables(groups);
			}
			const groups: DeploymentVariable[][] = [];
			for (const pin of events) {
				const event = await backend.eventState.getEventAuthoritative(
					config.project_id,
					pin.event_id,
					pin.event_version,
				);
				const board = await backend.boardState.getBoardAuthoritative(
					config.project_id,
					event.board_id,
					pin.board_version,
				);
				groups.push(boardDefinitions(board));
			}
			return mergeVariables(groups);
		},
		enabled: !!config && (source === "online" || live),
		staleTime: Number.POSITIVE_INFINITY,
		retry: false,
		meta: { persist: false },
	});
	return useMemo(
		() => ({ definitions: query.data, loading: query.isLoading }),
		[query.data, query.isLoading],
	);
}

/* Gates of the settings actions. */

export const SETTINGS_ACTIONS = [
	"update_service",
	"update_with_checks",
	"advanced_config",
	"change_tls",
] as const;
export type SettingsAction = (typeof SETTINGS_ACTIONS)[number];

const ACTIVE_ROLLOUT = new Set([
	"staged",
	"validating",
	"activating",
	"rolling_back",
]);

export interface SettingsGates {
	gates: Record<SettingsAction, GateResult>;
	/** Writing a secret value; allowed while the service runs. */
	secret: GateResult;
	target: GateTarget;
	/** An update holds the settings: the device refuses changes and secret writes until it ends. */
	rollout: { staged: boolean; deadlineAt?: number } | null;
	pendingSecret: boolean;
}

/** `service` and the settings as read decide what each settings action may do now. */
export function useSettingsGates(
	deviceId: string,
	serviceId: string,
	service: ServiceView | undefined,
	configuration: PlacementConfiguration | undefined,
): SettingsGates {
	const { input } = useAttentionState();
	const secrets = useActivity({ deviceId, serviceId, kind: "secret_write" });
	const pendingSecret = secrets.inProgress.length > 0;
	const hostOperation =
		input.live[deviceId]?.inspection?.value.hostOperation?.state;
	const config = configuration?.config;
	const source = config?.source ?? service?.source ?? undefined;
	const profile = config?.resources?.profile ?? "trusted_process";
	const sources = configuration?.rollout_sources;
	const target = useMemo<GateTarget>(() => {
		const { desiredState: _desired, ...base } = serviceGateExtra(
			service,
			hostOperation,
		);
		const desired = configuration?.desired_state ?? service?.desired;
		const extra: GateExtra = {
			...base,
			...(desired ? { desiredState: desired } : {}),
			...(source ? { source } : {}),
			sameSource: true,
			isolationProfile: profile,
			pendingSecret,
		};
		return {
			placementId: serviceId,
			...(service ? { projectId: service.projectId } : {}),
			labels: { service: serviceId },
			extra,
			...(sources ? { features: { rolloutSources: sources } } : {}),
		};
	}, [
		service,
		hostOperation,
		configuration?.desired_state,
		source,
		profile,
		pendingSecret,
		serviceId,
		sources,
	]);
	const gates = useGates(SETTINGS_ACTIONS, deviceId, target);
	// A secret write needs no stopped service: the device stores it under its name while the service runs.
	const secretTarget = useMemo<GateTarget>(
		() => ({
			placementId: serviceId,
			...(service ? { projectId: service.projectId } : {}),
			labels: { service: serviceId },
		}),
		[serviceId, service],
	);
	const secretGate = useGate("set_secret", deviceId, secretTarget);
	return useMemo(() => {
		const state = configuration?.rollout?.state ?? service?.rollout?.state;
		const active =
			(state !== undefined && ACTIVE_ROLLOUT.has(state)) ||
			target.extra?.activeRollout === true;
		const deadlineAt = target.extra?.rolloutDeadlineAt;
		return {
			gates,
			secret: secretGate,
			target,
			rollout: active
				? {
						staged: state === "staged",
						...(deadlineAt ? { deadlineAt } : {}),
					}
				: null,
			pendingSecret,
		};
	}, [gates, secretGate, target, configuration, service, pendingSecret]);
}

/* Applying a settings change. */

export class QueueNotEmptyError extends Error {
	constructor(readonly pending: number) {
		super(`${pending} buffered changes still wait.`);
		this.name = "QueueNotEmptyError";
	}
}

export type ApplyOutcome =
	| { status: "done"; revision: number }
	| { status: "gated"; gate: GateFailure }
	| { status: "busy" | "cancelled" | "unknown" }
	/** The settings changed on the device since they were read. */
	| { status: "stale" }
	| { status: "rolled_back" | "update_failed" | "update_cancelled" }
	| { status: "review" }
	| { status: "queue_waiting"; pending: number }
	| { status: "too_large"; bytes: number }
	/** The device's own sentence. */
	| { status: "refused"; reason: string }
	| { status: "failed" };

export interface ApplyRequest {
	existing: PlacementConfiguration;
	config: PlacementConfig;
	how: ApplyHow;
	/** A safe update: the device took the new settings and is now checking them. */
	onStarted?(): void;
}

export interface SettingsApply {
	apply(request: ApplyRequest): Promise<ApplyOutcome>;
	pending: boolean;
	resultKey: string;
}

export const settingsResultKey = (deviceId: string, serviceId: string) =>
	`service-config:${deviceId}/${serviceId}`;

const ENDED: Record<string, "rolled_back" | "update_cancelled"> = {
	rolled_back: "rolled_back",
	cancelled: "update_cancelled",
};

/** What each typed failure of the plan means for the person; the first match wins. */
const FAILURES: readonly ((error: unknown) => ApplyOutcome | undefined)[] = [
	(error) =>
		error instanceof StaleDeploymentRevisionError
			? { status: "stale" }
			: undefined,
	(error) =>
		error instanceof DeploymentRolloutEndedError
			? { status: ENDED[error.status.state] ?? "update_failed" }
			: undefined,
	(error) =>
		error instanceof DeploymentReviewRequiredError
			? { status: "review" }
			: undefined,
	(error) =>
		error instanceof QueueNotEmptyError
			? { status: "queue_waiting", pending: error.pending }
			: undefined,
	(error) =>
		error instanceof ConfigTooLargeError
			? { status: "too_large", bytes: error.bytes }
			: undefined,
	(error) =>
		error instanceof DeploymentRejectedError
			? { status: "refused", reason: error.rejection.error }
			: undefined,
];

function failureOutcome(error: unknown): ApplyOutcome {
	for (const match of FAILURES) {
		const outcome = match(error);
		if (outcome) return outcome;
	}
	return { status: "failed" };
}

function outcomeOf(outcome: DeviceActionOutcome<number>): ApplyOutcome {
	switch (outcome.status) {
		case "done":
			return { status: "done", revision: outcome.result };
		case "gated":
			return { status: "gated", gate: outcome.gate };
		case "rejected":
			return { status: "refused", reason: outcome.rejection.error };
		case "failed":
			return failureOutcome(outcome.error);
		default:
			return { status: outcome.status };
	}
}

/**
 * One settings change, through the action layer: gate, the commands of
 * `settingsPlan` with the revision the settings were read at, a tray item and
 * the re-read of the settings. Buffering is never removed while changes wait.
 */
export function useSettingsApply(
	deviceId: string,
	serviceId: string,
	gates: SettingsGates,
	deviceLabel: string,
): SettingsApply {
	const { t } = useTranslation("devices");
	const { workspace } = useAttentionState();
	const actions = useDeviceAction();
	const resultKey = settingsResultKey(deviceId, serviceId);
	const { target } = gates;
	const apply = useCallback(
		async ({
			existing,
			config,
			how,
			onStarted,
		}: ApplyRequest): Promise<ApplyOutcome> => {
			const running =
				(existing.desired_state ?? target.extra?.desiredState) === "running";
			let plan: ReturnType<typeof settingsPlan>;
			try {
				plan = settingsPlan(existing, config, how, running);
			} catch (error) {
				return failureOutcome(error);
			}
			const safe = how.mode === "safe";
			const outcome = await actions.run<number>({
				action: safe ? "update_with_checks" : "update_service",
				deviceId,
				target,
				label: safe
					? t("serviceConfig.apply.safeLabel", "Safe update of {{service}}", {
							service: serviceId,
						})
					: t("serviceConfig.apply.quickLabel", "Quick update of {{service}}", {
							service: serviceId,
						}),
				resultKey,
				lane: "operation",
				call: async (context) => {
					if (
						removesOfflineBuffering(
							existing.config.offline_writes,
							config.offline_writes,
						)
					) {
						const queues = await readOfflineQueues(context.call, serviceId);
						const pending = queues.reduce(
							(sum, queue) => sum + queue.pending_count,
							0,
						);
						if (pending > 0) throw new QueueNotEmptyError(pending);
					}
					if (plan.rollout_id)
						context.track({
							resume: {
								type: "rollout",
								rolloutId: plan.rollout_id,
								placementId: serviceId,
								projectId: existing.project_id,
							},
							deadlineAt: workspace.clock.now() + how.deadlineS * 1000,
						});
					const progress = { started: false };
					await executeDeploymentPlan(context.call, plan, undefined, () => {
						if (progress.started) return;
						progress.started = true;
						onStarted?.();
					});
					return existing.config_revision + 1;
				},
				activity: {
					kind: safe ? "safe_update" : "command",
					params: { command: "apply" },
					deviceName: deviceLabel,
					serviceId,
					projectId: existing.project_id,
					href: { screen: "service", deviceId, serviceId, tab: "status" },
				},
				invalidate: [serviceConfigKey(workspace.scopeKey, deviceId, serviceId)],
			});
			const result = outcomeOf(outcome);
			// Every outcome but "no reply" gets its own sentence next to the control.
			if (result.status !== "unknown" && result.status !== "busy")
				inlineResultsOf(workspace).clear(resultKey);
			if (result.status === "done" || result.status === "stale")
				void workspace.live.refreshInspection(deviceId);
			return result;
		},
		[
			actions,
			workspace,
			deviceId,
			serviceId,
			target,
			resultKey,
			deviceLabel,
			t,
		],
	);
	return useMemo(
		() => ({ apply, pending: actions.pending(resultKey), resultKey }),
		[apply, actions, resultKey],
	);
}

/* Secret writes (S26): variable secrets and the endpoint token. */

export interface SecretWriteInput {
	/** The stored secret's reference in the settings; the value replaces it in place. */
	reference: string;
	/** What people call it ("ERP password", "Access token"). */
	label: string;
	value: string;
	/** The settings revision the reference was read at. */
	revision: number;
}

export interface SecretWrite {
	write(
		input: SecretWriteInput,
	): Promise<DeviceActionOutcome<ManagementResponse>>;
	pending: boolean;
	resultKey: string;
}

export const secretResultKey = (deviceId: string, serviceId: string) =>
	`service-secret:${deviceId}/${serviceId}`;

const SECRET_POLLS = 40;
const SECRET_POLL_MS = 500;

const secretState = (response: ManagementResponse) =>
	response.state === "failed" || response.result.secret === "failed"
		? "failed"
		: response.state === "completed" && response.result.secret === "completed"
			? "done"
			: undefined;

/** Resolves when the device saved the value; a device that stays silent leaves the tray item waiting with "Check result". */
async function secretSaved(
	workspace: DeviceWorkspace,
	deviceId: string,
	first: ManagementResponse,
): Promise<"done" | "failed"> {
	const known = secretState(first);
	if (known) return known;
	const call = deviceCall(workspace, deviceId, "poll");
	for (let attempt = 0; attempt < SECRET_POLLS; attempt++) {
		await new Promise((resolve) => setTimeout(resolve, SECRET_POLL_MS));
		const state = secretState(
			await call({ type: "operation", operation_id: first.operation_id }),
		);
		if (state) return state;
	}
	return new Promise(() => undefined);
}

/** Sends one secret value inside the encrypted live session; it is never part of a hub request, a log or the tray. */
export function useSecretWrite(
	deviceId: string,
	serviceId: string,
	service: ServiceView | undefined,
	deviceLabel: string,
): SecretWrite {
	const { t } = useTranslation("devices");
	const { workspace } = useAttentionState();
	const actions = useDeviceAction();
	const resultKey = secretResultKey(deviceId, serviceId);
	const projectId = service?.projectId;
	const write = useCallback(
		({ reference, label, value, revision }: SecretWriteInput) =>
			actions.run<ManagementResponse>({
				action: "set_secret",
				deviceId,
				target: {
					placementId: serviceId,
					...(projectId ? { projectId } : {}),
					labels: { service: serviceId },
				},
				label: t("serviceConfig.secret.actionLabel", "New value for {{name}}", {
					name: label,
				}),
				resultKey,
				call: (context) =>
					context.request(
						{
							type: "set_secret",
							placement_id: serviceId,
							expected_revision: revision,
							name: reference,
							value,
						},
						crypto.randomUUID(),
					),
				activity: {
					kind: "secret_write",
					params: { name: label },
					deviceName: deviceLabel,
					serviceId,
					...(projectId ? { projectId } : {}),
					href: {
						screen: "service",
						deviceId,
						serviceId,
						tab: "configuration",
					},
					resume: (response) => ({
						type: "secret",
						operationId: response.operation_id,
						placementId: serviceId,
						name: label,
					}),
					settle: (response) => secretSaved(workspace, deviceId, response),
				},
			}),
		[
			actions,
			workspace,
			deviceId,
			serviceId,
			projectId,
			deviceLabel,
			resultKey,
			t,
		],
	);
	return useMemo(
		() => ({ write, pending: actions.pending(resultKey), resultKey }),
		[write, actions, resultKey],
	);
}
