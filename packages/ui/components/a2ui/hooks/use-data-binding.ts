"use client";

import { useCallback, useContext, useMemo } from "react";
import { DataContext } from "../DataContext";
import type { BoundValue } from "../types";

export function useDataBinding(bound: BoundValue | undefined): unknown {
	const dataContext = useContext(DataContext);

	return useMemo(() => {
		if (!bound) return undefined;

		if ("literalString" in bound) {
			return bound.literalString;
		}

		if ("literalNumber" in bound) {
			return bound.literalNumber;
		}

		if ("literalBool" in bound) {
			return bound.literalBool;
		}

		if ("literalOptions" in bound) {
			return bound.literalOptions;
		}

		if ("literalJson" in bound) {
			try {
				return JSON.parse(bound.literalJson);
			} catch {
				return undefined;
			}
		}

		if ("path" in bound && dataContext) {
			const value = dataContext.get(bound.path);
			return value !== undefined ? value : bound.defaultValue;
		}

		return undefined;
	}, [bound, dataContext]);
}

export function useDataBindingSetter(
	bound: BoundValue | undefined,
): ((value: unknown) => void) | undefined {
	const dataContext = useContext(DataContext);

	return useMemo(() => {
		if (!bound || !("path" in bound) || !dataContext) {
			return undefined;
		}

		return (value: unknown) => {
			dataContext.set(bound.path, value);
		};
	}, [bound, dataContext]);
}

export function useBoundValue(
	bound: BoundValue | undefined,
): [unknown, ((value: unknown) => void) | undefined] {
	const value = useDataBinding(bound);
	const setter = useDataBindingSetter(bound);
	return [value, setter];
}

export function useDataPath(path: string): unknown {
	const dataContext = useContext(DataContext);
	return useMemo(() => dataContext?.get(path), [dataContext, path]);
}

export function useSetDataPath(): (path: string, value: unknown) => void {
	const dataContext = useContext(DataContext);

	return useCallback(
		(path: string, value: unknown) => {
			dataContext?.set(path, value);
		},
		[dataContext],
	);
}
