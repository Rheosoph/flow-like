"use client";

import {
	type RuntimeVariableValue,
	RuntimeVariablesProvider,
} from "@flow-like/flow-like-ui";
import { useCallback, useMemo } from "react";
import {
	getRuntimeVarsForApp,
	hasAllRuntimeVars,
	setRuntimeVar,
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
			for (const v of values) {
				await setRuntimeVar(
					appId,
					boardId,
					v.variableId,
					v.variableName,
					v.value,
					v.isSecret,
				);
			}
		},
		[],
	);

	const hasAllValues = useCallback(
		async (appId: string, variableIds: string[]): Promise<boolean> => {
			return hasAllRuntimeVars(appId, variableIds);
		},
		[],
	);

	const contextValue = useMemo(
		() => ({
			getValues,
			saveValues,
			hasAllValues,
		}),
		[getValues, saveValues, hasAllValues],
	);

	return (
		<RuntimeVariablesProvider value={contextValue}>
			{children}
		</RuntimeVariablesProvider>
	);
}
