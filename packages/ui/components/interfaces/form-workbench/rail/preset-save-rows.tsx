"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useId } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { Checkbox } from "../../../ui/checkbox";
import { type SaveRow, isTickable } from "./preset-save-model";

/** Checked is neutral (foreground fill), never coral (house rule, devices CheckField). */
export const NEUTRAL_CHECK =
	"mt-0 border-border-strong shadow-none data-[state=checked]:border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background dark:data-[state=checked]:bg-foreground focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring focus-visible:outline-solid";

export const FIELD_LABEL = "text-[13px]/[18px] font-medium";

function Tag({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<span className="flex-none rounded-sm border border-border bg-surface-sunken px-1.5 text-xs/4 text-muted-foreground">
			{children}
		</span>
	);
}

function RowTags({ row }: Readonly<{ row: SaveRow }>) {
	const { t } = useTranslation("interfaces");
	return (
		<>
			{row.perRun ? (
				<Tag>{t("workbench.preset.save.perRun", "Per run")}</Tag>
			) : null}
			{row.kind === "files" ? (
				<Tag>{t("workbench.preset.save.files", "Files are not saved")}</Tag>
			) : null}
			{row.kind === "secret" ? (
				<Tag>
					{t("workbench.preset.save.secret", "Not saved on this device")}
				</Tag>
			) : null}
		</>
	);
}

export interface RowViewProps {
	readonly row: SaveRow;
	readonly checked: boolean;
	readonly valueText: string;
	readonly touch: boolean;
	readonly onToggle: () => void;
}

/** One input the preset could set: a check, its label and tags, and what it holds now. */
export function RowView(props: Readonly<RowViewProps>) {
	const { row, checked, valueText, touch, onToggle } = props;
	const id = useId();
	const disabled = !isTickable(row);
	return (
		<li>
			<label
				htmlFor={id}
				className={cx(
					"flex items-start gap-2.5 rounded-md px-1",
					touch ? "min-h-11 py-3.25" : "min-h-9 py-2.25",
					disabled ? "cursor-default" : "cursor-pointer hover:bg-row-hover",
				)}
			>
				<Checkbox
					id={id}
					checked={checked}
					disabled={disabled}
					onCheckedChange={onToggle}
					className={cx(NEUTRAL_CHECK, "mt-px")}
				/>
				<span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
					<span
						className={cx(FIELD_LABEL, disabled && "text-muted-foreground")}
					>
						{row.field.label}
					</span>
					<RowTags row={row} />
				</span>
				<span
					className={cx(
						"max-w-[45%] flex-none truncate text-[13px]/[18px] text-ink-2",
						row.field.kind === "number" && "font-mono tabular-nums",
					)}
				>
					{valueText}
				</span>
			</label>
		</li>
	);
}
