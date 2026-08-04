"use client";

import { type ComponentPropsWithoutRef, forwardRef } from "react";
import { useForwardedRef } from "../../lib/use-forwarded-ref";
import { cn } from "../../lib/utils";
import { BubbleOrbFilm } from "./bubble-orb-film";
import type { FlowPilotOrbState } from "./flowpilot-orb-state";

export type FlowPilotBubbleOrbProps = ComponentPropsWithoutRef<"button"> & {
	/** What the assistant is doing. Defaults to `idle` for callers that don't track it. */
	orbState?: FlowPilotOrbState;
	/** Bump to fire the acknowledge burst (e.g. on a state change). */
	ackNonce?: number;
};

/**
 * Pure, inline FlowPilot orb button. It owns only the canonical shader and pointer interaction;
 * callers supply positioning, animation, click behavior, and any surrounding assistant state.
 */
export const FlowPilotBubbleOrb = forwardRef<
	HTMLButtonElement,
	FlowPilotBubbleOrbProps
>(function FlowPilotBubbleOrb(
	{
		className,
		children,
		type = "button",
		orbState = "idle",
		ackNonce = 0,
		"aria-label": ariaLabel = "Ask FlowPilot",
		...props
	},
	forwardedRef,
) {
	const [hostRef, setHostRef] = useForwardedRef(forwardedRef);

	return (
		<button
			ref={setHostRef}
			type={type}
			aria-label={ariaLabel}
			className={cn(
				"relative inline-flex size-[72px] shrink-0 items-center justify-center overflow-visible rounded-full border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-primary/40",
				className,
			)}
			{...props}
		>
			<BubbleOrbFilm hostRef={hostRef} state={orbState} ackNonce={ackNonce} />
			{children ?? <span className="sr-only">{ariaLabel}</span>}
		</button>
	);
});
