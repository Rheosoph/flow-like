"use client";

import { useCallback, useEffect, useState } from "react";

export type DashboardMode = "launch" | "control";
export type DashboardModePreference = "auto" | DashboardMode;

const STORAGE_PREFIX = "flow-like:dashboard-mode:";

function readPreference(appId: string | undefined): DashboardModePreference {
	if (!appId || typeof window === "undefined") return "auto";
	const stored = window.localStorage.getItem(`${STORAGE_PREFIX}${appId}`);
	return stored === "launch" || stored === "control" ? stored : "auto";
}

/**
 * Picks which dashboard a project gets.
 *
 * A project is on the Launch Path until it has actually done something —
 * the switch to Mission Control happens on the first successful run, because
 * that is the point where "what do I do next" turns into "is it healthy".
 * The user can pin either mode; the pin is per project and per device.
 */
export function useDashboardMode(
	appId: string | undefined,
	hasEverSucceeded: boolean,
	runsReady: boolean,
): {
	mode: DashboardMode;
	preference: DashboardModePreference;
	autoMode: DashboardMode;
	setPreference: (next: DashboardModePreference) => void;
} {
	const [preference, setPreferenceState] =
		useState<DashboardModePreference>("auto");

	useEffect(() => {
		setPreferenceState(readPreference(appId));
	}, [appId]);

	const setPreference = useCallback(
		(next: DashboardModePreference) => {
			setPreferenceState(next);
			if (!appId || typeof window === "undefined") return;
			if (next === "auto") {
				window.localStorage.removeItem(`${STORAGE_PREFIX}${appId}`);
				return;
			}
			window.localStorage.setItem(`${STORAGE_PREFIX}${appId}`, next);
		},
		[appId],
	);

	// Until the run aggregation resolves, stay on Launch Path rather than
	// flashing an empty Mission Control at a project that has plenty of history.
	const autoMode: DashboardMode =
		runsReady && hasEverSucceeded ? "control" : "launch";

	return {
		mode: preference === "auto" ? autoMode : preference,
		preference,
		autoMode,
		setPreference,
	};
}
