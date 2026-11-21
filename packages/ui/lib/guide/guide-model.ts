import type { GuideEventName } from "./guide-events";

export type GuideTourId = "use" | "build";

export type GuideTrigger =
	| { readonly event: GuideEventName }
	| { readonly route: string };

/** The card's main button: click the ringed control for the user, or just move on. */
export type GuideAct = "click" | "next";

/** Short label on the ring around the target. */
export type GuideTag =
	| "click"
	| "type"
	| "rightClick"
	| "drag"
	| "pick"
	| "open";

export interface IGuideStep {
	readonly id: string;
	/** `data-guide` value of the real control this step points at. */
	readonly target?: string;
	readonly tag?: GuideTag;
	/** Pathname prefix the step happens on; elsewhere the card offers to go there. */
	readonly route?: string;
	readonly completeOn: readonly GuideTrigger[];
	/** The step only runs once this flag was raised by an event. */
	readonly requires?: string;
	/** Doing the next step's action also counts for this one. */
	readonly passable?: boolean;
	readonly act?: GuideAct;
}

export interface IGuideTour {
	readonly id: GuideTourId;
	readonly steps: readonly IGuideStep[];
	/** Events that raise a flag, whichever step is active. */
	readonly flags?: Partial<Record<GuideEventName, string>>;
}

export interface IGuideRun {
	readonly tour: GuideTourId;
	readonly step: number;
	readonly flags: readonly string[];
}

export type GuideSignal =
	| { readonly kind: "event"; readonly name: GuideEventName }
	| { readonly kind: "route"; readonly path: string };

export function routeMatches(path: string, route: string): boolean {
	if (route === "/") return path === "/";
	const base = route.replace(/\/+$/, "");
	return path === base || path.startsWith(`${base}/`);
}

function triggerMatches(trigger: GuideTrigger, signal: GuideSignal): boolean {
	if ("event" in trigger) {
		return signal.kind === "event" && signal.name === trigger.event;
	}
	return signal.kind === "route" && routeMatches(signal.path, trigger.route);
}

/** Moves past steps whose required flag never came up. */
export function settleRun(tour: IGuideTour, run: IGuideRun): IGuideRun {
	let step = run.step;
	while (step < tour.steps.length) {
		const required = tour.steps[step].requires;
		if (!required || run.flags.includes(required)) break;
		step += 1;
	}
	return step === run.step ? run : { ...run, step };
}

export function startRun(tour: IGuideTour): IGuideRun {
	return settleRun(tour, { tour: tour.id, step: 0, flags: [] });
}

export function currentStep(
	tour: IGuideTour,
	run: IGuideRun,
): IGuideStep | undefined {
	return tour.steps[run.step];
}

export function isRunFinished(tour: IGuideTour, run: IGuideRun): boolean {
	return run.step >= tour.steps.length;
}

function completes(step: IGuideStep | undefined, signal: GuideSignal) {
	return (
		step?.completeOn.some((trigger) => triggerMatches(trigger, signal)) ?? false
	);
}

/**
 * Applies one thing the user did. Only the active step can complete, so a
 * later step's event never skips ahead, except past a `passable` step whose
 * successor it completes. Flags are raised whatever step runs.
 */
export function applySignal(
	tour: IGuideTour,
	run: IGuideRun,
	signal: GuideSignal,
): IGuideRun {
	let next = run;
	if (signal.kind === "event") {
		const flag = tour.flags?.[signal.name];
		if (flag && !next.flags.includes(flag)) {
			next = { ...next, flags: [...next.flags, flag] };
		}
	}
	const step = currentStep(tour, next);
	if (!step) return next;
	if (completes(step, signal)) {
		return settleRun(tour, { ...next, step: next.step + 1 });
	}
	if (!step.passable) return next;
	const after = settleRun(tour, { ...next, step: next.step + 1 });
	if (!completes(currentStep(tour, after), signal)) return next;
	return settleRun(tour, { ...after, step: after.step + 1 });
}

/** Completes every step the current location already satisfies. */
export function applyLocation(
	tour: IGuideTour,
	run: IGuideRun,
	path: string,
): IGuideRun {
	let next = run;
	for (let guard = 0; guard < tour.steps.length; guard += 1) {
		const advanced = applySignal(tour, next, { kind: "route", path });
		if (advanced.step === next.step) return advanced;
		next = advanced;
	}
	return next;
}

/** Leaves the active step without doing it, for steps the user can't finish here. */
export function skipStep(tour: IGuideTour, run: IGuideRun): IGuideRun {
	if (isRunFinished(tour, run)) return run;
	return settleRun(tour, { ...run, step: run.step + 1 });
}

/** Steps the user will actually see, for the "2 of 7" counter. */
export function visibleProgress(
	tour: IGuideTour,
	run: IGuideRun,
): { readonly index: number; readonly total: number } {
	const optional = tour.steps.filter((step) => step.requires).length;
	const skippedBefore = tour.steps
		.slice(0, run.step)
		.filter(
			(step) => step.requires && !run.flags.includes(step.requires),
		).length;
	const raised = tour.steps.filter(
		(step) => step.requires && run.flags.includes(step.requires),
	).length;
	const total = tour.steps.length - optional + raised;
	return {
		index: Math.min(run.step - skippedBefore + 1, total),
		total,
	};
}
