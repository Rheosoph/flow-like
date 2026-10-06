"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { ChevronRight, OctagonX, TriangleAlert } from "lucide-react";
import { type ReactNode, useId, useMemo } from "react";
import { DvButton } from "../../../settings/devices/primitives/dv-button";
import { cx } from "../../../settings/devices/primitives/tone";
import { Checkbox } from "../../../ui/checkbox";
import { FORM_LIMITS, type Preset } from "../contracts";
import { shortWordsOf } from "../model/date-text";
import { shortText } from "../model/values";
import type { SaveBlock, SaveRow, SaveStart } from "./preset-save-model";
import { FIELD_LABEL, NEUTRAL_CHECK, RowView } from "./preset-save-rows";
import type { RailPartProps } from "./rail-model";
import { type SaveModel, useSaveModel } from "./use-save-model";

type InterfacesT = TFunction<"interfaces">;

function Message({
	tone,
	children,
}: Readonly<{ tone: "critical" | "warning"; children: ReactNode }>) {
	const Icon = tone === "critical" ? OctagonX : TriangleAlert;
	return (
		<p
			className={cx(
				"m-0 flex items-start gap-1 text-xs/4",
				tone === "critical" ? "text-critical" : "text-warning",
			)}
		>
			<Icon aria-hidden className="mt-px size-3.25 flex-none" />
			<span>{children}</span>
		</p>
	);
}

const BLOCK_TEXT: Readonly<Record<SaveBlock, (t: InterfacesT) => string>> = {
	name: (t) => t("interfaces:workbench.preset.save.nameEmpty", "Enter a name."),
	nothing: (t) =>
		t("interfaces:workbench.preset.save.nothing", "Tick at least one input."),
	limit: (t) =>
		t(
			"interfaces:workbench.preset.save.limit",
			"A form keeps up to {{max}} presets. Delete one to save another.",
			{ max: FORM_LIMITS.presetsPerForm },
		),
};

interface NameFieldProps {
	readonly model: SaveModel;
	readonly touch: boolean;
}

/** The name box with its messages: an empty name, a name that replaces a preset. */
function NameField({ model, touch }: Readonly<NameFieldProps>) {
	const { t } = useTranslation("interfaces");
	const nameId = useId();
	const messageId = useId();
	const { form, block, showBlock, clash } = model;
	const invalid = showBlock && block === "name";
	return (
		<div className="flex flex-col gap-1.5">
			<label htmlFor={nameId} className={FIELD_LABEL}>
				{t("workbench.preset.save.name", "Name")}
			</label>
			<input
				ref={model.nameRef}
				id={nameId}
				data-autofocus=""
				value={form.name}
				maxLength={FORM_LIMITS.presetNameChars}
				autoComplete="off"
				aria-invalid={invalid || undefined}
				aria-describedby={showBlock || clash ? messageId : undefined}
				onChange={(event) => model.setName(event.target.value)}
				className={cx(
					"block w-full rounded-md border bg-card px-3 text-foreground hover:border-border-strong focus-visible:outline-2 focus-visible:outline-ring",
					touch ? "h-11 text-base" : "h-9 text-[13.5px]",
					invalid ? "border-critical-line" : "border-input",
				)}
			/>
			<div id={messageId} className="flex flex-col gap-1">
				{showBlock && block ? (
					<Message tone="critical">{BLOCK_TEXT[block](t)}</Message>
				) : null}
				{clash ? (
					<Message tone="warning">
						{t(
							"workbench.preset.save.clash",
							"A preset called {{name}} exists. Saving replaces it.",
							{ name: clash.name },
						)}
					</Message>
				) : null}
			</div>
		</div>
	);
}

function OnOpenRow({
	model,
	touch,
}: Readonly<{ model: SaveModel; touch: boolean }>) {
	const { t } = useTranslation("interfaces");
	const id = useId();
	return (
		<label
			htmlFor={id}
			className={cx(
				"flex cursor-pointer items-center gap-2.5 rounded-md px-1",
				touch ? "min-h-11" : "min-h-9",
			)}
		>
			<Checkbox
				id={id}
				checked={model.form.openDefault}
				onCheckedChange={(next) => model.setOpenDefault(next === true)}
				className={NEUTRAL_CHECK}
			/>
			<span className="text-[13px]/[18px]">
				{t("workbench.preset.save.onOpen", "Apply when this form opens")}
			</span>
		</label>
	);
}

interface SectionProps {
	readonly model: SaveModel;
	readonly state: RailPartProps["state"];
	readonly touch: boolean;
}

/** INPUTS IT SETS: the inputs that differ, and the rest behind "At their defaults". */
function InputsSection({ model, state, touch }: Readonly<SectionProps>) {
	const { t } = useTranslation("interfaces");
	const defaultsId = useId();
	const { locale } = state.form.viewer;
	const words = useMemo(() => shortWordsOf(t, locale), [t, locale]);
	const { form, rows } = model;
	const draw = (row: SaveRow) => (
		<RowView
			key={row.field.name}
			row={row}
			checked={form.ticked.has(row.field.name)}
			valueText={shortText(row.field, row.value, words)}
			touch={touch}
			onToggle={() => model.toggle(row.field.name)}
		/>
	);
	const atDefaults = rows.filter((row) => row.atDefault);
	return (
		<section className="flex flex-col gap-1">
			<div className="flex items-baseline justify-between gap-3">
				<h3 className="m-0 text-label/4 font-semibold tracking-[0.06em] text-muted-foreground uppercase">
					{t("workbench.preset.save.inputs", "Inputs it sets")}
				</h3>
				<span className="font-mono text-xs/4 text-muted-foreground tabular-nums">
					{t("workbench.preset.save.count", "{{ticked}} of {{total}}", {
						ticked: form.ticked.size,
						total: rows.length,
					})}
				</span>
			</div>
			<ul className="m-0 flex list-none flex-col p-0">
				{rows.filter((row) => !row.atDefault).map(draw)}
			</ul>
			{atDefaults.length > 0 ? (
				<>
					<button
						type="button"
						aria-expanded={form.showDefaults}
						aria-controls={defaultsId}
						onClick={model.toggleDefaults}
						className={cx(
							"flex w-fit items-center gap-1 rounded-md px-1 text-[12.5px] font-medium text-ink-2 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring",
							touch ? "min-h-11" : "min-h-8",
						)}
					>
						<ChevronRight
							aria-hidden
							className={cx("size-3.5", form.showDefaults && "rotate-90")}
						/>
						{t(
							"workbench.preset.save.defaults",
							"At their defaults ({{count}})",
							{
								count: atDefaults.length,
							},
						)}
					</button>
					{form.showDefaults ? (
						<ul id={defaultsId} className="m-0 flex list-none flex-col p-0">
							{atDefaults.map(draw)}
						</ul>
					) : null}
				</>
			) : null}
		</section>
	);
}

function submitLabel(t: InterfacesT, start: SaveStart, clash: Preset | null) {
	if (clash)
		return t("interfaces:workbench.preset.save.replace", "Replace {{name}}", {
			name: clash.name,
		});
	return start.updating
		? t("interfaces:workbench.preset.save.submitUpdate", "Update preset")
		: t("interfaces:workbench.preset.save.submit", "Save preset");
}

export interface PresetSaveFormProps extends RailPartProps {
	readonly start: SaveStart;
	readonly fromRunId: string | null;
	/** The proposed name, worded ("Preset 3"). */
	readonly initialName: string;
}

/** S1: the Save dialog's body and footer (the frame around it is the modal or the sheet). */
export function PresetSaveForm(props: Readonly<PresetSaveFormProps>) {
	const { state, actions, layout, start } = props;
	const { t } = useTranslation("interfaces");
	const model = useSaveModel(props);
	const touch = layout.touch;
	const buttonSize = touch ? "h-11" : "h-9";
	return (
		<form
			noValidate
			onSubmit={(event) => {
				event.preventDefault();
				model.submit();
			}}
			className="flex min-h-0 flex-1 flex-col"
		>
			<div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 py-4 [scrollbar-width:thin]">
				<NameField model={model} touch={touch} />
				<OnOpenRow model={model} touch={touch} />
				<InputsSection model={model} state={state} touch={touch} />
				<p className="m-0 text-xs/4 text-muted-foreground">
					{t(
						"workbench.preset.save.hint",
						"Unticked inputs are left at their defaults. Files are never saved.",
					)}
				</p>
			</div>
			<footer className="flex flex-none justify-end gap-2 border-t border-hairline px-6 pt-3 pb-5">
				<DvButton className={buttonSize} onClick={() => actions.closeOverlay()}>
					{t("workbench.preset.save.cancel", "Cancel")}
				</DvButton>
				<DvButton variant="primary" type="submit" className={buttonSize}>
					{submitLabel(t, start, model.clash)}
				</DvButton>
			</footer>
		</form>
	);
}
