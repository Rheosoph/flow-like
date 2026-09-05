"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../lib/api-url";
import { useBackend } from "../../state/backend-state";
import type { ExecuteSqlResult } from "../../state/backend-state/query-state";
import {
	type HomeDataConfig,
	type HomeDataSourceContext,
	buildHomeDataQuery,
	homeDataColumns,
	homeOntologyColumns,
} from "./home-data-query";

export function useHomeData(config: HomeDataConfig, enabled = true) {
	const backend = useBackend();
	const auth = useAuth();
	const viewerId = auth?.user?.profile?.sub;
	const [result, setResult] = useState<ExecuteSqlResult | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [refreshedAt, setRefreshedAt] = useState<Date | null>(null);
	const [revision, setRevision] = useState(0);
	const [requestKey, setRequestKey] = useState("");
	const signature = JSON.stringify(config);
	const identity = JSON.stringify([
		signature,
		viewerId,
		backend.profile?.id,
		getApiOrigin(backend.profile),
		revision,
	]);
	const stable = useMemo(
		() => JSON.parse(signature) as HomeDataConfig,
		[signature],
	);
	const ready = Boolean(
		stable.appId &&
			(stable.sourceKind === "table"
				? stable.table
				: stable.sourceKind === "ontology"
					? stable.ontologyId && stable.objectType
					: stable.queryId),
	);
	const refresh = useCallback(() => setRevision((value) => value + 1), []);
	useEffect(() => {
		let cancelled = false;
		setRequestKey(identity);
		setResult(null);
		setError(null);
		setRefreshedAt(null);
		if (!enabled || !ready) {
			setLoading(false);
			return;
		}
		setLoading(true);
		const timer = setTimeout(() => {
			void (async () => {
				try {
					const context: HomeDataSourceContext = { viewerId };
					const personal = stable.scope === "personal";
					if (stable.sourceKind === "ontology") {
						context.overlay = await backend.graphState.getOverlay(
							stable.appId,
							stable.ontologyId,
							personal,
						);
						context.columns = homeOntologyColumns(
							context.overlay,
							stable.objectType,
						);
					} else if (stable.sourceKind === "query") {
						context.savedQuery = await backend.queryState.getSavedQuery(
							stable.appId,
							stable.queryId,
							personal,
						);
					} else {
						context.columns = homeDataColumns(
							await backend.dbState.getSchema(
								stable.appId,
								stable.table,
								personal,
							),
						);
					}
					const query = buildHomeDataQuery(stable, context);
					if (cancelled) return;
					const data = await backend.queryState.executeSql(
						stable.appId,
						query,
						personal,
					);
					if (!cancelled) {
						setResult(data);
						setRefreshedAt(new Date());
					}
				} catch (reason) {
					if (!cancelled)
						setError(
							reason instanceof Error
								? reason.message
								: "This data source could not be loaded. Check the source and your access.",
						);
				} finally {
					if (!cancelled) setLoading(false);
				}
			})();
		}, 200);
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [
		stable,
		ready,
		enabled,
		identity,
		viewerId,
		backend.dbState,
		backend.graphState,
		backend.queryState,
	]);
	useEffect(() => {
		if (!enabled || !ready || !stable.refreshSeconds) return;
		const timer = setInterval(() => {
			if (document.visibilityState === "visible" && !loading) refresh();
		}, stable.refreshSeconds * 1000);
		return () => clearInterval(timer);
	}, [enabled, ready, stable.refreshSeconds, refresh, loading]);
	const current = requestKey === identity;
	return {
		result: current ? result : null,
		loading: current ? loading : enabled && ready,
		error: current ? error : null,
		refreshedAt: current ? refreshedAt : null,
		refresh,
		ready,
	};
}
