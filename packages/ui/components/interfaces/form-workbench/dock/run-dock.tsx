"use client";

import type { DockProps } from "../contracts";
import { RunButton } from "./run-button";
import { useDockWords, useRunControl } from "./use-dock";

/**
 * Run inside the zero-field card before the first run (spec §7): one full-width coral button, nothing else. The shell
 * puts the name and description above it.
 */
export function HeroDock({ state, actions, layout }: Readonly<DockProps>) {
	const kit = useDockWords(state);
	const run = useRunControl(state, actions, "hero", kit);
	return (
		<RunButton
			variant="hero"
			label={run.label}
			title={run.title}
			capped={run.capped}
			large={layout.touch}
			mac={kit.copy.mac}
			onRun={run.onRun}
		/>
	);
}

/**
 * The coral Run at the left end of a zero-field form's run strip: "Run" until a run has ended, then "Run again". At the
 * cap it is the neutral, aria-disabled "Wait for a run to end" (spec M5: a form without fields never queues).
 */
export function StripDock({ state, actions, layout }: Readonly<DockProps>) {
	const kit = useDockWords(state);
	const run = useRunControl(state, actions, "strip", kit);
	return (
		<div data-fw-dock="strip" className="flex items-center pr-3">
			<RunButton
				variant="strip"
				label={run.label}
				title={run.title}
				capped={run.capped}
				large={layout.touch}
				mac={kit.copy.mac}
				onRun={run.onRun}
			/>
		</div>
	);
}
