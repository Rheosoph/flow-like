"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRight, TriangleAlert } from "lucide-react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { FieldChange } from "../contracts";
import type { ChipsView } from "./pane-model";

function ChangeChip({
	change,
	wrap,
}: Readonly<{ change: FieldChange; wrap: boolean }>) {
	const { t } = useTranslation("interfaces");
	const text = cx(
		"min-w-0 text-muted-foreground",
		wrap ? "wrap-anywhere" : "truncate",
	);
	return (
		<span
			title={`${change.label} ${change.from} → ${change.to}`}
			className={cx(
				"inline-flex min-h-6 max-w-full items-center gap-x-1.25 gap-y-0.5 rounded-lg border border-border bg-card px-2 py-0.5 text-xs/4",
				wrap && "flex-wrap",
			)}
		>
			<span className="shrink-0 font-medium">{change.label}</span>
			<span className={text}>{change.from}</span>
			<ArrowRight
				role="img"
				aria-label={t("workbench.stage.chips.changedTo", "changed to")}
				className="size-2.75 shrink-0 text-muted-foreground"
			/>
			<span className={cx("min-w-0", wrap ? "wrap-anywhere" : "truncate")}>
				{change.to}
			</span>
		</span>
	);
}

/** The bar's second line: what changed against the run before, and the warning that the rail was edited since this run. */
export function ChangeChips({
	chips,
	touch,
	onGoInputs,
}: Readonly<{
	chips: ChipsView;
	touch: boolean;
	onGoInputs: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	return (
		<div className="mt-2 flex flex-wrap items-center gap-1.5">
			{chips.sinceRun !== null ? (
				<span className="mr-0.5 whitespace-nowrap text-muted-foreground text-xs/4">
					{t("workbench.stage.chips.since", "Changed since run {{n}}", {
						n: chips.sinceRun,
					})}
				</span>
			) : null}
			{chips.changes.map((change) => (
				<ChangeChip key={change.name} change={change} wrap={touch} />
			))}
			{chips.edited ? (
				<button
					type="button"
					onClick={onGoInputs}
					className={cx(
						"inline-flex items-center gap-1.25 whitespace-nowrap rounded-lg border border-warning-line bg-warning-bg px-2 py-0.5 font-medium text-warning text-xs/4 outline-ring focus-visible:outline-2 focus-visible:outline-offset-1",
						touch ? "min-h-11" : "min-h-6",
					)}
				>
					<TriangleAlert aria-hidden className="size-3.25 shrink-0" />
					<span>
						{t("workbench.stage.chips.edited", "Inputs edited since this run")}
					</span>
				</button>
			) : null}
		</div>
	);
}
