"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import {
	homeWidgetParamsKey,
	readHomeWidgetParams,
	writeHomeWidgetParams,
} from "../../db/home-widget-params-db";

/** A viewer's own values for a widget's interactive parameters, kept in IndexedDB and never in the layout. */
export function useHomeParamOverrides(widgetId: string) {
	const auth = useAuth();
	const key = useMemo(
		() => homeWidgetParamsKey(auth?.user?.profile?.sub, widgetId),
		[auth?.user?.profile?.sub, widgetId],
	);
	const [state, setState] = useState<{
		key: string;
		values: Record<string, unknown>;
	}>({ key: "", values: {} });
	const latest = useRef(state);
	latest.current = state;
	useEffect(() => {
		let active = true;
		void readHomeWidgetParams(key).then((values) => {
			if (active) setState({ key, values });
		});
		return () => {
			active = false;
		};
	}, [key]);
	const commit = useCallback(
		(values: Record<string, unknown>) => {
			setState({ key, values });
			void writeHomeWidgetParams(key, values);
		},
		[key],
	);
	const set = useCallback(
		(name: string, value: unknown) => {
			const base = latest.current.key === key ? latest.current.values : {};
			const next = { ...base };
			if (value === undefined) delete next[name];
			else next[name] = value;
			commit(next);
		},
		[key, commit],
	);
	const reset = useCallback(() => commit({}), [commit]);
	return {
		loaded: state.key === key,
		overrides: state.key === key ? state.values : undefined,
		set,
		reset,
	};
}
