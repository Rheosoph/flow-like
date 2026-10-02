"use client";

import { useQueries, useQuery } from "@tanstack/react-query";
import {
	type ReactNode,
	createContext,
	createElement,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import type {
	CertificateInventory,
	PublicCertificateInventory,
} from "../../../../lib/device-management/certificates";
import {
	type HubResult,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import {
	hubDeviceSupport,
	queries,
	releaseConfigOf,
} from "../../../../lib/device-management/hub/queries";
import {
	type AttentionCounts,
	type AttentionFilter,
	computeAttention,
	countAttention,
	filterAttention,
} from "../../../../lib/device-management/model/attention";
import { classify } from "../../../../lib/device-management/model/freshness";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import { relationshipOf } from "../../../../lib/device-management/model/presence";
import type {
	AccountBackupList,
	ActionId,
	ArchiveUsage,
	AttentionActionCode,
	AttentionInput,
	AttentionItem,
	DeviceRow,
	DeviceUsageResponse,
	FleetCertificateInventoryRow,
	Freshness,
	GateContext,
	GateExtra,
	GateFeatures,
	GateResult,
	GrantedCapabilities,
	HubDeviceSupport,
	HubEnrollment,
	LiveDeviceInput,
	MyAccess,
	PendingSetup,
	ResourceSummary,
	SourcePlane,
} from "../../../../lib/device-management/model/types";
import type { OfflineQueueStatus } from "../../../../lib/device-management/offline-queue";
import type { DeviceSetupReadiness } from "../../../../lib/device-management/readiness";
import type {
	DeviceCrypto,
	ManagementPolicy,
	PolicyView,
} from "../../../../lib/device-management/types";
import { isActivityFinished } from "../../../../lib/device-management/workspace/activity";
import type {
	DeviceWorkspace,
	LiveStreams,
	StreamSpec,
	StreamState,
} from "../../../../lib/device-management/workspace/types";
import type { VerifiedRelease } from "../../../../lib/device-package";
import type { DeviceResources } from "../../../../lib/device-resources";

/** IA §6.5 dwell conditions ("for > 2 min") are re-checked on this tick, never on the 1 s clock. */
export const DWELL_TICK_MS = 30_000;

/** Fleet-wide hub reads the attention engine takes; everything is optional (older hub, not loaded). */
export interface AttentionHubData {
	hub: HubDeviceSupport;
	rows?: DeviceRow[];
	readiness?: DeviceSetupReadiness;
	usage?: HubResult<DeviceUsageResponse>;
	enrollments?: HubResult<HubEnrollment[]>;
	accountBackups?: HubResult<AccountBackupList>;
	certInventoryAll?: HubResult<FleetCertificateInventoryRow[]>;
	resourceSummary?: HubResult<ResourceSummary>;
	archiveUsage?: HubResult<ArchiveUsage>;
	release?: VerifiedRelease;
	/** Hub-corrected unix seconds; defaults to the workspace clock. */
	now?: number;
	/** Verifies a hub policy copy with the device's open keys. */
	verifyPolicy?: (
		deviceId: string,
		view: PolicyView,
	) => ManagementPolicy | undefined;
}

const ok = <T>(result: HubResult<T> | undefined): T | undefined =>
	result?.kind === "ok" ? result.data : undefined;

/** Per-device hub reads the open screens already hold in the query cache. */
function cachedByDevice<T>(
	workspace: DeviceWorkspace,
	kind: string,
): Map<string, T> {
	const found = new Map<string, T>();
	const entries = workspace.deps.queryClient.getQueriesData<T>({
		queryKey: ["devices", workspace.scopeKey, kind],
	});
	for (const [key, data] of entries) {
		const deviceId = key[3];
		if (key.length === 4 && typeof deviceId === "string" && data !== undefined)
			found.set(deviceId, data);
	}
	return found;
}

type Peek = <T>(
	deviceId: string,
	spec: StreamSpec,
) => StreamState<T> | undefined;

const peekOf = (streams: LiveStreams): Peek | undefined => {
	const peek = (streams as { peek?: Peek }).peek;
	return peek ? peek.bind(streams) : undefined;
};

/** Offline queues and certificates load on connect (IA §2.1): read their buffers without adding demand. */
function streamFacts(
	workspace: DeviceWorkspace,
	deviceId: string,
	placementIds: readonly string[],
): Pick<LiveDeviceInput, "offlineQueues" | "certificates"> {
	const peek = peekOf(workspace.streams);
	if (!peek) return {};
	const queues: Record<string, OfflineQueueStatus[]> = {};
	for (const placementId of placementIds) {
		const data = peek<OfflineQueueStatus[]>(deviceId, {
			kind: "offline_queues",
			placementId,
		})?.data;
		if (data) queues[placementId] = data;
	}
	const certificates = peek<CertificateInventory>(deviceId, {
		kind: "certificates",
	})?.data;
	return {
		...(Object.keys(queues).length ? { offlineQueues: queues } : {}),
		...(certificates ? { certificates } : {}),
	};
}

function liveInput(
	workspace: DeviceWorkspace,
	deviceId: string,
): LiveDeviceInput | undefined {
	const state = workspace.live.state(deviceId);
	const inspection = workspace.live.inspection(deviceId);
	const reported = workspace.facts.get(deviceId);
	if (state.kind === "idle" && !inspection && !reported) return undefined;
	const streamed = inspection
		? streamFacts(
				workspace,
				deviceId,
				inspection.value.placements.map((placement) => placement.id),
			)
		: {};
	const offlineQueues = {
		...streamed.offlineQueues,
		...reported?.offlineQueues,
	};
	return {
		state,
		...(inspection ? { inspection } : {}),
		...streamed,
		...reported,
		...(Object.keys(offlineQueues).length ? { offlineQueues } : {}),
	};
}

/** BG2: the hub's enrollment list, or on older hubs the setups this computer still tracks. */
function pendingSetups(
	workspace: DeviceWorkspace,
	enrollments: HubResult<HubEnrollment[]> | undefined,
): PendingSetup[] | undefined {
	if (enrollments?.kind === "ok")
		return enrollments.data.map((row) => ({
			enrollmentId: row.enrollment_id,
			deviceId: row.device_id,
			name: row.name,
			state: row.state,
			createdAt: row.created_at,
			expiresAt: row.expires_at,
			local: false,
		}));
	if (!enrollments) return undefined;
	return workspace.activity.list().flatMap((item) => {
		const handle = item.resume;
		if (
			handle?.type !== "setup" ||
			item.deadlineAt === undefined ||
			isActivityFinished(item)
		)
			return [];
		return [
			{
				enrollmentId: handle.enrollmentId,
				...(handle.deviceId ? { deviceId: handle.deviceId } : {}),
				name: item.target.deviceName ?? "",
				state: "pending" as const,
				createdAt: Math.floor(item.startedAt / 1000),
				expiresAt: Math.floor(item.deadlineAt / 1000),
				local: true,
			},
		];
	});
}

function accountBackups(
	workspace: DeviceWorkspace,
	list: AccountBackupList | undefined,
): AttentionInput["accountBackups"] {
	if (list)
		return Object.fromEntries(
			list.vaults.map((vault) => [
				vault.key_id,
				{ revision: vault.revision, updatedAt: vault.updated_at },
			]),
		);
	return Object.fromEntries(
		cachedByDevice<{ revision: number }>(workspace, "account-backup"),
	);
}

function certInventory(
	workspace: DeviceWorkspace,
	all: FleetCertificateInventoryRow[] | undefined,
): AttentionInput["certInventory"] {
	if (all) return Object.fromEntries(all.map((row) => [row.device_id, row]));
	return Object.fromEntries(
		cachedByDevice<PublicCertificateInventory>(workspace, "cert-inventory"),
	);
}

function policies(
	workspace: DeviceWorkspace,
	verify: AttentionHubData["verifyPolicy"],
): AttentionInput["policies"] {
	const result: AttentionInput["policies"] = {};
	for (const [deviceId, view] of cachedByDevice<PolicyView>(
		workspace,
		"policy",
	)) {
		const policy = verify?.(deviceId, view);
		result[deviceId] = policy ? { ...view, policy } : view;
	}
	return result;
}

function myAccess(workspace: DeviceWorkspace): AttentionInput["myAccess"] {
	const result: NonNullable<AttentionInput["myAccess"]> = {};
	for (const [deviceId, read] of cachedByDevice<HubResult<MyAccess>>(
		workspace,
		"my-access",
	))
		if (read.kind === "ok") result[deviceId] = read.data;
	return result;
}

/**
 * CA11 from the workspace snapshots and the P1 query results. Every hub-backed
 * field stays absent until it is known, so its rules emit nothing (plane honesty).
 */
/** `{ [key]: value }`, or nothing while the value is not known. */
const known = <K extends string, V>(key: K, value: V | undefined) =>
	(value === undefined ? {} : { [key]: value }) as { [P in K]?: V };

/** Fleet-wide hub reads; a route the hub lacks or has not answered leaves its field out. */
function hubFields(
	workspace: DeviceWorkspace,
	hubData: AttentionHubData,
): Partial<AttentionInput> &
	Pick<AttentionInput, "accountBackups" | "certInventory"> {
	const usage = ok(hubData.usage);
	const backups = ok(hubData.accountBackups);
	const manifest = hubData.release?.manifest;
	return {
		...known("readiness", hubData.readiness),
		...known("releaseTrust", hubData.hub.releaseTrust),
		...known(
			"latestRelease",
			manifest && {
				version: manifest.release_version,
				sequence: manifest.sequence,
			},
		),
		...known("usage", usage && { limits: usage.limits, usage: usage.usage }),
		...known("pendingSetups", pendingSetups(workspace, hubData.enrollments)),
		accountBackups: accountBackups(workspace, backups),
		...known(
			"accountBackupSlots",
			backups && { used: backups.used, max: backups.max },
		),
		certInventory: certInventory(workspace, ok(hubData.certInventoryAll)),
		...known("resourceSummary", ok(hubData.resourceSummary)),
		...known("archiveUsage", ok(hubData.archiveUsage)),
	};
}

/** What the managers hold per device: encrypted status, live facts, clock skew. */
function deviceFields(
	workspace: DeviceWorkspace,
	deviceIds: Iterable<string>,
): Pick<AttentionInput, "fleet" | "live" | "clock"> {
	const { clock, fleet } = workspace;
	const states: AttentionInput["fleet"] = {};
	const live: AttentionInput["live"] = {};
	const deviceSkewS: Record<string, number | undefined> = {};
	for (const deviceId of deviceIds) {
		const state = fleet.get(deviceId);
		if (state) states[deviceId] = state;
		const input = liveInput(workspace, deviceId);
		if (input) live[deviceId] = input;
		const skew = clock.deviceSkewS(deviceId);
		if (skew !== undefined) deviceSkewS[deviceId] = skew;
	}
	return {
		fleet: states,
		live,
		clock: { hubOffsetS: clock.hubOffsetS, deviceSkewS },
	};
}

export function buildAttentionInput(
	workspace: DeviceWorkspace,
	hubData: AttentionHubData,
): AttentionInput {
	const { clock, keys, local, activity, facts } = workspace;
	const devices = hubData.rows ?? [];
	const sessions = keys.list();
	const deviceIds = new Set([
		...devices.map((row) => row.device_id),
		...sessions.map((session) => session.deviceId),
	]);
	return {
		now: hubData.now ?? Math.floor(clock.now() / 1000),
		me: workspace.deps.scope.account,
		hub: hubData.hub,
		devices,
		devicesLoaded: hubData.rows !== undefined,
		...hubFields(workspace, hubData),
		...deviceFields(workspace, deviceIds),
		myAccess: myAccess(workspace),
		resources: Object.fromEntries(
			cachedByDevice<DeviceResources>(workspace, "resources"),
		),
		keys: sessions,
		local: local.summary(),
		policies: policies(workspace, hubData.verifyPolicy),
		authorities: [],
		activity: activity.list(),
		agentLastRead: { ...facts.agentLastRead() },
		// Without the hub list a request can't be told from access that ended.
		...known("accessRequests", hubData.rows && [...facts.accessRequests()]),
	};
}

/* Gates (IA §3.1): one context builder for `useGate`, the attention actions and the action layer. */

export interface GateTarget {
	projectId?: string;
	placementId?: string;
	/** Action-specific preconditions the screen knows (rollout running, replicas, sizes, …). */
	extra?: GateExtra;
	labels?: GateContext["labels"];
	projectRole?: GateContext["projectRole"];
	/** Facts the inspection does not carry (rollout sources of the placement, …). */
	features?: Partial<GateFeatures>;
}

export interface GateSources {
	workspace: DeviceWorkspace;
	input: AttentionInput;
	tokenScopeAll: boolean;
}

function capabilitiesOf(
	{ workspace, input }: GateSources,
	deviceId: string,
): readonly GrantedCapabilities[] | undefined {
	const access = input.myAccess?.[deviceId];
	if (access?.grants.length)
		return access.grants.map((grant) => ({
			scope: grant.scope,
			caps: grant.capabilities,
			expiresAt: grant.expires_at,
		}));
	const mine = workspace.fleet.get(deviceId)?.policy?.myGrant;
	return mine
		? [
				{
					scope: mine.scope,
					caps: mine.capabilities,
					expiresAt: mine.expires_at,
				},
			]
		: undefined;
}

function featuresOf(
	{ input }: GateSources,
	deviceId: string,
	extra: Partial<GateFeatures> | undefined,
): GateFeatures | undefined {
	const read = input.live[deviceId];
	const inspection = read?.inspection;
	if (!inspection) return undefined;
	const { value } = inspection;
	const open = read.state.kind === "live" || read.state.kind === "renewing";
	const version = value.agent?.release_version ?? value.agentVersion;
	return {
		flags: value.features,
		...(value.hostOperations ? { hostOperations: value.hostOperations } : {}),
		certificateManagement: value.certificate_management === 1,
		certificateIssuance: value.certificate_issuance === 1,
		certificateAcme: value.certificate_acme === 1,
		...(version ? { agentVersion: version } : {}),
		...(value.isolation?.platform ? { os: value.isolation.platform } : {}),
		source: classify("live_inspection", {
			now: input.now,
			at: inspection.readAt,
			loaded: true,
			sessionOpen: open,
		}),
		...extra,
	};
}

/** G0–G13 inputs for one device (or none, for account-level actions) from the current snapshots. */
export function buildGateContext(
	sources: GateSources,
	deviceId?: string,
	target: GateTarget = {},
): GateContext {
	const { workspace, input, tokenScopeAll } = sources;
	const base: GateContext = {
		now: input.now,
		platform: workspace.deps.platform,
		auth: { signedIn: true, tokenScopeAll },
		hub: input.hub,
		relationship: "unknown",
		ownerPowers: false,
		...(target.projectId || target.placementId
			? {
					target: {
						...(target.projectId ? { projectId: target.projectId } : {}),
						...(target.placementId ? { placementId: target.placementId } : {}),
					},
				}
			: {}),
		...(target.labels ? { labels: target.labels } : {}),
		...(target.projectRole ? { projectRole: target.projectRole } : {}),
		...(target.extra ? { extra: target.extra } : {}),
		...(input.archiveUsage
			? {
					planTier: {
						storesHistory: input.archiveUsage.max_bytes > 0,
						tierName: input.archiveUsage.tier,
					},
				}
			: {}),
	};
	if (!deviceId) return base;
	const device = input.devices.find((row) => row.device_id === deviceId);
	const relationship = device
		? relationshipOf(device, input.me)
		: input.devices.length
			? "none"
			: "unknown";
	const owner = relationship === "owner";
	const capabilities = owner ? undefined : capabilitiesOf(sources, deviceId);
	const features = featuresOf(sources, deviceId, target.features);
	const hostIsolation = input.live[deviceId]?.inspection?.value.hostIsolation;
	const vault = input.local.vaults.find((row) => row.deviceId === deviceId);
	return {
		...base,
		...(device ? { device } : {}),
		relationship,
		...(capabilities ? { capabilities } : {}),
		keys: workspace.keys.snapshot(deviceId),
		live: workspace.live.state(deviceId),
		...(features ? { features } : {}),
		...(hostIsolation ? { hostIsolation } : {}),
		ownerPowers: owner && vault?.role === "owner",
	};
}

/** The gated action behind an attention button; labels without one only navigate. */
const ATTENTION_ACTION_GATES: Partial<Record<AttentionActionCode, ActionId>> = {
	connect_live: "connect_live",
	update_agent: "agent_update",
	revoke: "revoke_device",
	revoke_spending_limit: "spending_limit_revoke",
	back_up_to_account: "account_backup_save",
	update_account_backup: "account_backup_save",
	retry_upload: "account_backup_save",
	restore_keys: "account_backup_restore",
	delete_keys: "delete_local_keys",
	remove_from_computer: "delete_local_keys",
	renew_access_rules: "share_access",
	activate: "activate_staged",
	try_again: "offline_queue_retry",
	check_result: "check_unconfirmed",
	check_again: "check_unconfirmed",
	renew_readers: "approve_metric_readers",
	resume_recording: "approve_history_readers",
	replace_approval: "cloud_access_create",
	replace_limit: "spending_limit_create",
	set_up_again: "setup_device",
};

function subjectTarget(item: AttentionItem): {
	deviceId?: string;
	target: GateTarget;
} {
	const { subject } = item;
	if (subject.kind === "service")
		return {
			deviceId: subject.deviceId,
			target: {
				placementId: subject.serviceId,
				...(subject.projectId ? { projectId: subject.projectId } : {}),
			},
		};
	return {
		deviceId: "deviceId" in subject ? subject.deviceId : undefined,
		target: {},
	};
}

/** "Renew" is said of a certificate, a person's access and a device plan; only the certificate is renewed on the device. */
function actionGate(item: AttentionItem): ActionId | undefined {
	const code = item.action?.code;
	if (!code) return undefined;
	if (code === "renew")
		return item.subject.kind === "certificate" ? "csr_create" : undefined;
	return ATTENTION_ACTION_GATES[code];
}

function gateOf(sources: GateSources, item: AttentionItem): GateResult | null {
	const action = actionGate(item);
	if (!action) return null;
	const { deviceId, target } = subjectTarget(item);
	return evaluateGate(action, buildGateContext(sources, deviceId, target));
}

/** Attention rules never evaluate gates (plan §4.0): the binding attaches them. */
function withGates(
	sources: GateSources,
	items: readonly AttentionItem[],
): AttentionItem[] {
	return items.map((item) => {
		const gate = gateOf(sources, item);
		return gate && item.action
			? { ...item, action: { ...item.action, gate } }
			: item;
	});
}

/* The one computation per provider. */

export interface AttentionState extends GateSources {
	items: AttentionItem[];
	/** `GET /devices` state for the area gates (G3) and the Hub stamp (R5). */
	rows: {
		data: DeviceRow[] | undefined;
		error: unknown;
		dataUpdatedAt: number;
		errorUpdatedAt: number;
		errorUpdateCount: number;
		refetch(): Promise<unknown>;
	};
	verifyPolicy(
		deviceId: string,
		view: PolicyView,
	): ManagementPolicy | undefined;
	/** Why `verifyPolicy` has, or has no, rules for this hub copy. */
	policyState(deviceId: string, view: PolicyView): PolicyVerification;
}

/**
 * `pending`: can't be checked right now (keys closed here, the crypto module
 * still loading, or a hub copy without signed rules). `rejected`: the keys
 * are open and the rules don't check out with the owner key.
 */
export type PolicyVerification = "verified" | "pending" | "rejected";

const AttentionContext = createContext<AttentionState | null>(null);

function useDwellTick() {
	const [tick, setTick] = useState(0);
	useEffect(() => {
		const timer = setInterval(
			() => setTick((value) => value + 1),
			DWELL_TICK_MS,
		);
		return () => clearInterval(timer);
	}, []);
	return tick;
}

const orUndefined = <T>(run: () => T): T | undefined => {
	try {
		return run();
	} catch {
		return undefined;
	}
};

/** The owner-signed rules of a hub copy, checked against the device's signed identity; `undefined` while locked or when the check fails. */
function verifiedPolicy(
	crypto: DeviceCrypto | undefined,
	workspace: DeviceWorkspace,
	deviceId: string,
	view: PolicyView,
): ManagementPolicy | undefined {
	const receipt = workspace.keys.receipt(deviceId);
	const vault = workspace.keys.vault(deviceId);
	const jws = view.policy_jws;
	if (!crypto || !receipt || !vault || !jws) return undefined;
	return orUndefined(() =>
		crypto.verifyManagementPolicy(
			jws,
			crypto.verifyDeviceReceipt(
				receipt,
				vault.manifestJws,
				vault.ownerControllerKey ?? vault.controllerPublic.controller_key,
			).owner_invitation_key,
		),
	);
}

/** The crypto module once any device is unlocked; it is a library handle and stays in a ref. */
function useLoadedCrypto(workspace: DeviceWorkspace, version: number) {
	const crypto = useRef<DeviceCrypto | undefined>(undefined);
	const [loaded, setLoaded] = useState(0);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `version` re-checks the key sessions
	const anyUnlocked = useMemo(
		() => workspace.keys.list().some((row) => row.state === "unlocked"),
		[workspace, version],
	);
	useEffect(() => {
		if (!anyUnlocked || crypto.current) return;
		let active = true;
		workspace.deps.crypto().then(
			(module) => {
				if (!active) return;
				crypto.current = module;
				setLoaded((value) => value + 1);
			},
			() => undefined,
		);
		return () => {
			active = false;
		};
	}, [anyUnlocked, workspace]);
	return { crypto, loaded };
}

/** Owner policies are verified with the device's open keys; a verified copy is cached per version and digest. */
function usePolicyVerifier(workspace: DeviceWorkspace, version: number) {
	const { crypto, loaded } = useLoadedCrypto(workspace, version);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `loaded` swaps the verifier once the crypto module is there
	return useMemo(() => {
		const cache = new Map<string, ManagementPolicy>();
		const verifyPolicy = (deviceId: string, view: PolicyView) => {
			if (!workspace.keys.receipt(deviceId)) return undefined;
			const key = `${deviceId}|${view.version}|${view.digest ?? ""}`;
			const policy =
				cache.get(key) ??
				verifiedPolicy(crypto.current, workspace, deviceId, view);
			if (policy) cache.set(key, policy);
			return policy;
		};
		const policyState = (
			deviceId: string,
			view: PolicyView,
		): PolicyVerification => {
			if (verifyPolicy(deviceId, view)) return "verified";
			const { keys } = workspace;
			const checkable =
				!!crypto.current &&
				!!view.policy_jws &&
				!!keys.receipt(deviceId) &&
				!!keys.vault(deviceId);
			return checkable ? "rejected" : "pending";
		};
		return { verifyPolicy, policyState };
	}, [workspace, crypto, loaded]);
}

/**
 * Mounted by `DeviceWorkspaceProvider`. Holds the P1 queries the attention
 * engine needs and recomputes when a manager or a hub query changes, or on
 * the dwell tick. `passive` (Events column) keeps only the device list.
 */
export function AttentionProvider({
	workspace,
	passive = false,
	children,
}: Readonly<{
	workspace: DeviceWorkspace;
	passive?: boolean;
	children: ReactNode;
}>) {
	const ctx = workspace.hub;
	const version = useSyncExternalStore(
		workspace.store.subscribe,
		workspace.store.getVersion,
		workspace.store.getVersion,
	);
	const tick = useDwellTick();
	const hubQuery = useQuery(queries.hub(ctx));
	const off = hubQuery.data?.enabled === false;
	const fleetWide = !passive && hubQuery.data?.enabled === true;
	const list = useQuery({ ...queries.list(ctx), enabled: !off });
	const readiness = useQuery({
		...queries.readiness(ctx),
		enabled: fleetWide,
	});
	const usage = useQuery({ ...queries.usage(ctx), enabled: fleetWide });
	const enrollments = useQuery({
		...queries.enrollments(ctx),
		enabled: fleetWide,
	});
	const backups = useQuery({
		...queries.accountBackups(ctx),
		enabled: fleetWide,
	});
	const certificates = useQuery({
		...queries.certInventoryAll(ctx),
		enabled: fleetWide,
	});
	const resourceSummary = useQuery({
		...queries.resourceSummary(ctx),
		enabled: fleetWide,
	});
	const archiveUsage = useQuery({
		...queries.archiveUsage(ctx),
		enabled: fleetWide,
	});
	const release = useQuery({
		...queries.release(ctx, releaseConfigOf(hubQuery.data)),
		enabled: fleetWide && !!hubQuery.data?.release_trust,
	});

	// biome-ignore lint/correctness/useExhaustiveDependencies: `version` re-reads the key sessions
	const ownedUnlocked = useMemo(() => {
		const me = workspace.deps.scope.account;
		const owned = new Set(
			(list.data ?? [])
				.filter((row) => row.owner_id === me && row.status === "active")
				.map((row) => row.device_id),
		);
		return passive
			? []
			: workspace.keys
					.list()
					.filter((row) => row.state === "unlocked" && owned.has(row.deviceId))
					.map((row) => row.deviceId)
					.sort();
	}, [workspace, list.data, passive, version]);
	useQueries({
		queries: ownedUnlocked.map((deviceId) => queries.policy(ctx, deviceId)),
	});

	const { verifyPolicy, policyState } = usePolicyVerifier(workspace, version);
	const hub = useMemo(
		() =>
			hubDeviceSupport(
				{ data: hubQuery.data, error: hubQuery.error },
				{ data: usage.data },
			),
		[hubQuery.data, hubQuery.error, usage.data],
	);
	const tokenScopeAll =
		!list.error || toHubError(list.error).code !== "token_restricted";

	// biome-ignore lint/correctness/useExhaustiveDependencies: `version` and `tick` stand for the manager snapshots and the dwell clock
	const computed = useMemo(() => {
		const input = buildAttentionInput(workspace, {
			hub,
			rows: list.data,
			readiness: readiness.data,
			usage: usage.data,
			enrollments: enrollments.data,
			accountBackups: backups.data,
			certInventoryAll: certificates.data,
			resourceSummary: resourceSummary.data,
			archiveUsage: archiveUsage.data,
			release: release.data,
			verifyPolicy,
		});
		const sources: GateSources = { workspace, input, tokenScopeAll };
		return {
			...sources,
			items: withGates(sources, computeAttention(input, workspace.attention)),
		};
	}, [
		workspace,
		version,
		tick,
		hub,
		tokenScopeAll,
		verifyPolicy,
		list.data,
		readiness.data,
		usage.data,
		enrollments.data,
		backups.data,
		certificates.data,
		resourceSummary.data,
		archiveUsage.data,
		release.data,
	]);

	const state = useMemo<AttentionState>(
		() => ({
			...computed,
			rows: {
				data: list.data,
				error: list.error,
				dataUpdatedAt: list.dataUpdatedAt,
				errorUpdatedAt: list.errorUpdatedAt,
				errorUpdateCount: list.errorUpdateCount,
				refetch: list.refetch,
			},
			verifyPolicy,
			policyState,
		}),
		[
			computed,
			verifyPolicy,
			policyState,
			list.data,
			list.error,
			list.dataUpdatedAt,
			list.errorUpdatedAt,
			list.errorUpdateCount,
			list.refetch,
		],
	);

	return createElement(AttentionContext.Provider, { value: state }, children);
}

/** The provider-level computation; throws outside a `DeviceWorkspaceProvider`. */
export function useAttentionState(): AttentionState {
	const state = useContext(AttentionContext);
	if (!state)
		throw new Error(
			"Device hooks need a DeviceWorkspaceProvider with a signed-in account above them.",
		);
	return state;
}

export type { AttentionCounts, AttentionFilter };

/**
 * The memoised CA11 input (same identity until it recomputes), for views that
 * derive many devices at once with the pure model (`fleetFacts`, `coverage`,
 * `rollupHealth`) without a hook per row.
 */
export function useAttentionInput(): AttentionInput {
	return useAttentionState().input;
}

const NO_SETUPS: PendingSetup[] = [];

/** Setups waiting for their first check-in: the hub's list, or on older hubs what this computer tracks. */
export function usePendingSetups(): PendingSetup[] {
	return useAttentionState().input.pendingSetups ?? NO_SETUPS;
}

/** One global list, filtered per placement (IA §6.5): device, service, app or severity. */
export function useAttention(filter: AttentionFilter = {}): AttentionItem[] {
	const { items, input } = useAttentionState();
	const { deviceId, serviceId, appId, minSeverity } = filter;
	return useMemo(
		() =>
			filterAttention(
				items,
				{ deviceId, serviceId, appId, minSeverity },
				input,
			),
		[items, input, deviceId, serviceId, appId, minSeverity],
	);
}

/** Split counts for the top bar; Info never counts. */
export function useAttentionCounts(
	filter: AttentionFilter = {},
): AttentionCounts {
	const items = useAttention(filter);
	return useMemo(() => countAttention(items), [items]);
}

/** `snoozeAttention` bound to the workspace memory: notices only, 7 days, this computer. */
export function useSnoozeAttention(): (item: AttentionItem) => boolean {
	const { workspace, input } = useAttentionState();
	return useMemo(
		() => (item: AttentionItem) => {
			if (item.severity !== "notice") return false;
			workspace.attention.snoozed[item.id] = input.now + 7 * 86_400;
			workspace.attention.save();
			workspace.store.bump();
			return true;
		},
		[workspace, input.now],
	);
}

/* Status bar (SPEC §3.5, M-DATA §3.6). */

export type PlaneSegmentId =
	| "hub"
	| "status"
	| "live"
	| "local"
	| "device"
	| "certificates";

export interface PlaneSegment {
	id: PlaneSegmentId;
	plane: SourcePlane;
	freshness: Freshness;
	/** Devices this plane covers now. */
	count: number;
	/** Devices it could cover (non-revoked devices you can see). */
	total: number;
	/** Devices whose plane is failing (snapshot or live errors). */
	failing: number;
}

export interface PlaneStatus {
	segments: PlaneSegment[];
	/** The segment to show alone below 720 px. */
	worst: PlaneSegmentId;
	hubOffsetS?: number;
}

const AGE_ORDER: Freshness["age"][] = [
	"error",
	"lastknown",
	"delayed",
	"locked",
	"noaccess",
	"unsupported",
	"notloaded",
	"snapshot",
	"current",
	"live",
];

const worse = (a: Freshness, b: Freshness) =>
	AGE_ORDER.indexOf(a.age) <= AGE_ORDER.indexOf(b.age) ? a : b;

function hubSegment(state: AttentionState, total: number): PlaneSegment {
	const { rows, input, workspace } = state;
	const offsetMs = (workspace.clock.hubOffsetS ?? 0) * 1000;
	const at = rows.dataUpdatedAt
		? Math.floor((rows.dataUpdatedAt - offsetMs) / 1000)
		: undefined;
	const failed = rows.error
		? { code: "refresh_failed" as const }
		: input.hub.error
			? { code: "refresh_failed" as const }
			: undefined;
	return {
		id: "hub",
		plane: "hub",
		freshness: classify("device_row", {
			now: input.now,
			at,
			loaded: rows.data !== undefined,
			...(failed ? { error: failed } : {}),
		}),
		count: rows.data?.length ?? 0,
		total,
		failing: failed ? 1 : 0,
	};
}

function statusSegment(
	state: AttentionState,
	active: DeviceRow[],
): PlaneSegment {
	const { input } = state;
	let freshness: Freshness | undefined;
	let count = 0;
	let failing = 0;
	for (const row of active) {
		const fleet = input.fleet[row.device_id];
		if (!fleet?.status) continue;
		count++;
		if (fleet.error) failing++;
		freshness = freshness
			? worse(freshness, fleet.freshness.status)
			: fleet.freshness.status;
	}
	return {
		id: "status",
		plane: "snap",
		freshness:
			freshness ??
			classify("fleet_status", {
				now: input.now,
				loaded: false,
				...(input.keys.some((row) => row.state !== "none")
					? { locked: true }
					: { notLoadedReason: { code: "no_keys_here" as const } }),
			}),
		count,
		total: active.length,
		failing,
	};
}

function liveSegment(state: AttentionState, active: DeviceRow[]): PlaneSegment {
	const { input } = state;
	const states = active.map((row) => input.live[row.device_id]?.state);
	const open = states.filter(
		(live) => live?.kind === "live" || live?.kind === "renewing",
	).length;
	const failing = states.filter(
		(live) =>
			live?.kind === "failed" ||
			live?.kind === "unreachable" ||
			live?.kind === "reconnecting",
	).length;
	return {
		id: "live",
		plane: "live",
		freshness: classify("session", {
			now: input.now,
			loaded: open > 0,
			sessionOpen: open > 0,
			...(open === 0
				? { notLoadedReason: { code: "connect_required" as const } }
				: {}),
		}),
		count: open,
		total: active.length,
		failing,
	};
}

function planeSegments(state: AttentionState): PlaneSegment[] {
	const { input } = state;
	const active = input.devices.filter((row) => row.status === "active");
	const withKeys = input.local.vaults.length;
	const reporting = active.filter(
		(row) => input.certInventory[row.device_id]?.updated_at != null,
	).length;
	return [
		hubSegment(state, active.length),
		statusSegment(state, active),
		liveSegment(state, active),
		{
			id: "local",
			plane: "local",
			freshness: classify("local_vault", { now: input.now, loaded: true }),
			count: withKeys,
			total: active.length,
			failing: 0,
		},
		{
			id: "device",
			plane: "device",
			freshness: classify("agent_local", {
				now: input.now,
				loaded: false,
				notLoadedReason: { code: "only_on_device" },
			}),
			count: 0,
			total: active.length,
			failing: 0,
		},
		{
			id: "certificates",
			plane: "hub",
			freshness: classify("cert_inventory", {
				now: input.now,
				loaded: reporting > 0,
				...(reporting === 0
					? { notLoadedReason: { code: "never_reported" as const } }
					: {}),
			}),
			count: reporting,
			total: active.length,
			failing: 0,
		},
	];
}

/** The six status-bar segments: Hub, Encrypted status, Live, This computer, On-device only, Certificates. */
export function usePlaneStatus(): PlaneStatus {
	const state = useAttentionState();
	return useMemo(() => {
		const segments = planeSegments(state);
		const worst = segments
			.filter((segment) => segment.id !== "device")
			.reduce((current, segment) =>
				worse(current.freshness, segment.freshness) === current.freshness
					? current
					: segment,
			);
		return {
			segments,
			worst: worst.id,
			...(state.workspace.clock.hubOffsetS === undefined
				? {}
				: { hubOffsetS: state.workspace.clock.hubOffsetS }),
		};
	}, [state]);
}
