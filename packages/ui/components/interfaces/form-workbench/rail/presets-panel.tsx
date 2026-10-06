"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { type RefObject, useMemo, useRef, useState } from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import { FORM_LIMITS, type FormSessionState, type Preset } from "../contracts";
import { shortWordsOf } from "../model/date-text";
import { activePresetOf } from "../model/markers";
import {
	type PresetSummary,
	presetEdits,
	presetSummaryParts,
} from "../model/presets";
import { foldText } from "../model/secrets";
import { ActionRow, PresetRow } from "./preset-rows";
import { onPanelKey } from "./presets-keys";
import type { RailPartProps } from "./rail-model";

type InterfacesT = TFunction<"interfaces">;

const byDigit = (a: Preset, b: Preset) =>
	(a.digit || 99) - (b.digit || 99) || a.createdAt - b.createdAt;

function summaryText(t: InterfacesT, summary: PresetSummary) {
	const parts = summary.parts.map((part) => `${part.label} ${part.text}`);
	if (summary.more > 0)
		parts.push(
			t("interfaces:workbench.preset.menu.more", "+ {{count}} more", {
				count: summary.more,
			}),
		);
	return parts.join(" · ");
}

function FindBox({
	value,
	onChange,
	touch,
	inputRef,
}: Readonly<{
	value: string;
	onChange: (value: string) => void;
	touch: boolean;
	inputRef: RefObject<HTMLInputElement | null>;
}>) {
	const { t } = useTranslation("interfaces");
	const label = t("workbench.preset.menu.find", "Find a preset");
	return (
		<input
			ref={inputRef}
			type="search"
			data-preset-find=""
			value={value}
			onChange={(event) => onChange(event.target.value)}
			placeholder={label}
			aria-label={label}
			autoComplete="off"
			className={cx(
				"mb-1 block w-full rounded-md border border-input bg-card px-2.5 text-[13px] text-foreground placeholder:text-muted-foreground hover:border-border-strong focus-visible:outline-2 focus-visible:outline-ring [&::-webkit-search-cancel-button]:appearance-none",
				touch ? "h-11 text-base" : "h-8",
			)}
		/>
	);
}

interface ListProps {
	readonly shown: readonly Preset[];
	readonly state: FormSessionState;
	readonly activeId: string | null;
	readonly touch: boolean;
	readonly onApply: (id: string) => void;
	readonly onDelete: (id: string) => void;
}

function PresetList(props: Readonly<ListProps>) {
	const { shown, state, activeId, touch, onApply, onDelete } = props;
	const { t } = useTranslation("interfaces");
	const locale = state.form.viewer.locale;
	const words = useMemo(() => shortWordsOf(t, locale), [t, locale]);
	return (
		<>
			<ul className="m-0 min-h-0 flex-1 list-none overflow-y-auto p-0 [scrollbar-width:thin]">
				{shown.map((preset) => (
					<PresetRow
						key={preset.id}
						preset={preset}
						summary={summaryText(
							t,
							presetSummaryParts(preset, state.form.fields, words),
						)}
						active={preset.id === activeId}
						touch={touch}
						onApply={() => onApply(preset.id)}
						onDelete={() => onDelete(preset.id)}
					/>
				))}
			</ul>
			{shown.length === 0 ? (
				<p className="m-0 px-2 py-2 text-xs/4 text-muted-foreground">
					{t("workbench.preset.menu.noMatch", "No preset matches.")}
				</p>
			) : null}
		</>
	);
}

interface ActionsProps extends RailPartProps {
	readonly active: Preset | null;
	readonly edited: number;
	readonly release: () => void;
	readonly finish: () => void;
}

/** Update {name} and Reset to {name} only while the preset is edited; Save as new preset always. */
function PanelActions(props: Readonly<ActionsProps>) {
	const { state, actions, layout, active, edited, release, finish } = props;
	const { t } = useTranslation("interfaces");
	const touch = layout.touch;
	const key = state.form.viewer.mac ? "⌘S" : "Ctrl+S";
	const edit = active !== null && edited > 0 ? active : null;
	const noPresets = state.memory.presets.length === 0;
	const saveNew = () => {
		release();
		actions.openOverlay({ id: "presetSave", mode: "save", fromRunId: null });
	};
	const update = () => {
		if (edit) actions.updatePreset(edit.id);
		finish();
	};
	const reset = () => {
		actions.resetToPreset();
		finish();
	};
	return (
		<>
			{edit ? (
				<ActionRow
					touch={touch}
					label={t("workbench.preset.menu.update", "Update {{name}}", {
						name: edit.name,
					})}
					hint={
						touch
							? undefined
							: t("workbench.preset.menu.changes", "{{count}} changes", {
									count: edited,
									defaultValue_one: "{{count}} change",
								})
					}
					onSelect={update}
				/>
			) : null}
			<ActionRow
				touch={touch}
				label={
					noPresets
						? t(
								"workbench.preset.menu.saveFirst",
								"Save these inputs as a preset…",
							)
						: t("workbench.preset.menu.saveNew", "Save as new preset…")
				}
				hint={touch ? undefined : key}
				onSelect={saveNew}
			/>
			{edit ? (
				<ActionRow
					touch={touch}
					label={t("workbench.preset.menu.resetTo", "Reset to {{name}}", {
						name: edit.name,
					})}
					onSelect={reset}
				/>
			) : null}
		</>
	);
}

export interface PanelProps extends RailPartProps {
	/** The next thing moves focus itself: the menu must not hand it back to the button when it closes. */
	readonly release: () => void;
}

/** S1: the preset rows and the actions under them; shared by the popover and the phone's sheet. */
export function PresetsPanel(props: Readonly<PanelProps>) {
	const { state, actions, layout, release } = props;
	const { t } = useTranslation("interfaces");
	const finish = () => {
		release();
		actions.closeOverlay();
	};
	const [query, setQuery] = useState("");
	const findRef = useRef<HTMLInputElement | null>(null);
	const presets = useMemo(
		() => [...state.memory.presets].sort(byDigit),
		[state.memory.presets],
	);
	const active = activePresetOf(state);
	const edited = presetEdits(state.form.fields, state.rail.values, active);
	const needle = foldText(query).trim();
	const shown = needle
		? presets.filter((item) => foldText(item.name).includes(needle))
		: presets;
	const apply = (id: string) => {
		actions.applyPreset(id);
		finish();
	};
	const remove = (id: string) => {
		actions.deletePreset(id);
		finish();
	};
	const keys = {
		presets,
		digits: !layout.touch,
		apply,
		remove,
		find: () => findRef.current,
	};
	return (
		<div
			onKeyDown={(event) => onPanelKey(event, keys)}
			className="flex min-h-0 flex-1 flex-col"
		>
			{presets.length === 0 ? (
				<p className="m-0 px-2 py-2 text-xs/4 text-muted-foreground">
					{t(
						"workbench.preset.menu.empty",
						"Save inputs you use often, then fill the form in one step.",
					)}
				</p>
			) : (
				<>
					{presets.length >= FORM_LIMITS.presetFindFrom ? (
						<FindBox
							value={query}
							onChange={setQuery}
							touch={layout.touch}
							inputRef={findRef}
						/>
					) : null}
					<PresetList
						shown={shown}
						state={state}
						activeId={active?.id ?? null}
						touch={layout.touch}
						onApply={apply}
						onDelete={remove}
					/>
					<hr className="mx-0 my-1 border-0 border-t border-hairline" />
				</>
			)}
			<PanelActions
				state={state}
				actions={actions}
				layout={layout}
				active={active}
				edited={edited}
				release={release}
				finish={finish}
			/>
		</div>
	);
}
