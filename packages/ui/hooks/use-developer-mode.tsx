"use client";

import { useCallback, useEffect, useRef } from "react";
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { useBackend, useSignedIn } from "../state/backend-state";
import { useInvalidateInvoke, useInvoke } from "./use-invoke";

interface DeveloperModeMirror {
	localValue: boolean;
	/** An explicit choice not yet confirmed by the hub — kept until it can be
	 * pushed, so a toggle made while signed out survives the next login. */
	pendingSync: boolean;
	setLocalValue: (value: boolean, pendingSync: boolean) => void;
	clearPendingSync: () => void;
}

const useDeveloperModeMirror = create<DeveloperModeMirror>()(
	persist(
		(set) => ({
			localValue: false,
			pendingSync: false,
			setLocalValue: (value, pendingSync) =>
				set({ localValue: value, pendingSync }),
			clearPendingSync: () => set({ pendingSync: false }),
		}),
		{
			name: "developer-mode",
			storage: createJSONStorage(() => localStorage),
		},
	),
);

/**
 * Per-user developer mode. The authoritative value lives on the user row
 * (`devMode`, read via `GET /user/info`, written via `PUT /user/info`) so it
 * follows the account across devices; a localStorage mirror covers sessions
 * where the hub is unreachable.
 *
 * Toggling only marks the mirror dirty; the reconciler effect is the single
 * writer towards the hub. While dirty, the local value wins the read, so the
 * UI flips instantly and an offline choice is pushed up once `getInfo`
 * succeeds after login instead of being clobbered by the server default.
 *
 * Named `useDeveloperMode` rather than `useDevMode` because the widget
 * builder already has an unrelated `devMode` flag (raw-JSON panel).
 */
export function useDeveloperMode() {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const signedIn = useSignedIn();
	// This hook mounts app-wide, so an ungated query re-runs `getInfo` (and its
	// retry) on every route while signed out, where it can only fail.
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		signedIn,
	);
	const localValue = useDeveloperModeMirror((state) => state.localValue);
	const pendingSync = useDeveloperModeMirror((state) => state.pendingSync);
	const setLocalValue = useDeveloperModeMirror((state) => state.setLocalValue);
	const clearPendingSync = useDeveloperModeMirror(
		(state) => state.clearPendingSync,
	);
	const syncing = useRef(false);

	const serverValue = info.data?.dev_mode;

	useEffect(() => {
		if (serverValue === undefined) return;
		if (!pendingSync) {
			if (serverValue !== localValue) setLocalValue(serverValue, false);
			return;
		}
		if (serverValue === localValue) {
			clearPendingSync();
			return;
		}
		if (syncing.current) return;
		syncing.current = true;
		backend.userState
			.updateUser({ dev_mode: localValue })
			.then(() => invalidate(backend.userState.getInfo, []))
			.catch(() => {
				// Hub unreachable — retried on the next getInfo refetch.
			})
			.finally(() => {
				syncing.current = false;
			});
	}, [
		serverValue,
		localValue,
		pendingSync,
		setLocalValue,
		clearPendingSync,
		backend.userState,
		invalidate,
	]);

	const setDeveloperMode = useCallback(
		(value: boolean) => setLocalValue(value, true),
		[setLocalValue],
	);

	return {
		developerMode: pendingSync ? localValue : (serverValue ?? localValue),
		setDeveloperMode,
	};
}
