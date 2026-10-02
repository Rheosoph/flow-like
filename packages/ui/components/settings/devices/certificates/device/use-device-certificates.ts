"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo } from "react";
import {
	type AcmeCertificate,
	readAcmeCertificates,
} from "../../../../../lib/device-management/certificate-acme";
import {
	type CertificateIssuer,
	type CertificateRequest,
	readCertificateIssuers,
	readCertificateRequests,
} from "../../../../../lib/device-management/certificate-issuance";
import {
	type CertificateInventory,
	type DeviceCertificate,
	type PublicCertificateInventory,
	certificateStatus,
} from "../../../../../lib/device-management/certificates";
import { fleetFacts } from "../../../../../lib/device-management/model/device-view";
import { classify } from "../../../../../lib/device-management/model/freshness";
import { evaluateGate } from "../../../../../lib/device-management/model/gates";
import type {
	ActionId,
	Freshness,
	GateExtra,
	GateResult,
	InspectionPlus,
	PresenceKind,
} from "../../../../../lib/device-management/model/types";
import type { ManagementCall } from "../../../../../lib/device-management/telemetry";
import type { ManagementRejection } from "../../../../../lib/device-management/types";
import type {
	DeviceWorkspace,
	StreamSpec,
	StreamState,
} from "../../../../../lib/device-management/workspace/types";
import {
	type HubRead,
	buildGateContext,
	deviceCall,
	useAttentionState,
	useCertificateInventory,
	useLiveStream,
	useMyAccess,
} from "../../workspace";

/** A device keeps at most this many certificates, pending requests and Let's Encrypt setups together. */
export const MAX_CERTIFICATE_SLOTS = 32;
/** How often the live reads repeat while the tab is open. */
export const LIVE_EVERY_S = 60;

const CERTIFICATES: StreamSpec = {
	kind: "certificates",
	everyMs: LIVE_EVERY_S * 1000,
};

export type RenewalMode = "manual" | "delegated" | "acme";
export type Validity = ReturnType<typeof certificateStatus>;

export interface CertificateRow {
	id: string;
	fingerprint: string;
	/** Unix seconds; the earliest expiry in the chain. */
	notAfter: number;
	revision: number;
	validity: Validity;
	/** Label, names, issuer and uses: known after a live read. */
	detail?: DeviceCertificate;
	issuer?: CertificateIssuer;
	acme?: AcmeCertificate;
	/** Undefined while the renewal settings can't be read (not the owner, not connected). */
	mode?: RenewalMode;
	/** Automatic renewal is set up and its last attempt failed, or its authority ran out. */
	failing: boolean;
	/** Services that use it; undefined until a live read. */
	uses?: number;
}

export interface LiveRead<T> {
	/** The agent supports the read and this viewer may ask for it. */
	enabled: boolean;
	data: T | undefined;
	freshness: Freshness;
	loading: boolean;
	/** The last read failed; `data` is what was read before. */
	failed: boolean;
	refetch(): Promise<void>;
}

export interface CertificateSupport {
	/** A live read said what the agent supports; false while locked or never connected. */
	known: boolean;
	management: boolean;
	issuance: boolean;
	acme: boolean;
	canManage: boolean;
	canDelegate: boolean;
	/** BG27: the agent reports how often renewal failed and why. */
	failureDetail: boolean;
}

export interface CertificateDetail {
	inventory: CertificateInventory | undefined;
	freshness: Freshness;
	/** The first live read is running. */
	loading: boolean;
	/** The live read failed and nothing was read before. */
	failed: boolean;
	rejected?: ManagementRejection;
}

export interface DeviceCertificates {
	deviceId: string;
	name: string;
	owner: boolean;
	ownerId: string;
	presence: PresenceKind;
	/** A live session is open. */
	live: boolean;
	support: CertificateSupport;
	/** What the device reported to the hub: IDs, fingerprints and expiry. */
	hub: HubRead<PublicCertificateInventory>;
	detail: CertificateDetail;
	rows: CertificateRow[];
	requests: LiveRead<CertificateRequest[]>;
	issuers: LiveRead<CertificateIssuer[]>;
	acme: LiveRead<AcmeCertificate[]>;
	/** `complete`: requests and Let's Encrypt setups were read too. */
	slots?: { used: number; complete: boolean };
	/** The device's services, from the last live read. */
	services: { id: string; projectId: string }[];
	/** Listening addresses of the services whose settings were read so far. */
	endpoints: { serviceId: string; host: string; port: number }[];
	/** Services whose settings were read: their address is in `endpoints`, or they have none. */
	checkedServices: string[];
	/** Unix seconds of the attention clock (changes every 30 s, never every second). */
	now: number;
	gate(action: ActionId, extra?: GateExtra): GateResult;
	/** Re-reads everything the tab shows. Never rejects. */
	reload(): Promise<void>;
}

export const certificateLabel = (
	row: Pick<CertificateRow, "id" | "detail">,
): string => row.detail?.label ?? row.id.slice(0, 8);

export const certificateNames = (
	value: Pick<DeviceCertificate, "dns_names" | "ip_addresses">,
): string[] => [...value.dns_names, ...value.ip_addresses];

type Peek = <T>(
	deviceId: string,
	spec: StreamSpec,
) => StreamState<T> | undefined;

/** The certificate list loads on connect: read its buffer without asking the device again. */
function peekCertificates(workspace: DeviceWorkspace, deviceId: string) {
	const peek = (workspace.streams as { peek?: Peek }).peek;
	return peek?.call(workspace.streams, deviceId, CERTIFICATES) as
		| StreamState<CertificateInventory>
		| undefined;
}

type CertificateStream = Pick<
	StreamState<CertificateInventory>,
	"data" | "freshness" | "rejected"
>;

interface DetailSources {
	/** The tab's own subscription, once it reported. */
	stream: CertificateStream | undefined;
	/** What the connection loaded by itself, while the tab doesn't subscribe. */
	peeked: StreamState<CertificateInventory> | undefined;
	remembered: CertificateInventory | undefined;
	now: number;
	/** A session is open and the agent lists certificates. */
	readable: boolean;
}

function detailFreshness(sources: DetailSources, loaded: boolean): Freshness {
	if (sources.stream) return sources.stream.freshness;
	return (
		sources.peeked?.freshness ??
		classify("certificates_live", {
			now: sources.now,
			loaded,
			sessionOpen: false,
		})
	);
}

function detailOf(sources: DetailSources): CertificateDetail {
	const { stream, peeked, remembered, readable } = sources;
	const inventory = stream?.data ?? peeked?.data ?? remembered;
	const rejected = stream?.rejected;
	const failed = !inventory && stream?.freshness.age === "error";
	const waiting = !inventory && !rejected && !failed;
	return {
		inventory,
		freshness: detailFreshness(sources, !!inventory),
		loading: readable && waiting,
		failed,
		...(rejected ? { rejected } : {}),
	};
}

/** The last attempt failed or attempts keep failing (BG27 count, when the agent reports it). */
const attemptsFailing = (row: {
	last_error: string | null;
	failures?: number;
}) => !!row.last_error || (row.failures ?? 0) > 0;

/** Renewal with the owner's authority also fails once that authority has run out. */
export const issuerFailing = (now: number, issuer: CertificateIssuer) =>
	attemptsFailing(issuer) || issuer.not_after <= now;

const renewalFailing = (
	now: number,
	issuer: CertificateIssuer | undefined,
	acme: AcmeCertificate | undefined,
) =>
	(!!issuer && issuerFailing(now, issuer)) || (!!acme && attemptsFailing(acme));

interface RowSources {
	hub: PublicCertificateInventory | undefined;
	detail: CertificateInventory | undefined;
	issuers: CertificateIssuer[] | undefined;
	acme: AcmeCertificate[] | undefined;
	/** Every renewal setting this viewer may read was read. */
	renewalKnown: boolean;
	now: number;
}

function detailRows(sources: RowSources): CertificateRow[] {
	const { detail, issuers, acme, renewalKnown, now } = sources;
	return (detail?.certificates ?? []).map((certificate) => {
		const id = certificate.certificate_id;
		const issuer = issuers?.find((row) => row.certificate_id === id);
		const policy = acme?.find((row) => row.certificate_id === id);
		const mode: RenewalMode | undefined = policy
			? "acme"
			: issuer
				? "delegated"
				: renewalKnown
					? "manual"
					: undefined;
		return {
			id,
			fingerprint: certificate.sha256_fingerprint,
			notAfter: certificate.not_after,
			revision: certificate.revision,
			validity: certificateStatus(certificate, now),
			detail: certificate,
			...(issuer ? { issuer } : {}),
			...(policy ? { acme: policy } : {}),
			...(mode ? { mode } : {}),
			failing: renewalFailing(now, issuer, policy),
			uses: certificate.binding_count ?? certificate.bindings.length,
		};
	});
}

/** Soonest expiry first, so what needs work leads the list. */
function buildRows(sources: RowSources): CertificateRow[] {
	const rows = sources.detail
		? detailRows(sources)
		: (sources.hub?.certificates ?? []).map((certificate) => ({
				id: certificate.certificate_id,
				fingerprint: certificate.fingerprint_sha256,
				notAfter: certificate.not_after,
				revision: certificate.revision,
				validity: certificateStatus(certificate, sources.now),
				failing: false,
			}));
	return rows.sort(
		(a, b) => a.notAfter - b.notAfter || a.id.localeCompare(b.id),
	);
}

function slotsOf(
	detail: CertificateInventory | undefined,
	requests: LiveRead<CertificateRequest[]>,
	acme: LiveRead<AcmeCertificate[]>,
): DeviceCertificates["slots"] {
	if (!detail) return undefined;
	const ids = new Set([
		...detail.certificates.map((row) => row.certificate_id),
		...(requests.data ?? []).map((row) => row.certificate_id),
		...(acme.data ?? []).map((row) => row.certificate_id),
	]);
	return {
		used: ids.size,
		complete: requests.data !== undefined && acme.data !== undefined,
	};
}

interface LiveReadSpec<T> {
	deviceId: string;
	name: "requests" | "issuers" | "acme";
	enabled: boolean;
	live: boolean;
	unlocked: boolean;
	read(call: ManagementCall): Promise<T>;
	record(value: T): void;
}

function useLiveRead<T>(spec: LiveReadSpec<T>): LiveRead<T> {
	const { workspace, input } = useAttentionState();
	const { deviceId, name, enabled, live, unlocked, read, record } = spec;
	const query = useQuery({
		queryKey: [
			"devices",
			workspace.scopeKey,
			"device-certificates",
			deviceId,
			name,
		],
		queryFn: async () => {
			const value = await read(deviceCall(workspace, deviceId, "poll"));
			if (workspace.keys.snapshot(deviceId).state === "unlocked") record(value);
			return value;
		},
		enabled: enabled && live,
		refetchInterval: LIVE_EVERY_S * 1000,
		staleTime: (LIVE_EVERY_S * 1000) / 2,
		gcTime: 0,
		retry: false,
		meta: { persist: false },
	});
	const { data, error, dataUpdatedAt, isLoading, refetch } = query;
	const shown = enabled && unlocked ? data : undefined;
	const offsetS = workspace.clock.hubOffsetS ?? 0;
	const at =
		shown === undefined || !dataUpdatedAt
			? undefined
			: Math.floor(dataUpdatedAt / 1000 - offsetS);
	return useMemo(
		() => ({
			enabled,
			data: shown,
			freshness: classify("certificates_live", {
				now: Math.max(input.now, at ?? 0),
				at,
				loaded: shown !== undefined,
				sessionOpen: live,
				...(error ? { error: { code: "refresh_failed" as const } } : {}),
			}),
			loading: enabled && live && isLoading,
			failed: !!error,
			refetch: async () => {
				if (enabled && live) await refetch();
			},
		}),
		[enabled, shown, input.now, at, live, error, isLoading, refetch],
	);
}

function useSupport(
	inspection: InspectionPlus | undefined,
): CertificateSupport {
	const management = inspection?.certificate_management === 1;
	const issuance = inspection?.certificate_issuance === 1;
	const acme = inspection?.certificate_acme === 1;
	const canManage = inspection?.can_manage_certificates === true;
	const canDelegate = inspection?.can_delegate_certificate_renewal === true;
	const failureDetail = inspection?.features?.acme_failure_detail === 1;
	const known = !!inspection;
	return useMemo(
		() => ({
			known,
			management,
			issuance,
			acme,
			canManage,
			canDelegate,
			failureDetail,
		}),
		[known, management, issuance, acme, canManage, canDelegate, failureDetail],
	);
}

/**
 * Everything the device's Certificates tab reads: the hub's public inventory
 * (IDs and expiry, readable while locked), the live list with names and uses,
 * and the pending requests and renewal settings the viewer is allowed to read.
 * Undefined while the device isn't in the hub list.
 */
export function useDeviceCertificates(
	deviceId: string,
): DeviceCertificates | undefined {
	const state = useAttentionState();
	const { workspace, input } = state;
	const queryClient = useQueryClient();
	const facts = fleetFacts(input).byId.get(deviceId);
	const hub = useCertificateInventory(deviceId);
	// What a shared device allows is known from the viewer's own access; the gates read it from the cache.
	useMyAccess(facts && facts.relationship !== "owner" ? deviceId : undefined);
	const support = useSupport(facts?.inspection);
	const live = facts?.liveOpen ?? false;
	const unlocked = facts?.keys.state === "unlocked";
	const stream = useLiveStream<CertificateInventory>(
		deviceId,
		live && support.management ? CERTIFICATES : null,
	);

	const record = useCallback(
		(value: Parameters<DeviceWorkspace["facts"]["record"]>[1]) =>
			workspace.facts.record(deviceId, value),
		[workspace, deviceId],
	);
	const requests = useLiveRead<CertificateRequest[]>({
		deviceId,
		name: "requests",
		enabled: support.issuance && support.canManage,
		live,
		unlocked,
		read: readCertificateRequests,
		record: (certificateRequests) => record({ certificateRequests }),
	});
	const issuers = useLiveRead<CertificateIssuer[]>({
		deviceId,
		name: "issuers",
		enabled: support.issuance && support.canDelegate,
		live,
		unlocked,
		read: readCertificateIssuers,
		record: (certificateIssuers) => record({ certificateIssuers }),
	});
	const acme = useLiveRead<AcmeCertificate[]>({
		deviceId,
		name: "acme",
		enabled: support.acme && support.canDelegate,
		live,
		unlocked,
		read: readAcmeCertificates,
		record: (policies) => record({ acme: policies }),
	});

	/* Decrypted reads never outlive the keys they were read with. */
	useEffect(() => {
		if (unlocked) return;
		queryClient.removeQueries({
			queryKey: [
				"devices",
				workspace.scopeKey,
				"device-certificates",
				deviceId,
			],
		});
	}, [unlocked, queryClient, workspace.scopeKey, deviceId]);

	const started = stream.started;
	const liveCertificates = facts?.liveInput?.certificates;
	const management = support.management;
	const { data, freshness, rejected } = stream;
	const detail = useMemo<CertificateDetail>(
		() =>
			detailOf({
				stream: started ? { data, freshness, rejected } : undefined,
				peeked: started ? undefined : peekCertificates(workspace, deviceId),
				remembered: liveCertificates,
				now: input.now,
				readable: live && management,
			}),
		[
			started,
			data,
			freshness,
			rejected,
			workspace,
			deviceId,
			liveCertificates,
			input.now,
			live,
			management,
		],
	);
	const inventory = detail.inventory;

	const renewalKnown =
		support.canDelegate &&
		(!issuers.enabled || issuers.data !== undefined) &&
		(!acme.enabled || acme.data !== undefined);
	const rows = useMemo(
		() =>
			buildRows({
				hub: hub.data,
				detail: inventory,
				issuers: issuers.data,
				acme: acme.data,
				renewalKnown,
				now: input.now,
			}),
		[hub.data, inventory, issuers.data, acme.data, renewalKnown, input.now],
	);
	const slots = useMemo(
		() => slotsOf(inventory, requests, acme),
		[inventory, requests, acme],
	);
	const placements = facts?.liveInput?.placements;
	const endpoints = useMemo(
		() =>
			Object.entries(placements ?? {}).flatMap(([serviceId, row]) =>
				row.host && row.port !== undefined
					? [{ serviceId, host: row.host, port: row.port }]
					: [],
			),
		[placements],
	);
	const checkedServices = useMemo(
		() => Object.keys(placements ?? {}),
		[placements],
	);
	const inspected = facts?.inspection?.placements;
	const services = useMemo(
		() =>
			(inspected ?? []).map((row) => ({
				id: row.id,
				projectId: row.project_id,
			})),
		[inspected],
	);

	const gate = useCallback(
		(action: ActionId, extra?: GateExtra) =>
			evaluateGate(
				action,
				buildGateContext(state, deviceId, extra ? { extra } : {}),
			),
		[state, deviceId],
	);
	const refreshStream = stream.refresh;
	const refetchHub = hub.refetch;
	const refetchRequests = requests.refetch;
	const refetchIssuers = issuers.refetch;
	const refetchAcme = acme.refetch;
	const reload = useCallback(async () => {
		await Promise.allSettled([
			refreshStream(),
			refetchRequests(),
			refetchIssuers(),
			refetchAcme(),
			refetchHub(),
		]);
	}, [refreshStream, refetchRequests, refetchIssuers, refetchAcme, refetchHub]);

	return useMemo(() => {
		if (!facts) return undefined;
		return {
			deviceId,
			name: facts.name,
			owner: facts.relationship === "owner",
			ownerId: facts.row.owner_id,
			presence: facts.presence.kind,
			live,
			support,
			hub,
			detail,
			rows,
			requests,
			issuers,
			acme,
			...(slots ? { slots } : {}),
			services,
			endpoints,
			checkedServices,
			now: input.now,
			gate,
			reload,
		};
	}, [
		facts,
		deviceId,
		live,
		support,
		hub,
		detail,
		rows,
		requests,
		issuers,
		acme,
		slots,
		services,
		endpoints,
		checkedServices,
		input.now,
		gate,
		reload,
	]);
}
