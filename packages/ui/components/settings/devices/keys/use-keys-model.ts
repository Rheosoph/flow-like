"use client";

import { useQueries, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import {
	HUB_CADENCE,
	deviceKeys,
	deviceQueryDefaults,
} from "../../../../lib/device-management/hub/queries";
import type {
	DeviceRow,
	Freshness,
} from "../../../../lib/device-management/model/types";
import { readAccountBackupStatus } from "../../../../lib/device-management/recovery";
import type { LocalSummary } from "../../../../lib/device-management/workspace/types";
import type { FreshnessStampProps } from "../primitives/freshness-stamp";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useAttentionState } from "../workspace/use-attention";
import { useAccountBackups, useDeviceRows } from "../workspace/use-hub";
import { useKeyChip, useLocalSummary } from "../workspace/use-keys";
import {
	type AccountBackupFact,
	type KeysModel,
	buildKeysModel,
} from "./keys-model";

/** Older hubs answer one backup per request; more devices than this stay "not checked" until asked. */
export const INTERIM_BACKUP_READS = 64;

/** A model freshness as stamp props (R5). */
export function stampOf(freshness: Freshness): FreshnessStampProps {
	const failure = freshness.error;
	return {
		source: freshness.src,
		age: freshness.age,
		...(freshness.at === undefined ? {} : { observedAt: freshness.at }),
		...(freshness.cadenceS === undefined
			? {}
			: { cadenceSec: freshness.cadenceS }),
		...(failure || freshness.dataFrom !== undefined
			? { error: { dataFrom: freshness.dataFrom, retryAt: failure?.retryAt } }
			: {}),
	};
}

export interface AccountBackupsRead {
	/** The hub has no backup list (BG25 interim): versions only, read per device. */
	interim: boolean;
	loading: boolean;
	/** The last read failed; the versions shown are from before. */
	failed: boolean;
	freshness: Freshness;
	slots?: { used: number; max: number };
	/** Unix seconds of the last completed comparison. */
	checkedAt?: number;
	/** Reads the account's backups again and resolves once every read settled. */
	check(): Promise<{ ok: boolean }>;
}

export interface KeysRead {
	model: KeysModel;
	backups: AccountBackupsRead;
	local: LocalSummary;
	/** `GET /devices` state, for the block stamps and the not-loaded state. */
	devices: { loaded: boolean; freshness: Freshness };
}

function interimIds(
	devices: readonly DeviceRow[],
	local: LocalSummary,
	only: readonly string[] | undefined,
): string[] {
	const held = new Set(local.vaults.map((vault) => vault.deviceId));
	return devices
		.filter(
			(row) =>
				row.status !== "revoked" &&
				row.relationship !== "cloud_approval" &&
				(!only || only.includes(row.device_id)),
		)
		.sort(
			(a, b) =>
				Number(held.has(b.device_id)) - Number(held.has(a.device_id)) ||
				a.name.localeCompare(b.name),
		)
		.slice(0, INTERIM_BACKUP_READS)
		.map((row) => row.device_id);
}

/**
 * Keys on this computer joined with the account backups on the hub (P4 + P1).
 * `interimFor` narrows the older-hub reads to the devices a view shows (the
 * device Keys tab reads one).
 */
export function useKeysModel(
	options: { interimFor?: readonly string[] } = {},
): KeysRead {
	const workspace = useDeviceWorkspace();
	const queryClient = useQueryClient();
	const { input } = useAttentionState();
	const rows = useDeviceRows();
	const list = useAccountBackups();
	const local = useLocalSummary();
	const { sessions } = useKeyChip();
	const { hub } = workspace;
	const me = workspace.deps.scope.account;
	const devices = input.devices;
	const devicesLoaded = rows.rows !== undefined;

	const interim = list.missingOnHub;
	const only = options.interimFor?.join("|");
	const ids = useMemo(
		() =>
			interim
				? interimIds(
						devices,
						local,
						only === undefined ? undefined : only.split("|"),
					)
				: [],
		[interim, devices, local, only],
	);
	const perDevice = useQueries({
		queries: ids.map((deviceId) => ({
			queryKey: deviceKeys.accountBackup(hub.scopeKey, deviceId),
			queryFn: async ({ signal }: { signal: AbortSignal }) =>
				(await readAccountBackupStatus(
					hub.api,
					hub.profile,
					deviceId,
					signal,
				)) ?? { revision: 0 },
			...deviceQueryDefaults(HUB_CADENCE.accountBackups),
		})),
	});

	const interimStamp = perDevice.map((query) => query.dataUpdatedAt).join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `interimStamp` stands for the query results
	const interimFacts = useMemo(() => {
		const facts = new Map<string, AccountBackupFact>();
		ids.forEach((deviceId, index) => {
			const data = perDevice[index]?.data;
			if (data) facts.set(deviceId, { revision: data.revision });
		});
		return facts;
	}, [ids, interimStamp]);

	const listFacts = useMemo(() => {
		if (!list.data) return undefined;
		return new Map<string, AccountBackupFact>(
			list.data.vaults.map((vault) => [
				vault.key_id,
				{ revision: vault.revision, updatedAt: vault.updated_at },
			]),
		);
	}, [list.data]);

	const model = useMemo(
		() =>
			buildKeysModel({
				me,
				devices,
				devicesLoaded,
				keys: sessions,
				local,
				live: (deviceId) => input.live[deviceId]?.state,
				accountBackup: (deviceId) =>
					listFacts
						? (listFacts.get(deviceId) ?? { revision: 0 })
						: interimFacts.get(deviceId),
				accessRequests: input.accessRequests ?? [],
			}),
		[
			me,
			devices,
			devicesLoaded,
			sessions,
			local,
			listFacts,
			interimFacts,
			input.live,
			input.accessRequests,
		],
	);

	const interimLoading = perDevice.some((query) => query.isLoading);
	const interimFailed = perDevice.some((query) => query.isError);
	const interimAt = perDevice.reduce(
		(oldest, query) =>
			query.dataUpdatedAt ? Math.min(oldest, query.dataUpdatedAt) : oldest,
		Number.POSITIVE_INFINITY,
	);
	const refetchList = list.refetch;
	const check = useCallback(async () => {
		if (!interim) {
			await refetchList();
			const state = queryClient.getQueryState(
				deviceKeys.accountBackups(hub.scopeKey),
			);
			return { ok: state?.status === "success" };
		}
		const results = await Promise.all(
			ids.map((deviceId) =>
				queryClient
					.refetchQueries({
						queryKey: deviceKeys.accountBackup(hub.scopeKey, deviceId),
						exact: true,
					})
					.then(
						() =>
							queryClient.getQueryState(
								deviceKeys.accountBackup(hub.scopeKey, deviceId),
							)?.status === "success",
					),
			),
		);
		return { ok: results.every(Boolean) };
	}, [interim, refetchList, queryClient, hub.scopeKey, ids]);

	const backups = useMemo<AccountBackupsRead>(() => {
		const offsetMs = (workspace.clock.hubOffsetS ?? 0) * 1000;
		const checkedAt = interim
			? Number.isFinite(interimAt)
				? Math.floor((interimAt - offsetMs) / 1000)
				: undefined
			: list.freshness.at;
		return {
			interim,
			loading: interim ? interimLoading : list.loading,
			failed: interim ? interimFailed : list.error !== undefined,
			freshness: list.freshness,
			...(list.data
				? { slots: { used: list.data.used, max: list.data.max } }
				: {}),
			...(checkedAt === undefined ? {} : { checkedAt }),
			check,
		};
	}, [
		workspace,
		interim,
		interimAt,
		interimLoading,
		interimFailed,
		list.freshness,
		list.loading,
		list.error,
		list.data,
		check,
	]);

	return useMemo(
		() => ({
			model,
			backups,
			local,
			devices: { loaded: devicesLoaded, freshness: rows.freshness },
		}),
		[model, backups, local, devicesLoaded, rows.freshness],
	);
}
