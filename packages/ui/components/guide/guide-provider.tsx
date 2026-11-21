"use client";

import { usePathname } from "next/navigation";
import { useEffect, useRef } from "react";
import { useDeveloperMode } from "../../hooks/use-developer-mode";
import { useInvoke } from "../../hooks/use-invoke";
import { GUIDE_EVENT, readGuideEvent } from "../../lib/guide/guide-events";
import { currentStep, routeMatches } from "../../lib/guide/guide-model";
import { GUIDE_TOURS } from "../../lib/guide/guide-tours";
import { cn } from "../../lib/utils";
import { useBackend, useBackendReady } from "../../state/backend-state";
import { useGuideStore } from "../../state/guide-state";
import { GuideCard } from "./guide-card";
import { useGuideCopy } from "./guide-copy";
import { useGuideDocked } from "./guide-dock";
import { GuideSpotlight } from "./guide-spotlight";
import { GuideWelcomeDialog } from "./guide-welcome-dialog";

/** Routes that own the whole screen before the app is usable. */
const QUIET_ROUTES = ["/onboarding", "/join", "/callback", "/login", "/notch"];

/** Set by the retired slide tutorial; people who saw it are not new. */
const LEGACY_TUTORIAL_KEY = "tutorial-finished-new";

function sawLegacyTutorial(): boolean {
	try {
		return localStorage.getItem(LEGACY_TUTORIAL_KEY) === "true";
	} catch {
		return false;
	}
}

/**
 * Decides once, on Home, whether this is a new user: no earlier tutorial and
 * no apps in the current profile. Onboarding can force the welcome instead.
 */
function useWelcomeDecision(pathname: string) {
	const welcome = useGuideStore((state) => state.welcome);
	const decideWelcome = useGuideStore((state) => state.decideWelcome);
	const backend = useBackend();
	const ready = useBackendReady();
	const deciding = welcome === "unknown" && pathname === "/" && ready;
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		deciding,
	);

	useEffect(() => {
		if (!deciding) return;
		if (sawLegacyTutorial()) {
			decideWelcome(false);
			return;
		}
		if (profile.isSuccess) {
			decideWelcome((profile.data?.apps?.length ?? 0) === 0);
		} else if (profile.isError) {
			decideWelcome(false);
		}
	}, [
		deciding,
		profile.isSuccess,
		profile.isError,
		profile.data,
		decideWelcome,
	]);
}

function useGuideSignals(pathname: string) {
	const signal = useGuideStore((state) => state.signal);
	const locate = useGuideStore((state) => state.locate);
	const tour = useGuideStore((state) => state.run?.tour);
	const step = useGuideStore((state) => state.run?.step);

	useEffect(() => {
		const onEvent = (event: Event) => {
			const name = readGuideEvent(event);
			if (name) signal({ kind: "event", name });
		};
		window.addEventListener(GUIDE_EVENT, onEvent);
		return () => window.removeEventListener(GUIDE_EVENT, onEvent);
	}, [signal]);

	useEffect(() => {
		if (tour === undefined || step === undefined) return;
		locate(pathname);
	}, [pathname, tour, step, locate]);
}

/**
 * The builder tour walks through Flows and the board, which developer mode
 * unhides. Starting it turns developer mode on once; turning it off again
 * mid-tour is respected.
 */
function useBuilderDeveloperMode() {
	const tour = useGuideStore((state) => state.run?.tour);
	const { developerMode, setDeveloperMode } = useDeveloperMode();
	const previous = useRef(tour);

	useEffect(() => {
		const started = tour === "build" && previous.current !== "build";
		previous.current = tour;
		if (started && !developerMode) setDeveloperMode(true);
	}, [tour, developerMode, setDeveloperMode]);
}

function ActiveSpotlight({ pathname }: Readonly<{ pathname: string }>) {
	const copy = useGuideCopy();
	const run = useGuideStore((state) => state.run);
	const step = run ? currentStep(GUIDE_TOURS[run.tour], run) : undefined;
	if (!step?.target) return null;
	if (step.route && !routeMatches(pathname, step.route)) return null;
	return <GuideSpotlight target={step.target} tag={copy.tag(step.tag)} />;
}

/**
 * Mount once per app, next to the other global overlays. It asks new users
 * the welcome question, follows what they do and points at the next control.
 */
export function GuideProvider() {
	const pathname = usePathname() ?? "/";
	const welcome = useGuideStore((state) => state.welcome);
	const visible = useGuideStore(
		(state) => state.run !== null || state.finished !== null,
	);
	const docked = useGuideDocked();
	const quiet = QUIET_ROUTES.some((route) => routeMatches(pathname, route));

	useWelcomeDecision(pathname);
	useGuideSignals(pathname);
	useBuilderDeveloperMode();

	if (quiet) return null;
	const onBoard = routeMatches(pathname, "/flow");
	return (
		<>
			<GuideWelcomeDialog open={welcome === "pending"} />
			<ActiveSpotlight pathname={pathname} />
			{visible && !docked && (
				<div
					className={cn(
						"fixed z-[60]",
						onBoard
							? "bottom-10 left-14"
							: "bottom-[calc(var(--fl-safe-bottom,0px)+5rem)] left-4 md:bottom-4",
					)}
				>
					<GuideCard variant="float" />
				</div>
			)}
		</>
	);
}
