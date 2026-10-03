"use client";

import {
	type RefObject,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type {
	SetupRoute,
	SetupStep,
} from "../../../../lib/device-management/model/types";
import { useDevicesRoute } from "../routing/use-devices-route";
import {
	type SetupDraft,
	type StepFacts,
	lockedOut,
	resolveStep,
} from "./setup-state";

export interface SetupSteps {
	step: SetupStep;
	goTo(step: SetupStep): void;
	/** Continue was pressed on the shown step with a field missing. */
	tried: boolean;
	markTried(): void;
	/** A link or Back asked for a step the registered device has locked. */
	lockedNote: boolean;
	dismissLockedNote(): void;
	rootRef: RefObject<HTMLDivElement | null>;
	headingRef: RefObject<HTMLHeadingElement | null>;
}

/**
 * The shown step: the URL's `step` (or the stored one) as far as the choices
 * made allow. Going to a step replaces the URL, and moves focus to the step's
 * heading at the top of the wizard.
 */
export function useSetupSteps(
	route: SetupRoute,
	facts: StepFacts,
	update: (patch: Partial<SetupDraft>) => void,
): SetupSteps {
	const { navigate } = useDevicesRoute();
	const [triedStep, setTriedStep] = useState<SetupStep>();
	const [localStep, setLocalStep] = useState<SetupStep>();
	const [lockDismissed, setLockDismissed] = useState(false);
	const rootRef = useRef<HTMLDivElement>(null);
	const headingRef = useRef<HTMLHeadingElement>(null);
	const { draft } = facts;
	const requested = localStep ?? route.step;
	const step = resolveStep(requested, facts);
	/* The hub stops listing a setup once its device started it; a URL change would then read as "not found". */
	const urlLocked = !!route.enrollmentId && draft.registeredAt !== undefined;
	const { enrollmentId } = route;

	const goTo = useCallback(
		(target: SetupStep) => {
			setTriedStep(undefined);
			setLockDismissed(true);
			update({ step: target });
			setLocalStep(urlLocked ? target : undefined);
			if (!urlLocked)
				navigate(
					{ screen: "setup", enrollmentId, step: target },
					{ replace: true },
				);
		},
		[update, navigate, enrollmentId, urlLocked],
	);

	const shown = useRef(step);
	useEffect(() => {
		if (shown.current === step) return;
		shown.current = step;
		rootRef.current?.scrollIntoView?.({ block: "start" });
		headingRef.current?.focus({ preventScroll: true });
	}, [step]);

	const markTried = useCallback(() => setTriedStep(step), [step]);
	const dismissLockedNote = useCallback(() => setLockDismissed(true), []);

	return {
		step,
		goTo,
		tried: triedStep === step,
		markTried,
		lockedNote: !lockDismissed && lockedOut(requested, draft),
		dismissLockedNote,
		rootRef,
		headingRef,
	};
}
