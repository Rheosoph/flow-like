"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../lib/api-url";
import { type IBackendState, useBackend } from "../../state/backend-state";
import type { ExecuteSqlResult } from "../../state/backend-state/query-state";
import { applyParamOverrides } from "./home-data-params";
import {
	type HomeDataConfig,
	buildHomeDataQuery,
	homeDataCrossAppPayload,
	homeDataSqlConfigError,
	loadHomeDataSourceContext,
} from "./home-data-query";

export interface HomeDataSnapshot {
	result: ExecuteSqlResult;
	refreshedAt: Date;
	cached: boolean;
	expiresAt: Date | null;
	warnings: string[];
	serverStale: boolean;
}

interface HomeDataLoad {
	key: string;
	snapshot: HomeDataSnapshot | null;
	loading: boolean;
	error: string | null;
}

const MAX_SNAPSHOTS = 100;
const snapshots = new Map<string, HomeDataSnapshot>();

function remember(key: string, snapshot: HomeDataSnapshot) {
	snapshots.delete(key);
	snapshots.set(key, snapshot);
	if (snapshots.size > MAX_SNAPSHOTS) {
		const oldest = snapshots.keys().next().value;
		if (oldest !== undefined) snapshots.delete(oldest);
	}
}

export function isHomeDataReady(config: HomeDataConfig): boolean {
	if (config.sourceKind === "sql") return !homeDataSqlConfigError(config);
	return Boolean(
		config.appId &&
			(config.sourceKind === "table"
				? config.table
				: config.sourceKind === "ontology"
					? config.ontologyId && config.objectType
					: config.queryId),
	);
}

async function fetchHomeData(
	backend: Pick<IBackendState, "dbState" | "graphState" | "queryState">,
	config: HomeDataConfig,
	viewerId: string | undefined,
	fresh: boolean,
): Promise<HomeDataSnapshot> {
	const context = await loadHomeDataSourceContext(backend, config, {
		viewerId,
	});
	const query = buildHomeDataQuery(config, context);
	if (config.sourceKind === "sql") {
		const data = await backend.queryState.executeCrossAppSql(
			homeDataCrossAppPayload(config, query, { fresh }),
		);
		return {
			result: data,
			refreshedAt: new Date(data.computed_at),
			cached: data.cached,
			expiresAt: data.expires_at ? new Date(data.expires_at) : null,
			warnings: data.warnings ?? [],
			serverStale: data.stale === true,
		};
	}
	return {
		result: await backend.queryState.executeSql(
			config.appId,
			query,
			config.scope === "personal",
		),
		refreshedAt: new Date(),
		cached: false,
		expiresAt: null,
		warnings: [],
		serverStale: false,
	};
}

/**
 * Loads a data widget's rows. The last rows for the same source stay visible while a
 * newer load runs or after one fails, marked stale, so a slow or broken source never
 * blanks a widget that already had data. `refresh` bypasses any shared result cache.
 */
export function useHomeData(
	config: HomeDataConfig,
	enabled = true,
	overrides?: Record<string, unknown>,
) {
	const backend = useBackend();
	const auth = useAuth();
	const viewerId = auth?.user?.profile?.sub;
	const [revision, setRevision] = useState(0);
	const [, setExpiryTick] = useState(0);
	const fresh = useRef(false);
	const signature = JSON.stringify(applyParamOverrides(config, overrides));
	const key = JSON.stringify([
		signature,
		viewerId,
		backend.profile?.id,
		getApiOrigin(backend.profile),
	]);
	const stable = useMemo(
		() => JSON.parse(signature) as HomeDataConfig,
		[signature],
	);
	const ready = isHomeDataReady(stable);
	const [load, setLoad] = useState<HomeDataLoad>(() => ({
		key,
		snapshot: snapshots.get(key) ?? null,
		loading: false,
		error: null,
	}));
	const refresh = useCallback(() => {
		fresh.current = true;
		setRevision((value) => value + 1);
	}, []);
	const poll = useCallback(() => setRevision((value) => value + 1), []);
	// biome-ignore lint/correctness/useExhaustiveDependencies: `revision` re-runs the load on refresh and polling.
	useEffect(() => {
		let cancelled = false;
		const active = enabled && ready;
		setLoad((current) => ({
			key,
			snapshot:
				(current.key === key ? current.snapshot : null) ??
				snapshots.get(key) ??
				null,
			loading: active,
			error: null,
		}));
		if (!active) return;
		const bypass = fresh.current;
		fresh.current = false;
		const timer = setTimeout(() => {
			void fetchHomeData(
				{
					dbState: backend.dbState,
					graphState: backend.graphState,
					queryState: backend.queryState,
				},
				stable,
				viewerId,
				bypass,
			).then(
				(snapshot) => {
					if (cancelled) return;
					remember(key, snapshot);
					setLoad({ key, snapshot, loading: false, error: null });
				},
				(reason) => {
					if (cancelled) return;
					setLoad((current) => ({
						...current,
						key,
						loading: false,
						error:
							reason instanceof Error
								? reason.message
								: "This data source could not be loaded. Check the source and your access.",
					}));
				},
			);
		}, 200);
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [
		key,
		revision,
		stable,
		ready,
		enabled,
		viewerId,
		backend.dbState,
		backend.graphState,
		backend.queryState,
	]);
	const current = load.key === key;
	const snapshot = current ? load.snapshot : (snapshots.get(key) ?? null);
	const loading = current ? load.loading : enabled && ready;
	const error = current ? load.error : null;
	const expiresAt = snapshot?.expiresAt?.getTime();
	useEffect(() => {
		if (!expiresAt) return;
		const remaining = expiresAt - Date.now();
		if (remaining <= 0) return;
		const timer = setTimeout(
			() => setExpiryTick((value) => value + 1),
			Math.min(remaining + 50, 2_147_000_000),
		);
		return () => clearTimeout(timer);
	}, [expiresAt]);
	useEffect(() => {
		if (!enabled || !ready || !stable.refreshSeconds) return;
		const timer = setInterval(() => {
			if (document.visibilityState === "visible" && !loading) poll();
		}, stable.refreshSeconds * 1000);
		return () => clearInterval(timer);
	}, [enabled, ready, stable.refreshSeconds, poll, loading]);
	const expired = expiresAt !== undefined && expiresAt <= Date.now();
	return {
		result: snapshot?.result ?? null,
		loading,
		error,
		refreshedAt: snapshot?.refreshedAt ?? null,
		cached: snapshot?.cached ?? false,
		expiresAt: snapshot?.expiresAt ?? null,
		warnings: snapshot?.warnings ?? [],
		serverStale: snapshot?.serverStale ?? false,
		/** Rows on screen are older than a load in flight, a failed load, or their cache lifetime. */
		stale: Boolean(snapshot) && (loading || Boolean(error) || expired),
		refresh,
		ready,
	};
}
