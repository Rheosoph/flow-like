"use client";

import { useTranslation } from "@flow-like/locales";
import { Ban, Upload } from "lucide-react";
import { cx } from "../../../settings/devices/primitives/tone";
import { type DragState, dropKind } from "./file-model";

/*
 * The empty state of a file field: the drop row (a button: ↵ or Space opens the dialog), and in its place the
 * disabled row of a FlowPath field this host cannot fill (spec F). Both are 36 px (44 px on a touch screen).
 */

/**
 * What a field says while files are dragged over it (spec M3): "Drop 10 files: one run each" (the files start one run
 * each), "Drop the files: one run each" when the browser gives no count, and over an attached file "Drop to replace the
 * file". Null keeps the plain invitation.
 */
export function useDropText(
	drag: DragState,
	oneRunEach: boolean,
	replace: boolean,
) {
	const { t } = useTranslation("interfaces");
	const kind = dropKind(drag, oneRunEach, replace);
	if (kind === "many")
		return t(
			"workbench.field.file.dragMany",
			"Drop {{n}} files: one run each",
			{
				n: drag.count,
			},
		);
	if (kind === "unknown")
		return t(
			"workbench.field.file.dragUnknown",
			"Drop the files: one run each",
		);
	return kind === "replace"
		? t("workbench.field.file.dragReplace", "Drop to replace the file")
		: null;
}

export interface DropRowProps {
	readonly label: string;
	readonly many: boolean;
	readonly touch: boolean;
	readonly invalid: boolean;
	readonly disabled: boolean;
	readonly drag: DragState;
	/** Dropping several files starts one run each (a one-file field on a host with next files). */
	readonly oneRunEach: boolean;
	readonly focus: Readonly<Record<string, string>>;
	readonly describedBy: string | undefined;
	readonly onOpen: () => void;
}

export function DropRow(props: Readonly<DropRowProps>) {
	const { t } = useTranslation("interfaces");
	const dragging = props.drag.over;
	const text = useDropText(props.drag, props.oneRunEach, false);
	const aria = props.many
		? t(
				"workbench.field.file.dropManyAria",
				"{{label}}. Enter or Space chooses files.",
				{
					label: props.label,
				},
			)
		: t(
				"workbench.field.file.dropAria",
				"{{label}}. Enter or Space chooses a file.",
				{
					label: props.label,
				},
			);
	return (
		<button
			type="button"
			disabled={props.disabled}
			aria-label={aria}
			aria-describedby={props.describedBy}
			{...props.focus}
			onClick={props.onOpen}
			className={cx(
				"flex w-full items-center gap-2 rounded-lg border border-dashed bg-card px-2.75 text-left text-ink-2 hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring disabled:opacity-60",
				props.touch ? "min-h-11 text-[15px]" : "h-9 text-[13px]",
				props.invalid ? "border-critical-line" : "border-border-strong",
				dragging && "border-foreground",
			)}
		>
			<Upload
				aria-hidden
				className="size-3.75 shrink-0 text-muted-foreground"
			/>
			{text !== null ? (
				<span>{text}</span>
			) : (
				<span>
					{props.many
						? t("workbench.field.file.dropManyLead", "Drop files or")
						: t("workbench.field.file.dropLead", "Drop a file or")}{" "}
					<span className="font-medium text-foreground underline underline-offset-[3px]">
						{t("workbench.field.file.browse", "browse")}
					</span>
				</span>
			)}
		</button>
	);
}

/** A FlowPath field where this host can make no FlowPath: not a button, not a Tab stop (spec F). */
export function BlockedRow({
	id,
	touch,
}: Readonly<{ id: string; touch: boolean }>) {
	const { t } = useTranslation("interfaces");
	return (
		<div
			id={id}
			className={cx(
				"flex items-center gap-2 rounded-lg border border-dashed border-border bg-surface-sunken px-2.75 text-[13px]/4.5 text-muted-foreground",
				touch ? "min-h-11" : "h-9",
			)}
		>
			<Ban aria-hidden className="size-3.25 shrink-0" />
			<span>
				{t(
					"workbench.field.file.blocked",
					"Files can't be sent from this page yet.",
				)}
			</span>
		</div>
	);
}
