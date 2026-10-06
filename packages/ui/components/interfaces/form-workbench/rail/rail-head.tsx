"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { RailTab } from "../contracts";
import { PresetsMenu } from "./presets-menu";
import type { RailPartProps, RailShape } from "./rail-model";

export interface RailHeadProps extends RailPartProps {
	readonly shape: RailShape;
	/** Some field differs from its starting value: "Reset to defaults" shows. */
	readonly differs: boolean;
}

interface TabsProps {
	readonly tab: RailTab;
	readonly count: number;
	readonly onChange: (tab: RailTab) => void;
}

/** Inputs | Runs: pressed is neutral (foreground fill), never coral. */
function RailTabs({ tab, count, onChange }: Readonly<TabsProps>) {
	const { t } = useTranslation("interfaces");
	const options: readonly { value: RailTab; label: string; count?: number }[] =
		[
			{ value: "inputs", label: t("workbench.rail.tabs.inputs", "Inputs") },
			{ value: "runs", label: t("workbench.rail.tabs.runs", "Runs"), count },
		];
	return (
		<fieldset
			aria-label={t("workbench.rail.tabs.label", "What the rail shows")}
			className="m-0 inline-flex max-w-full min-w-0 gap-0.5 rounded-md border border-border bg-card p-0.5"
		>
			{options.map((option) => {
				const pressed = option.value === tab;
				return (
					<button
						key={option.value}
						type="button"
						aria-pressed={pressed}
						onClick={() => onChange(option.value)}
						className={cx(
							"inline-flex h-6.5 items-center justify-center gap-1.5 rounded-sm px-3.5 text-ui font-medium whitespace-nowrap focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring",
							pressed
								? "bg-foreground text-background"
								: "text-ink-2 hover:bg-row-hover hover:text-foreground",
						)}
					>
						{option.label}
						{option.count !== undefined ? (
							<>
								<span className="sr-only"> </span>
								<span className="font-mono text-xs tabular-nums opacity-75">
									{option.count}
								</span>
							</>
						) : null}
					</button>
				);
			})}
		</fieldset>
	);
}

function ResetAll({
	touch,
	onReset,
}: Readonly<{ touch: boolean; onReset: () => void }>) {
	const { t } = useTranslation("interfaces");
	return (
		<button
			type="button"
			onClick={onReset}
			className={cx(
				"relative flex-none whitespace-nowrap bg-transparent px-0.5 font-medium text-ink-2 hover:underline focus-visible:outline-2 focus-visible:outline-ring",
				touch
					? "h-6.5 text-sm after:absolute after:-inset-x-1.5 after:top-1/2 after:h-11 after:-translate-y-1/2 after:content-['']"
					: "h-5 text-xs",
			)}
		>
			{t("workbench.rail.resetAll", "Reset to defaults")}
		</button>
	);
}

/**
 * "Reset to defaults" when the Presets button sits beside the name (a box too narrow for the split): under
 * the description, back on the name's line once the box has room for all three (spec S1 Phone: the button
 * right of the name, the description directly under it).
 */
const RESET_BELOW_DESCRIPTION =
	"order-5 ml-auto flex flex-none @min-[560px]/fw:order-2 @min-[560px]/fw:ml-0";

/** The name's line beside the name: Reset (A's place) and, without the split, the Presets anchor or button. */
function NameLineEnd({
	reset,
	presets,
	besideName,
}: Readonly<{
	reset: ReactNode;
	presets: ReactNode;
	besideName: boolean;
}>) {
	if (!besideName)
		return (
			<div className="order-2 ml-auto flex flex-none items-center gap-1">
				{reset}
				{presets}
			</div>
		);
	return (
		<>
			{reset ? <span className={RESET_BELOW_DESCRIPTION}>{reset}</span> : null}
			<span className="order-3 ml-auto flex flex-none items-center">
				{presets}
			</span>
		</>
	);
}

/**
 * The rail's head: the name, "Reset to defaults" at its right while anything differs, the description
 * (the stage shows it before the first run), and on a wide box the Inputs | Runs switch with the Presets
 * button at the right end of its row. On a narrow box the Presets button sits beside the name.
 */
export function RailHead({
	state,
	actions,
	layout,
	shape,
	differs,
}: Readonly<RailHeadProps>) {
	const { split, touch } = layout;
	const presets = (
		<PresetsMenu
			state={state}
			actions={actions}
			layout={layout}
			visible={shape.presets}
		/>
	);
	const inRow = split && (shape.tabs || shape.presets);
	const besideName = !inRow && shape.presets;
	const description = state.form.description;
	return (
		<header
			className={cx(
				"flex-none",
				split
					? cx("px-5 pt-4", inRow ? "pb-3" : "pb-2.5")
					: "mx-auto w-full max-w-2xl px-4 pt-3.5 pb-2",
			)}
		>
			<div
				data-fw-head-line=""
				className={cx(
					"flex flex-wrap gap-x-2 gap-y-1",
					besideName ? "items-center" : "items-start",
				)}
			>
				<h2
					className={cx(
						"order-1 m-0 min-w-0 flex-1 basis-32 font-semibold tracking-[-0.01em]",
						split ? "text-[15px]/5" : "text-xl/[26px]",
					)}
				>
					{state.form.name}
				</h2>
				<NameLineEnd
					besideName={besideName}
					presets={inRow ? null : presets}
					reset={
						differs && shape.tab === "inputs" ? (
							<ResetAll touch={touch} onReset={() => actions.resetAll()} />
						) : null
					}
				/>
				{shape.description !== "hidden" ? (
					<p
						title={description}
						className={cx(
							"order-4 m-0 basis-full text-muted-foreground",
							split ? "text-[13px]/[18px]" : "text-sm/5",
							shape.description === "clamped" && "line-clamp-2",
						)}
					>
						{description}
					</p>
				) : null}
			</div>
			{inRow ? (
				<div className="mt-3 flex items-center justify-between gap-2">
					{shape.tabs ? (
						<RailTabs
							tab={shape.tab}
							count={state.runs.length}
							onChange={(tab) => actions.setRailTab(tab)}
						/>
					) : (
						<span />
					)}
					{presets}
				</div>
			) : null}
		</header>
	);
}
