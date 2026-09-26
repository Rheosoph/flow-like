"use client";

import {
	type RuntimeVariableValue,
	RuntimeVariablesProvider,
	type StoredRuntimeVariable,
} from "@flow-like/flow-like-ui";
import { liveQuery } from "dexie";
import { useCallback, useMemo } from "react";
import {
	deleteRuntimeVar,
	deleteRuntimeVarsForApp,
	getRuntimeVarsForApp,
	hasAllRuntimeVars,
	runtimeVarsDB,
	setRuntimeVars,
} from "../lib/runtime-vars-db";

interface RuntimeVariablesProviderComponentProps {
	children: React.ReactNode;
}

export function RuntimeVariablesProviderComponent({
	children,
}: RuntimeVariablesProviderComponentProps) {
	const getValues = useCallback(
		async (appId: string): Promise<Map<string, RuntimeVariableValue>> => {
			const values = await getRuntimeVarsForApp(appId);
			const map = new Map<string, RuntimeVariableValue>();
			for (const v of values) {
				map.set(v.variableId, {
					variableId: v.variableId,
					value: v.value,
				});
			}
			return map;
		},
		[],
	);

	const saveValues = useCallback(
		async (
			appId: string,
			boardId: string,
			values: Array<{
				variableId: string;
				variableName: string;
				value: number[];
				isSecret: boolean;
			}>,
		): Promise<void> => {
			await setRuntimeVars(appId, boardId, values);
		},
		[],
	);

	const hasAllValues = useCallback(
		async (appId: string, variableIds: string[]): Promise<boolean> => {
			return hasAllRuntimeVars(appId, variableIds);
		},
		[],
	);

	const listValues = useCallback(
		async (appId: string): Promise<StoredRuntimeVariable[]> => {
			const values = await getRuntimeVarsForApp(appId);
			return values.map(toStored);
		},
		[],
	);

	const deleteValue = useCallback(
		async (appId: string, variableId: string): Promise<void> => {
			await deleteRuntimeVar(appId, variableId);
		},
		[],
	);

	const deleteValues = useCallback(
		async (appId: string, variableIds?: string[]): Promise<void> => {
			if (!variableIds) {
				await deleteRuntimeVarsForApp(appId);
				return;
			}
			await runtimeVarsDB.values.bulkDelete(
				variableIds.map((id) => `${appId}:${id}`),
			);
		},
		[],
	);

	const subscribe = useCallback(
		(
			appId: string,
			onChange: (values: StoredRuntimeVariable[]) => void,
		): (() => void) => {
			const observable = liveQuery(() => getRuntimeVarsForApp(appId));
			const subscription = observable.subscribe({
				next: (values) => onChange(values.map(toStored)),
				error: () => onChange([]),
			});
			return () => subscription.unsubscribe();
		},
		[],
	);

	const contextValue = useMemo(
		() => ({
			getValues,
			saveValues,
			hasAllValues,
			listValues,
			deleteValue,
			deleteValues,
			subscribe,
		}),
		[
			getValues,
			saveValues,
			hasAllValues,
			listValues,
			deleteValue,
			deleteValues,
			subscribe,
		],
	);

	return (
		<RuntimeVariablesProvider value={contextValue}>
			{children}
		</RuntimeVariablesProvider>
	);
}

function toStored(value: {
	variableId: string;
	value: number[];
	boardId: string;
	variableName: string;
	isSecret: boolean;
	updatedAt: string;
}): StoredRuntimeVariable {
	return {
		variableId: value.variableId,
		value: value.value,
		boardId: value.boardId,
		variableName: value.variableName,
		isSecret: value.isSecret,
		updatedAt: value.updatedAt,
	};
}
