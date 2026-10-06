"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { cx } from "../../../../settings/devices/primitives/tone";
import type { RunStatus, RunStep, RunStepState } from "../../contracts";
import { statusWord } from "../../status-look";
import { sectionLabel } from "../copy";
import { Section } from "../section-label";
import { StatusIcon } from "../status-icon";
import { TAIL_ATTR } from "../use-body-scroll";

/** A step's state as the status it looks like: the icon and tone are the run status's. */
const STEP_STATUS: Readonly<Record<RunStepState, RunStatus>> = {
	done: "done",
	active: "running",
	failed: "failed",
	stopped: "stopped",
	planned: "queued",
};

const STEP_WORD: Readonly<Partial<Record<RunStepState, RunStatus>>> = {
	failed: "failed",
	stopped: "stopped",
};

const LINK =
	"rounded-sm text-xs outline-ring hover:underline focus-visible:outline-2 focus-visible:outline-offset-1";

function StepRow({ step }: Readonly<{ step: RunStep }>) {
	const { t } = useTranslation("interfaces");
	const word = STEP_WORD[step.state];
	return (
		<li className="grid grid-cols-[16px_14px_minmax(0,1fr)] items-start gap-x-2">
			<StatusIcon status={STEP_STATUS[step.state]} className="mt-px size-4" />
			<span className="text-right font-mono text-muted-foreground text-xs/[18px] tabular-nums">
				{step.number}
			</span>
			<div className="flex min-w-0 flex-col gap-0.75">
				<div className="flex flex-wrap items-baseline gap-x-2 text-[13px]/[18px]">
					<span
						className={cx(
							step.state === "done" ? "font-medium" : "font-semibold",
							step.state === "planned" && "font-normal text-muted-foreground",
						)}
					>
						{step.title}
					</span>
					{step.detail ? (
						<span className="text-muted-foreground">{step.detail}</span>
					) : null}
					{word ? (
						<span
							className={cx(
								"font-medium",
								word === "failed" ? "text-critical" : "text-unknown",
							)}
						>
							{statusWord(t, word)}
						</span>
					) : null}
				</div>
				{step.message ? (
					<p className="m-0 max-w-xl text-pretty text-[13px]/[19px] text-ink-2">
						{step.message}
					</p>
				) : null}
			</div>
		</li>
	);
}

/**
 * The steps a run reached, numbered ("Step 4", there is no known total). A finished run folds them into
 * "6 steps finished · Show steps"; a failed or stopped one keeps them open.
 */
export function StepsSection({
	steps,
	finished,
	live,
	touch,
}: Readonly<{
	steps: readonly RunStep[];
	/** Done or empty: the list may fold away. */
	finished: boolean;
	/** The flow is still writing steps: the body follows the last one. */
	live: boolean;
	touch: boolean;
}>) {
	const { t } = useTranslation("interfaces");
	const [chosen, setChosen] = useState<boolean | null>(null);
	const open = chosen ?? !finished;
	const hide = finished && open;
	const target = touch && "min-h-11";
	return (
		<Section
			id="steps"
			label={sectionLabel(t, "steps")}
			gap="gap-2"
			aside={
				hide ? (
					<button
						type="button"
						aria-expanded
						onClick={() => setChosen(false)}
						className={cx(LINK, "ml-1 px-0.5 font-medium text-ink-2", target)}
					>
						{t("workbench.stage.steps.hide", "Hide steps")}
					</button>
				) : null
			}
		>
			<div>
				{open ? (
					<ol className="m-0 flex list-none flex-col gap-2.5 p-0">
						{steps.map((step) => (
							<StepRow key={step.id} step={step} />
						))}
					</ol>
				) : (
					<div className="flex min-h-6 items-center gap-2 text-[13px]/[18px] text-ink-2">
						<StatusIcon status="done" className="size-4" />
						<span>
							{t("workbench.stage.steps.finished", "{{count}} steps finished", {
								count: steps.length,
								defaultValue_one: "{{count}} step finished",
							})}
						</span>
						<button
							type="button"
							aria-expanded={false}
							onClick={() => setChosen(true)}
							className={cx(
								LINK,
								"ml-1 px-0.5 font-medium text-[12.5px] text-foreground underline underline-offset-[3px]",
								target,
							)}
						>
							{t("workbench.stage.steps.show", "Show steps")}
						</button>
					</div>
				)}
				{live ? (
					<div aria-hidden {...{ [TAIL_ATTR]: "" }} className="h-0" />
				) : null}
			</div>
		</Section>
	);
}
