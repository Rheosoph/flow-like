"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import type {
	IBotApp,
	IBotRestrictions,
} from "../../../state/backend-state/bot-state";
import { normalizeRestrictions, sameScopes } from "../bot-restrictions-model";

export interface BotAppChange {
	appId: string;
	attached?: boolean;
	restrictions?: IBotRestrictions;
}

export interface BotAppDraft {
	apps: IBotApp[];
	loading: boolean;
	changes: BotAppChange[];
	setAttached: (appId: string, attached: boolean) => void;
	setRestrictions: (appId: string, restrictions: IBotRestrictions) => void;
	markAttachedSaved: (appId: string, attached: boolean) => void;
	markRestrictionsSaved: (
		appId: string,
		restrictions: IBotRestrictions,
	) => void;
}

interface AppDraftState {
	saved: Map<string, IBotApp>;
	entries: Map<string, IBotApp>;
	attached: Set<string>;
}

function snapshot(apps: readonly IBotApp[]): AppDraftState {
	const saved = new Map(structuredClone(apps).map((app) => [app.app_id, app]));
	return {
		saved,
		entries: new Map(saved),
		attached: new Set(saved.keys()),
	};
}

function newApp(appId: string): IBotApp {
	return { app_id: appId, added_at: "", owner_role: null };
}

function sameRestrictions(
	a: IBotRestrictions | undefined,
	b: IBotRestrictions | undefined,
) {
	const left = normalizeRestrictions(a);
	const right = normalizeRestrictions(b);
	const samePermissions =
		left.permissions === null || right.permissions === null
			? left.permissions === right.permissions
			: new Set(left.permissions).size === new Set(right.permissions).size &&
				left.permissions.every((permission) =>
					right.permissions?.includes(permission),
				);
	return samePermissions && sameScopes(left.scopes, right.scopes);
}

function changesFrom(state: AppDraftState | null): BotAppChange[] {
	if (!state) return [];
	const changes: BotAppChange[] = [];
	for (const [appId, app] of state.entries) {
		const saved = state.saved.get(appId);
		if (!state.attached.has(appId)) {
			if (saved) changes.push({ appId, attached: false });
			continue;
		}
		const restrictionsChanged = !sameRestrictions(
			saved?.restrictions,
			app.restrictions,
		);
		if (!saved || restrictionsChanged)
			changes.push({
				appId,
				...(!saved && { attached: true }),
				...(restrictionsChanged && {
					restrictions: normalizeRestrictions(app.restrictions),
				}),
			});
	}
	return changes;
}

/** App edits keep their opening snapshot until each requested change is saved. */
export function useBotAppDraft(
	savedApps: readonly IBotApp[] | undefined,
): BotAppDraft {
	const [state, setState] = useState<AppDraftState | null>(() =>
		savedApps === undefined ? null : snapshot(savedApps),
	);
	useEffect(() => {
		if (savedApps !== undefined)
			setState((current) => current ?? snapshot(savedApps));
	}, [savedApps]);

	const setAttached = useCallback((appId: string, attached: boolean) => {
		setState((current) => {
			if (!current || current.attached.has(appId) === attached) return current;
			const entries = new Map(current.entries);
			if (!entries.has(appId)) entries.set(appId, newApp(appId));
			const selected = new Set(current.attached);
			if (attached) selected.add(appId);
			else selected.delete(appId);
			return { ...current, entries, attached: selected };
		});
	}, []);

	const setRestrictions = useCallback(
		(appId: string, restrictions: IBotRestrictions) => {
			setState((current) => {
				const app = current?.entries.get(appId);
				if (
					!current ||
					!app ||
					sameRestrictions(app.restrictions, restrictions)
				)
					return current;
				const entries = new Map(current.entries);
				entries.set(appId, {
					...app,
					restrictions: structuredClone(restrictions),
				});
				return { ...current, entries };
			});
		},
		[],
	);

	const markAttachedSaved = useCallback((appId: string, attached: boolean) => {
		setState((current) => {
			if (!current) return current;
			const saved = new Map(current.saved);
			if (attached) {
				// A newly attached app starts with the owner's full role.
				if (!saved.has(appId))
					saved.set(appId, {
						...(current.entries.get(appId) ?? newApp(appId)),
						restrictions: undefined,
					});
			} else saved.delete(appId);
			return { ...current, saved };
		});
	}, []);

	const markRestrictionsSaved = useCallback(
		(appId: string, restrictions: IBotRestrictions) => {
			setState((current) => {
				const app = current?.saved.get(appId);
				if (!current || !app) return current;
				const saved = new Map(current.saved);
				saved.set(appId, {
					...app,
					restrictions: structuredClone(restrictions),
				});
				return { ...current, saved };
			});
		},
		[],
	);

	return {
		apps: useMemo(
			() =>
				state
					? [...state.entries.values()].filter((app) =>
							state.attached.has(app.app_id),
						)
					: [],
			[state],
		),
		loading: state === null,
		changes: useMemo(() => changesFrom(state), [state]),
		setAttached,
		setRestrictions,
		markAttachedSaved,
		markRestrictionsSaved,
	};
}
