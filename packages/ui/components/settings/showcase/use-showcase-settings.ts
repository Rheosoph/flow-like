"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { apiErrorMessage } from "../../../lib/api-error";
import type { IAppShowcaseSettings } from "../../../lib/schema/app/showcase";
import { useBackend } from "../../../state/backend-state";
import { pruneSettings } from "./showcase-settings-model";

export type ShowcaseSaveState = "idle" | "saving" | "saved" | "error";

const SAVE_DEBOUNCE_MS = 400;

interface Allowlists {
	tables?: readonly string[];
	events?: readonly string[];
}

/**
 * Optimistic, debounced autosave: every change lands in local state at once and
 * one PUT carries the full settings. A failed save keeps the local state.
 */
export function useShowcaseSettings(appId: string, lists: Allowlists) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const remote = useInvoke(
		backend.appState.getShowcaseSettings,
		backend.appState,
		[appId],
		!!appId,
	);
	const [settings, setSettings] = useState<IAppShowcaseSettings | null>(null);
	const [saveState, setSaveState] = useState<ShowcaseSaveState>("idle");

	const latest = useRef<IAppShowcaseSettings | null>(null);
	const listsRef = useRef(lists);
	listsRef.current = lists;
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const inflight = useRef(false);
	const rerun = useRef(false);
	const mounted = useRef(true);

	useEffect(() => {
		if (remote.data && latest.current === null) {
			latest.current = remote.data;
			setSettings(remote.data);
		}
	}, [remote.data]);

	const flush = useCallback(async () => {
		timer.current = null;
		if (!latest.current) return;
		if (inflight.current) {
			rerun.current = true;
			return;
		}
		inflight.current = true;
		const payload = pruneSettings(
			latest.current,
			listsRef.current.tables,
			listsRef.current.events,
		);
		if (mounted.current) setSaveState("saving");
		try {
			await backend.appState.updateShowcaseSettings(appId, payload);
			await Promise.all([
				invalidate(backend.appState.getShowcase, [appId]),
				invalidate(backend.appState.getShowcaseSettings, [appId]),
			]);
			if (mounted.current) setSaveState("saved");
		} catch (error) {
			if (mounted.current) setSaveState("error");
			toast.error(
				apiErrorMessage(
					error,
					t("showcaseSettingSaveFailed", "Could not save the store page"),
				),
			);
		} finally {
			inflight.current = false;
			if (rerun.current) {
				rerun.current = false;
				void flush();
			}
		}
	}, [appId, backend.appState, invalidate, t]);

	const update = useCallback(
		(next: IAppShowcaseSettings) => {
			latest.current = next;
			setSettings(next);
			if (timer.current) clearTimeout(timer.current);
			timer.current = setTimeout(() => void flush(), SAVE_DEBOUNCE_MS);
		},
		[flush],
	);

	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
			if (timer.current) {
				clearTimeout(timer.current);
				void flush();
			}
		};
	}, [flush]);

	return {
		settings,
		update,
		saveState,
		isLoading: remote.isLoading,
		error: remote.error,
		refetch: remote.refetch,
	};
}
