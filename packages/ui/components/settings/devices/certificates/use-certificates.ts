"use client";

import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useContext, useEffect, useMemo, useRef } from "react";
import { AuthContext } from "react-oidc-context";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import {
	type CertificateAuthorityEnvelope,
	type LocalCertificateAuthority,
	MAX_AUTHORITY_BACKUP_BYTES,
} from "../../../../lib/device-management/certificate-authority";
import type { PublicCertificateInventory } from "../../../../lib/device-management/certificates";
import {
	type HubError,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import {
	deviceKeys,
	queries,
} from "../../../../lib/device-management/hub/queries";
import type { Freshness } from "../../../../lib/device-management/model/types";
import {
	type DeviceAccountScope,
	addCertificateAuthority,
	readCertificateAuthorities,
	removeCertificateAuthority,
	replaceCertificateAuthority,
} from "../../../../lib/device-management/storage";
import type { DeviceCrypto } from "../../../../lib/device-management/types";
import { identityName } from "../access/person-name";
import { useAreaTime } from "../primitives/area-context";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useAttentionInput } from "../workspace/use-attention";
import { useDeviceAction } from "../workspace/use-device-action";
import {
	useDeviceRows,
	useFleetCertificateInventory,
} from "../workspace/use-hub";
import {
	type CertificateFleet,
	type PerDeviceRead,
	buildCertificateFleet,
} from "./certificates-model";

/** Older hub: devices whose certificate report is read one by one (BG28 interim). */
export const PER_DEVICE_READ_CAP = 50;
/** Day counts and expiry states don't need the 1 s clock. */
const CLOCK_STEP_S = 30;

export interface CertificateFleetRead {
	fleet: CertificateFleet;
	/** Hub-corrected unix seconds the fleet was derived at. */
	now: number;
	/** The device list and the certificate reports answered at least once. */
	loaded: boolean;
	/** Where the reports came from and how old they are. */
	freshness: Freshness;
	/** The hub has no fleet inventory: every device's report is read on its own. */
	perDevice: boolean;
	error?: HubError;
	/** Reads the device list and the reports again; resolves with what went wrong, if anything. */
	refresh(): Promise<HubError | undefined>;
}

interface PerDeviceReads {
	inventory: Record<string, PublicCertificateInventory | undefined>;
	state: Record<string, PerDeviceRead | undefined>;
}

interface ReadResult {
	data: PublicCertificateInventory | undefined;
	error: unknown;
	isLoading: boolean;
}

/**
 * Certificate expiry across the fleet from each device's public report (P1),
 * joined with what live reads added on this computer. Without the hub's fleet
 * inventory the visible devices are read one by one.
 */
export function useCertificateFleet(): CertificateFleetRead {
	const { hub } = useDeviceWorkspace();
	const input = useAttentionInput();
	const list = useDeviceRows();
	const all = useFleetCertificateInventory();
	const time = useAreaTime();
	const now = Math.floor(time.nowS / CLOCK_STEP_S) * CLOCK_STEP_S;
	const devices = list.rows ?? input.devices;
	const perDevice = all.missingOnHub;

	const readIds = useMemo(
		() =>
			perDevice
				? devices
						.filter((row) => row.status !== "revoked")
						.slice(0, PER_DEVICE_READ_CAP)
						.map((row) => row.device_id)
				: [],
		[perDevice, devices],
	);
	const combine = useCallback(
		(results: ReadResult[]): PerDeviceReads => {
			const reads: PerDeviceReads = { inventory: {}, state: {} };
			results.forEach((result, index) => {
				const deviceId = readIds[index];
				if (!deviceId) return;
				if (result.data) reads.inventory[deviceId] = result.data;
				else if (result.error)
					reads.state[deviceId] =
						toHubError(result.error).code === "forbidden"
							? "forbidden"
							: "failed";
				else if (result.isLoading) reads.state[deviceId] = "reading";
			});
			return reads;
		},
		[readIds],
	);
	const reads = useQueries({
		queries: readIds.map((deviceId) => queries.certInventory(hub, deviceId)),
		combine,
	});

	const inventory = useMemo(
		() =>
			all.data
				? Object.fromEntries(all.data.map((row) => [row.device_id, row]))
				: reads.inventory,
		[all.data, reads.inventory],
	);
	const fleet = useMemo(
		() =>
			buildCertificateFleet({
				devices,
				me: input.me,
				now,
				inventory,
				fleetKnown: all.data !== undefined,
				perDevice: reads.state,
				live: input.live,
			}),
		[devices, input.me, input.live, now, inventory, all.data, reads.state],
	);

	const queryClient = useQueryClient();
	const refetchList = list.refetch;
	const refetchAll = all.refetch;
	const refresh = useCallback(async () => {
		const reports = deviceKeys.certInventoryAll(hub.scopeKey);
		await Promise.all([
			refetchList(),
			refetchAll(),
			...(perDevice ? [queryClient.refetchQueries({ queryKey: reports })] : []),
		]);
		const failure = [deviceKeys.list(hub.scopeKey), reports]
			.map((queryKey) => queryClient.getQueryState(queryKey)?.error)
			.find(Boolean);
		return failure ? toHubError(failure) : undefined;
	}, [refetchList, refetchAll, perDevice, queryClient, hub.scopeKey]);

	const error = all.error ?? list.error;
	return useMemo(
		() => ({
			fleet,
			now,
			loaded: list.rows !== undefined && (all.data !== undefined || perDevice),
			freshness: perDevice ? list.freshness : all.freshness,
			perDevice,
			...(error ? { error } : {}),
			refresh,
		}),
		[
			fleet,
			now,
			list.rows,
			list.freshness,
			all.data,
			all.freshness,
			perDevice,
			error,
			refresh,
		],
	);
}

/* Organisation authorities: kept in this app's storage per account, hub and profile. */

export interface AuthorityStore {
	authorities: LocalCertificateAuthority[];
	state: "loading" | "ready" | "failed";
	scope: DeviceAccountScope;
	crypto(): Promise<DeviceCrypto>;
	reload(): Promise<void>;
	/**
	 * `add` and `replace` run through the action layer (gate, busy state and an
	 * inline result under `authorityResultKey`); `label` is the verb + object
	 * of that result. False: nothing was written.
	 */
	add(authority: LocalCertificateAuthority, label: string): Promise<boolean>;
	replace(
		previous: LocalCertificateAuthority,
		next: LocalCertificateAuthority,
		label: string,
	): Promise<boolean>;
	/** The plain delete, called from the caller's own confirmed action. */
	remove(authorityId: string): Promise<void>;
}

const NO_AUTHORITIES: LocalCertificateAuthority[] = [];

/** Which control group shows the results of an authority's actions. */
export const authorityResultKey = (authorityId: string) =>
	`org_ca_manage:${authorityId}`;

export function useAuthorities(): AuthorityStore {
	const workspace = useDeviceWorkspace();
	const queryClient = useQueryClient();
	const actions = useDeviceAction();
	const { scope, crypto } = workspace.deps;
	const queryKey = useMemo(
		() => ["devices", workspace.scopeKey, "authorities"] as const,
		[workspace.scopeKey],
	);
	const query = useQuery({
		queryKey,
		queryFn: () => readCertificateAuthorities(scope),
		staleTime: Number.POSITIVE_INFINITY,
		retry: false,
		meta: { persist: false },
	});
	const reload = useCallback(async () => {
		await queryClient.invalidateQueries({ queryKey });
		await workspace.local.reload().catch(() => undefined);
	}, [queryClient, queryKey, workspace.local]);

	const run = actions.run;
	const write = useCallback(
		async (authorityId: string, label: string, call: () => Promise<void>) => {
			const outcome = await run({
				action: "org_ca_manage",
				label,
				resultKey: authorityResultKey(authorityId),
				call: async () => {
					await call();
					await reload();
				},
			});
			return outcome.status === "done";
		},
		[run, reload],
	);

	return useMemo(
		() => ({
			authorities: query.data ?? NO_AUTHORITIES,
			state: query.data ? "ready" : query.error ? "failed" : "loading",
			scope,
			crypto,
			reload,
			add: (authority, label) =>
				write(authority.public_bundle.authority_id, label, () =>
					addCertificateAuthority(scope, authority),
				),
			replace: (previous, next, label) =>
				write(previous.public_bundle.authority_id, label, () =>
					replaceCertificateAuthority(scope, previous, next),
				),
			remove: async (authorityId) => {
				await removeCertificateAuthority(scope, authorityId);
				await reload();
			},
		}),
		[query.data, query.error, scope, crypto, reload, write],
	);
}

/** The signing-key half of an envelope: what this computer keeps. The root vault stays in the backup file. */
export function localAuthority(
	envelope: CertificateAuthorityEnvelope,
): LocalCertificateAuthority {
	return {
		public_bundle: envelope.public_bundle,
		vault: Uint8Array.from(envelope.vault),
	};
}

/** The file's text, or null when it is larger than an authority backup can be. */
export async function readBackupFile(file: File): Promise<string | null> {
	if (file.size > MAX_AUTHORITY_BACKUP_BYTES) return null;
	return file.text();
}

export function saveFile(name: string, content: Blob | string) {
	const blob =
		typeof content === "string"
			? new Blob([content], { type: "application/x-pem-file" })
			: content;
	const url = URL.createObjectURL(blob);
	const link = document.createElement("a");
	link.href = url;
	link.download = name;
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/**
 * `const here = stillHere(); await work(); if (!here()) return;` — false once
 * the component unmounted or the account, hub or profile changed, so a late
 * completion is dropped.
 */
export function useStillHere(): () => () => boolean {
	const { scopeKey } = useDeviceWorkspace();
	const mounted = useRef(true);
	const current = useRef(scopeKey);
	current.current = scopeKey;
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	return useCallback(() => {
		const started = current.current;
		return () => mounted.current && current.current === started;
	}, []);
}

export interface Person {
	/** Undefined while the directory hasn't answered with an account: an id is never shown as a name. */
	name?: string;
	avatarUrl?: string;
}

/** A person through the app's batched account lookup. */
export function usePerson(userId: string | undefined): Person {
	const identity = useUserIdentity(userId);
	const name = identityName(identity, userId);
	return name ? { name, avatarUrl: identity.avatarUrl } : {};
}

/** The signed-in person's name as the host's sign-in knows it. */
export function useSignedInName(): string | undefined {
	const profile = useContext(AuthContext)?.user?.profile;
	if (!profile) return undefined;
	return [profile.name, profile.preferred_username, profile.email].find(
		Boolean,
	);
}
