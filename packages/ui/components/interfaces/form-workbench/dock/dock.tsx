"use client";

import { memo } from "react";
import type { DockProps } from "../contracts";
import { sameButStreamedOutput } from "../session/state";
import { PhoneDock } from "./phone-dock";
import { RailDock } from "./rail-dock";
import { HeroDock, StripDock } from "./run-dock";

const DOCKS = {
	rail: RailDock,
	phone: PhoneDock,
	hero: HeroDock,
	strip: StripDock,
} as const;

function DockView(props: Readonly<DockProps>) {
	const View = DOCKS[props.variant];
	return <View {...props} />;
}

/** Nothing the dock shows comes from what a run streamed (its line reads statuses, numbers and copies). */
const sameDock = (prev: Readonly<DockProps>, next: Readonly<DockProps>) =>
	prev.variant === next.variant &&
	prev.actions === next.actions &&
	prev.layout === next.layout &&
	prev.routes === next.routes &&
	sameButStreamedOutput(prev.state, next.state);

/**
 * Every Run control of the form and its single status line (PLAN §7): `rail` (desktop, bottom of the rail), `phone`
 * (narrow layout's bottom bar), `hero` (the zero-field card before the first run) and `strip` (the zero-field run
 * strip's left end). It renders again for what it shows, not for every chunk of a streamed answer.
 */
export const Dock = memo(DockView, sameDock);
