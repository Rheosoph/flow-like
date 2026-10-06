"use client";

import { LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../../../lib/utils";
import type {
	FormSessionActions,
	FormSessionState,
	RunEntry,
} from "../contracts";
import { stopAria, stopLabel, stopTitle } from "./copy";
import { outputSignOf } from "./dock-view";
import { StopButton } from "./run-button";
import type { DockWords } from "./use-dock";

/** Stop for the run on the stage: it stops (or takes the run out of the queue) and hands the cursor to Run. */
export function DockStop({
	run,
	actions,
	kit,
	large,
	iconOnly,
	onStopped,
}: Readonly<{
	run: RunEntry;
	actions: FormSessionActions;
	kit: DockWords;
	large: boolean;
	iconOnly: boolean;
	onStopped: () => void;
}>) {
	const { t, copy } = kit;
	return (
		<StopButton
			label={stopLabel(t)}
			ariaLabel={stopAria(t, run.n)}
			title={stopTitle(t, run.n, copy.mac)}
			large={large}
			iconOnly={iconOnly}
			mac={copy.mac}
			onStop={() => {
				actions.stop(run.id);
				onStopped();
			}}
		/>
	);
}

function PaneButton({
	pressed,
	onClick,
	children,
}: Readonly<{ pressed: boolean; onClick: () => void; children: ReactNode }>) {
	return (
		<button
			type="button"
			aria-pressed={pressed}
			onClick={onClick}
			className={cn(
				"inline-flex min-w-19 items-center justify-center gap-1.5 rounded-[5px] px-3 font-medium text-[14px] outline-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring focus-visible:outline-solid",
				pressed ? "bg-foreground text-background" : "text-ink-2",
			)}
		>
			{children}
		</button>
	);
}

function OutputSign({
	sign,
	running,
	unseen,
}: Readonly<{
	sign: "spinner" | "dot" | null;
	running: string;
	unseen: string;
}>) {
	if (sign === "spinner")
		return (
			<LoaderCircle
				role="img"
				aria-label={running}
				className="size-3.25 shrink-0 animate-spin motion-reduce:animate-none"
				strokeWidth={2.5}
			/>
		);
	if (sign === "dot")
		return (
			<span
				role="img"
				aria-label={unseen}
				className="size-1.75 shrink-0 rounded-full bg-info-solid"
			/>
		);
	return null;
}

/**
 * The narrow layout's Inputs / Output segments: a spinner while work goes on, a dot for a result waiting on Inputs. Two
 * equal columns, so both halves take the wider label's width and the switch stays even while Output shows its sign.
 */
export function PaneSwitch({
	state,
	actions,
	kit,
}: Readonly<{
	state: FormSessionState;
	actions: FormSessionActions;
	kit: DockWords;
}>) {
	const { t } = kit;
	const { pane } = state.view;
	return (
		<fieldset
			aria-label={t("interfaces:workbench.dock.panes", "Which pane to show")}
			className="m-0 grid h-11.5 min-w-0 shrink-0 grid-cols-2 rounded-lg border border-border bg-background p-0"
		>
			<PaneButton
				pressed={pane === "inputs"}
				onClick={() => actions.setPane("inputs")}
			>
				{t("interfaces:workbench.dock.paneInputs", "Inputs")}
			</PaneButton>
			<PaneButton
				pressed={pane === "output"}
				onClick={() => actions.setPane("output")}
			>
				{t("interfaces:workbench.dock.paneOutput", "Output")}
				<OutputSign
					sign={outputSignOf(state)}
					running={t("interfaces:workbench.dock.paneRunning", "running")}
					unseen={t("interfaces:workbench.dock.paneUnseen", "new result")}
				/>
			</PaneButton>
		</fieldset>
	);
}
