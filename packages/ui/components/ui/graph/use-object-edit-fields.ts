"use client";

import { type UseQueryResult, useQueries } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useInvalidateInvoke } from "../../../hooks/use-invoke";
import {
	type ObjectEditField,
	objectEditFields,
} from "../../../lib/ontology-object-edit";
import { useBackend, useBackendReady } from "../../../state/backend-state";

type TableFields = ReadonlyMap<string, ObjectEditField>;

export interface ObjectEditFieldsState {
	byTable: ReadonlyMap<string, TableFields>;
	loading: boolean;
	failed: ReadonlySet<string>;
}

const TABLE_KEY_SEPARATOR = "\u0000";

/**
 * Column types of the tables behind editable ontology objects, typed for the
 * property editors. Only local sources may pass `enabled`: a remote import's
 * tables live in another project and must never be fetched from this one.
 */
export function useObjectEditFields(
	appId: string | undefined,
	tables: readonly string[],
	userScoped: boolean,
	enabled: boolean,
): ObjectEditFieldsState {
	const { dbState } = useBackend();
	const backendReady = useBackendReady();
	const tableKey = [...new Set(tables)].sort().join(TABLE_KEY_SEPARATOR);
	const distinct = useMemo(
		() => (tableKey ? tableKey.split(TABLE_KEY_SEPARATOR) : []),
		[tableKey],
	);
	const active = enabled && Boolean(appId) && backendReady;

	const combine = useCallback(
		(results: UseQueryResult<TableFields, Error>[]): ObjectEditFieldsState => {
			const byTable = new Map<string, TableFields>();
			const failed = new Set<string>();
			results.forEach((result, index) => {
				const table = distinct[index];
				if (result.data) byTable.set(table, result.data);
				if (result.isError) failed.add(table);
			});
			return {
				byTable,
				failed,
				loading: results.some((result) => result.isLoading),
			};
		},
		[distinct],
	);

	return useQueries({
		queries: distinct.map((table) => ({
			queryKey: ["ontologyEditSchema", appId, table, userScoped],
			queryFn: async (): Promise<TableFields> =>
				objectEditFields(
					await dbState.getSchema(appId as string, table, userScoped),
				),
			enabled: active,
			retry: 1,
		})),
		combine,
	});
}

/** Refreshes any open Table view of `table` after its rows changed elsewhere. */
export function useInvalidateOntologyTable(): (
	appId: string,
	table: string,
) => Promise<void> {
	const { dbState } = useBackend();
	const invalidate = useInvalidateInvoke();
	return useCallback(
		async (appId: string, table: string) => {
			await Promise.allSettled([
				invalidate(dbState.listItems, [appId, table]),
				invalidate(dbState.countItems, [appId, table]),
				invalidate(dbState.databaseHistory, [appId, table]),
			]);
		},
		[dbState, invalidate],
	);
}
