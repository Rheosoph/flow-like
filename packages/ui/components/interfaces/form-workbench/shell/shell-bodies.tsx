"use client";

import { useTranslation } from "@flow-like/locales";
import { type ComponentType, useEffect, useState } from "react";
import type {
	DockProps,
	RailProps,
	StageProps,
	WorkbenchViewProps,
} from "../contracts";
import { ZeroFieldHero } from "./zero-field-hero";

/** The three views the shell composes; tests hand in small stand-ins. */
export interface ShellParts {
	readonly Rail: ComponentType<RailProps>;
	readonly Dock: ComponentType<DockProps>;
	readonly Stage: ComponentType<StageProps>;
}

export interface BodyProps {
	readonly parts: ShellParts;
	readonly view: WorkbenchViewProps;
}

/** How long a form without fields waits for the device memory before it shows its button anyway. */
export const HERO_WAIT_MS = 700;

/** A form without fields waits for its history before it shows the first-run card, so repeat users never see it flash. */
function useHistoryWaited(loaded: boolean) {
	const [timedOut, setTimedOut] = useState(false);
	useEffect(() => {
		if (loaded) return;
		const timer = setTimeout(() => setTimedOut(true), HERO_WAIT_MS);
		return () => clearTimeout(timer);
	}, [loaded]);
	return loaded || timedOut;
}

/**
 * What the output side shows: the stage once the form has runs or fields, else the first-run card (a
 * card with the Run button in a split box, bare text above the phone dock otherwise).
 */
function StageSlot({
	parts,
	view,
	carded,
}: Readonly<BodyProps & { carded: boolean }>) {
	const { Dock, Stage } = parts;
	const { state } = view;
	const noFields = state.form.fields.length === 0;
	const waited = useHistoryWaited(!noFields || state.memory.loaded);
	if (!noFields || state.runs.length > 0) return <Stage {...view} />;
	if (!waited) return null;
	return (
		<ZeroFieldHero
			name={state.form.name}
			description={state.form.description}
			carded={carded}
		>
			{carded ? <Dock {...view} variant="hero" /> : null}
		</ZeroFieldHero>
	);
}

/**
 * The 400 px rail: the rail sizes to its fields, so on a short form the dock follows the last field and
 * sticks to the bottom only once the list scrolls (the Runs list fills the column). `group/rail` lets the
 * dock take its card fill from what the rail shows.
 */
function RailColumn({
	parts,
	view,
	label,
}: Readonly<BodyProps & { label: string }>) {
	const { Rail, Dock } = parts;
	return (
		<aside
			aria-label={label}
			data-fw-region="rail"
			className="group/rail flex min-h-0 w-100 shrink-0 flex-col border-hairline border-r bg-surface-sunken"
		>
			<Rail {...view} />
			<div data-fw-region="dock" className="shrink-0">
				<Dock {...view} variant="rail" />
			</div>
		</aside>
	);
}

/** Box of 900 px or more: the 400 px rail with its dock beside the stage; no rail for a form without fields. */
export function SplitBody({ parts, view }: Readonly<BodyProps>) {
	const { t } = useTranslation("interfaces");
	const inputsLabel = t("workbench.shell.regionInputs", "Inputs");
	const outputLabel = t("workbench.shell.regionOutput", "Output");
	const hasRail = view.state.form.fields.length > 0;
	return (
		<div className="flex min-h-0 flex-1">
			{hasRail ? (
				<RailColumn parts={parts} view={view} label={inputsLabel} />
			) : null}
			<section
				aria-label={outputLabel}
				data-fw-region="stage"
				className="flex min-h-0 min-w-0 flex-1 flex-col bg-background"
			>
				<StageSlot parts={parts} view={view} carded />
			</section>
		</div>
	);
}

/** Narrower box: one pane at a time above the phone dock. */
export function SingleBody({ parts, view }: Readonly<BodyProps>) {
	const { Rail, Dock } = parts;
	const { state } = view;
	const showInputs =
		state.form.fields.length > 0 && state.view.pane === "inputs";
	return (
		<div className="flex min-h-0 flex-1 flex-col">
			<div
				data-fw-region={showInputs ? "rail" : "stage"}
				className="flex min-h-0 flex-1 flex-col bg-background"
			>
				{showInputs ? (
					<Rail {...view} />
				) : (
					<StageSlot parts={parts} view={view} carded={false} />
				)}
			</div>
			<div data-fw-region="dock" className="shrink-0">
				<Dock {...view} variant="phone" />
			</div>
		</div>
	);
}
