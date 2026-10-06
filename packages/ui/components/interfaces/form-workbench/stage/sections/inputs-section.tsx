"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "../../../../settings/devices/primitives/tone";
import { sectionLabel } from "../copy";
import { guardRepeat } from "../key-guards";
import type { InputRowView, InputsView } from "../pane-model";
import { Section } from "../section-label";

function ValueCell({ row }: Readonly<{ row: InputRowView }>) {
	const { t } = useTranslation("interfaces");
	const { text } = row;
	if (text.kind === "value")
		return (
			<span className="min-w-0 whitespace-pre-line text-right text-[13px]/[18px] wrap-anywhere">
				{text.text}
			</span>
		);
	return (
		<span className="min-w-0 text-right text-[13px]/[18px] text-muted-foreground">
			{text.kind === "hidden"
				? t("workbench.stage.inputs.hidden", "Not saved on this device")
				: t("workbench.stage.inputs.empty", "Empty")}
		</span>
	);
}

function InputRow({ row }: Readonly<{ row: InputRowView }>) {
	return (
		<div className="relative flex min-h-8 break-inside-avoid items-baseline justify-between gap-4 border-hairline border-b pt-1.75 pb-1.5 pl-2.5">
			{row.differs ? (
				<span
					aria-hidden
					className="absolute top-1.75 bottom-1.75 left-0 w-0.5 rounded-xs bg-foreground"
				/>
			) : null}
			<span className="shrink-0 text-[13px]/[18px] text-muted-foreground">
				{row.label}
			</span>
			<ValueCell row={row} />
		</div>
	);
}

/**
 * What this run was given: label and value pairs, two columns where there is room. A 2 px bar marks an input
 * the rail differs in (only for the run the rail is compared with, never a per-run field).
 */
export function InputsSection({
	inputs,
	touch,
	onUse,
}: Readonly<{
	inputs: InputsView;
	touch: boolean;
	onUse: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	const notes = [
		inputs.presetName
			? t("workbench.stage.inputs.preset", "Preset {{name}}", {
					name: inputs.presetName,
				})
			: null,
		inputs.sameAsForm
			? t("workbench.stage.inputs.same", "Same as the form")
			: null,
	].filter((note): note is string => note !== null);
	return (
		<Section
			id="inputs"
			label={sectionLabel(t, "inputs")}
			gap="gap-1.5"
			rowHeight="min-h-7"
			aside={
				<>
					<span className="min-w-0 flex-1 text-muted-foreground text-xs/4">
						{notes.join(" · ")}
					</span>
					{inputs.canUse ? (
						<button
							type="button"
							onKeyDown={guardRepeat}
							onClick={onUse}
							className={cx(
								"inline-flex shrink-0 items-center whitespace-nowrap rounded-lg border border-border bg-card px-2.5 font-medium text-[12.5px] outline-ring hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1",
								touch ? "h-11" : "h-7",
							)}
						>
							{t("workbench.stage.action.use", "Use these inputs")}
						</button>
					) : null}
				</>
			}
		>
			<div className="columns-[2_300px] gap-x-8">
				{inputs.rows.map((row) => (
					<InputRow key={row.key} row={row} />
				))}
			</div>
		</Section>
	);
}
