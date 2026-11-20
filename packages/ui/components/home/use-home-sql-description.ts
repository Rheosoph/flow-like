"use client";

import { useEffect, useMemo, useState } from "react";
import { useBackend } from "../../state/backend-state";
import type {
	SqlParameterInfo,
	SqlParameterOptions,
	SqlQueryDescription,
} from "../../state/backend-state/query-state";

import {
	type HomeDataConfig,
	homeDataSqlConfigError,
	homeDataSqlSourcesPayload,
} from "./home-data-query";

const MAX_DESCRIPTIONS = 50;
const descriptions = new Map<string, SqlQueryDescription>();

function remember(key: string, description: SqlQueryDescription) {
	descriptions.delete(key);
	descriptions.set(key, description);
	if (descriptions.size > MAX_DESCRIPTIONS) {
		const oldest = descriptions.keys().next().value;
		if (oldest !== undefined) descriptions.delete(oldest);
	}
}

export interface SqlDescriptionState {
	description: SqlQueryDescription | null;
	loading: boolean;
	error: string | null;
}

/** Output columns and `$parameters` of a widget's SQL, debounced and cached per statement and sources. */
export function useHomeSqlDescription(
	config: Pick<HomeDataConfig, "sql" | "sources" | "sourceKind">,
	options: { debounceMs?: number; enabled?: boolean } = {},
): SqlDescriptionState {
	const backend = useBackend();
	const { debounceMs = 0, enabled = true } = options;
	const sources = useMemo(
		() => homeDataSqlSourcesPayload(config.sources),
		[config.sources],
	);
	const key = JSON.stringify([config.sql, sources]);
	const active =
		enabled &&
		config.sourceKind === "sql" &&
		homeDataSqlConfigError(config) === null;
	const [state, setState] = useState<SqlDescriptionState & { key: string }>(
		() => ({
			key,
			description: descriptions.get(key) ?? null,
			loading: false,
			error: null,
		}),
	);
	useEffect(() => {
		if (!active) {
			setState({ key, description: null, loading: false, error: null });
			return;
		}
		const cached = descriptions.get(key);
		if (cached) {
			setState({ key, description: cached, loading: false, error: null });
			return;
		}
		let cancelled = false;
		setState((current) => ({
			key,
			description: current.description,
			loading: true,
			error: null,
		}));
		const timer = setTimeout(() => {
			backend.queryState.describeCrossAppSql({ sql: config.sql, sources }).then(
				(description) => {
					if (cancelled) return;
					remember(key, description);
					setState({ key, description, loading: false, error: null });
				},
				(reason) => {
					if (cancelled) return;
					setState({
						key,
						description: null,
						loading: false,
						error:
							reason instanceof Error
								? reason.message
								: "The statement could not be analyzed.",
					});
				},
			);
		}, debounceMs);
		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [active, key, debounceMs, backend.queryState, config.sql, sources]);
	return {
		description: state.key === key ? state.description : null,
		loading: active && (state.key !== key || state.loading),
		error: state.key === key ? state.error : null,
	};
}

export function useHomeParamOptionsLoader(
	config: Pick<HomeDataConfig, "sources">,
) {
	const backend = useBackend();
	const sources = useMemo(
		() => homeDataSqlSourcesPayload(config.sources),
		[config.sources],
	);
	return useMemo(
		() =>
			(
				info: SqlParameterInfo,
				request: { search?: string; range?: boolean } = {},
			): Promise<SqlParameterOptions> => {
				const binding =
					info.bindings.find(
						(item) => item.operator === "eq" || item.operator === "in",
					) ?? info.bindings[0];
				if (!binding)
					return Promise.reject(new Error("This parameter is not bound."));
				return backend.queryState.crossAppSqlParameterOptions({
					sources,
					table: binding.table,
					column: binding.column,
					...request,
				});
			},
		[backend.queryState, sources],
	);
}
