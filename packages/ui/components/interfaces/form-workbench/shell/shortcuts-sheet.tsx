"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { X } from "lucide-react";
import { useId, useMemo } from "react";
import { DvButton } from "../../../settings/devices/primitives/dv-button";
import { Kbd } from "../../../settings/devices/primitives/kbd";
import { cx } from "../../../settings/devices/primitives/tone";
import type {
	FormSessionActions,
	FormSessionState,
	ShortcutActionId,
	ShortcutGroup,
	ShortcutGroupId,
	ShortcutRow,
} from "../contracts";
import { formatDate } from "../model/date-text";
import { dateAnchorOf } from "../model/dates";
import { targets } from "../model/fields";
import { shortcutGroups } from "../model/shortcuts";
import { InterfaceModal } from "./interface-modal";

type InterfacesT = TFunction<"interfaces">;
type RowText = (t: InterfacesT, row: ShortcutRow, locale: string) => string;

const text =
	(read: (t: InterfacesT) => string): RowText =>
	(t) =>
		read(t);

const dateText = (row: ShortcutRow, locale: string) =>
	row.example ? formatDate(row.example, locale) : "";

/** One literal `t()` per action, so the extractor keeps every key (PLAN §9). */
const ROW_TEXT: Readonly<Record<ShortcutActionId, RowText>> = {
	run: text((t) => t("interfaces:workbench.shell.shortcuts.run", "Run")),
	runLeave: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.runLeave",
			"Run and leave the inputs as they are",
		),
	),
	stop: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.stop",
			"Stop the run on screen, or take it out of the queue",
		),
	),
	enterNext: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.enterNext",
			"Next empty field, or run when none is left",
		),
	),
	tabNext: text((t) =>
		t("interfaces:workbench.shell.shortcuts.tabNext", "Next or previous field"),
	),
	filter: text((t) =>
		t("interfaces:workbench.shell.shortcuts.filter", "Filter fields"),
	),
	runTabs: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.runTabs",
			"Previous or next run, in the run strip",
		),
	),
	recent: text((t) =>
		t("interfaces:workbench.shell.shortcuts.recent", "Recent values"),
	),
	acceptSuggestion: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.acceptSuggestion",
			"Take the suggested value",
		),
	),
	resetField: text((t) =>
		t("interfaces:workbench.shell.shortcuts.resetField", "Reset this input"),
	),
	chooseFiles: text((t) =>
		t("interfaces:workbench.shell.shortcuts.chooseFiles", "Choose files"),
	),
	replaceFile: text((t) =>
		t("interfaces:workbench.shell.shortcuts.replaceFile", "Replace the file"),
	),
	removeFile: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.removeFile",
			"Remove the file; the next one moves in",
		),
	),
	undo: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.undo",
			"Undo the last removal or change",
		),
	),
	calendar: text((t) =>
		t("interfaces:workbench.shell.shortcuts.calendar", "Open the calendar"),
	),
	nudge: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.nudge",
			"Change a number by 1, or by 10",
		),
	),
	presets: text((t) =>
		t("interfaces:workbench.shell.shortcuts.presets", "Presets"),
	),
	savePreset: text((t) =>
		t(
			"interfaces:workbench.shell.shortcuts.savePreset",
			"Save the inputs as a preset",
		),
	),
	dateDay: (t, row, locale) =>
		t(
			"interfaces:workbench.shell.shortcuts.dateDay",
			"{{date}}, in the month of the last date",
			{ date: dateText(row, locale) },
		),
	dateDayMonth: (_t, row, locale) => dateText(row, locale),
	dateFull: (_t, row, locale) => dateText(row, locale),
	dateWords: text((t) =>
		t("interfaces:workbench.shell.shortcuts.dateWords", "Counted from today"),
	),
};

const GROUP_TITLE: Readonly<
	Record<ShortcutGroupId, (t: InterfacesT) => string>
> = {
	run: (t) => t("interfaces:workbench.shell.shortcuts.groupRun", "Run"),
	move: (t) =>
		t("interfaces:workbench.shell.shortcuts.groupMove", "Move around"),
	faster: (t) =>
		t("interfaces:workbench.shell.shortcuts.groupFaster", "Fill in faster"),
	dates: (t) =>
		t("interfaces:workbench.shell.shortcuts.groupDates", "Typing dates"),
};

function localDay(date: Date) {
	const month = String(date.getMonth() + 1).padStart(2, "0");
	const day = String(date.getDate()).padStart(2, "0");
	return `${date.getFullYear()}-${month}-${day}`;
}

/** The sheet's groups for this form: only what applies, in the viewer's date order against the first date field's anchor. */
export function sheetGroupsOf(
	state: FormSessionState,
	today: string,
): readonly ShortcutGroup[] {
	const { form } = state;
	const dateStop = targets(form.fields).find(
		(stop) => stop.field.kind === "date",
	);
	return shortcutGroups(form.fields, {
		anchorIso: dateStop ? dateAnchorOf(state, dateStop.key, today) : null,
		todayIso: today,
		mac: form.viewer.mac,
		built: { presets: true, numbers: true },
		dateLocale: form.viewer.dateLocale,
	});
}

function Keys({ keys }: Readonly<{ keys: readonly string[] }>) {
	return (
		<span className="flex shrink-0 items-center gap-1">
			{keys.map((key) => (
				<Kbd key={key} className={cx("h-5 px-1.5 text-xs", "text-ink-2")}>
					{key}
				</Kbd>
			))}
		</span>
	);
}

function GroupBlock({
	group,
	locale,
}: Readonly<{ group: ShortcutGroup; locale: string }>) {
	const { t } = useTranslation("interfaces");
	const labelId = useId();
	return (
		<section aria-labelledby={labelId} className="mb-5 break-inside-avoid">
			<h3
				id={labelId}
				className="mb-1 font-semibold text-label text-muted-foreground uppercase tracking-[0.06em]"
			>
				{GROUP_TITLE[group.id](t)}
			</h3>
			<ul className="m-0 list-none p-0">
				{group.rows.map((row) => (
					<li
						key={`${row.action}-${row.keys.join("+")}`}
						className={cx(
							"flex min-h-7 items-center justify-between gap-x-4 gap-y-1 py-0.5 text-[13px] leading-[18px]",
							row.keys.length >= 3 && "flex-wrap",
						)}
					>
						<span className="min-w-0">
							{ROW_TEXT[row.action](t, row, locale)}
						</span>
						<Keys keys={row.keys} />
					</li>
				))}
			</ul>
		</section>
	);
}

export interface ShortcutsSheetProps {
	readonly state: FormSessionState;
	readonly actions: Pick<FormSessionActions, "closeOverlay">;
}

/** S2: the keyboard shortcuts of this form. Opened by ⌘/ and ?, closed by Esc, ⌘/ and its close button. */
export function ShortcutsSheet({
	state,
	actions,
}: Readonly<ShortcutsSheetProps>) {
	const { t } = useTranslation("interfaces");
	const titleId = useId();
	const open = state.view.overlay?.id === "shortcuts";
	const groups = useMemo(
		() => (open ? sheetGroupsOf(state, localDay(new Date())) : []),
		[open, state],
	);
	const { viewer } = state.form;
	const close = () => actions.closeOverlay();
	return (
		<InterfaceModal
			open={open}
			onClose={close}
			labelledBy={titleId}
			width={640}
			placement="center"
		>
			<div className="flex items-start gap-4 p-6 pb-4">
				<div className="min-w-0 flex-1">
					<h2 id={titleId} className="font-semibold text-[15px] leading-5">
						{t("workbench.shell.shortcuts.title", "Keyboard shortcuts")}
					</h2>
					<p className="mt-1 text-muted-foreground text-xs leading-4">
						{t(
							"workbench.shell.shortcuts.hint",
							"They work while the cursor is in this form.",
						)}
					</p>
				</div>
				<DvButton
					variant="ghost"
					iconOnly
					icon={X}
					aria-label={t("workbench.shell.shortcuts.close", "Close")}
					onClick={close}
				/>
			</div>
			<div className="px-6 @min-[632px]/fw:columns-2 @min-[632px]/fw:gap-x-8">
				{groups.map((group) => (
					<GroupBlock key={group.id} group={group} locale={viewer.locale} />
				))}
			</div>
			<p className="px-6 pb-6 text-muted-foreground text-xs leading-4">
				{t(
					"workbench.shell.shortcuts.footer",
					"Open this list again with {{keys}}.",
					{ keys: viewer.mac ? "⌘/" : "Ctrl+/" },
				)}
			</p>
		</InterfaceModal>
	);
}
