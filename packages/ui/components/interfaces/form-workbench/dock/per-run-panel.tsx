"use client";

import { useTranslation } from "@flow-like/locales";
import { Check } from "lucide-react";
import {
	type ComponentType,
	type KeyboardEvent,
	type ReactNode,
	useId,
	useRef,
	useState,
} from "react";
import { cn } from "../../../../lib/utils";
import type { AfterRunRow, FormSessionActions, ShortWords } from "../contracts";
import { isPlain } from "../fields/keys";
import { backText } from "./copy";
import { matchingRows } from "./dock-view";

const FOCUS_RING =
	"outline-none focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring focus-visible:outline-solid";

/** The panel's title and lead: a plain element in the popover, the dialog's title and description in the sheet. */
export type TextSlot = ComponentType<{
	readonly className: string;
	readonly children: ReactNode;
}>;

/** The checkbox of a row (by field name), for the shells' initial focus. */
export function rowInput(root: HTMLElement | null, name: string | null) {
	if (!root) return null;
	const inputs = Array.from(
		root.querySelectorAll<HTMLInputElement>("input[data-per-run]"),
	);
	return (
		inputs.find((input) => name !== null && input.dataset.perRun === name) ??
		inputs[0] ??
		null
	);
}

function Box({ on }: Readonly<{ on: boolean }>) {
	return (
		<span
			aria-hidden="true"
			className={cn(
				"flex size-4 shrink-0 items-center justify-center rounded-md border peer-focus-visible:outline-2 peer-focus-visible:outline-offset-1 peer-focus-visible:outline-ring peer-focus-visible:outline-solid",
				on
					? "border-foreground bg-foreground text-background"
					: "border-border-strong bg-card",
			)}
		>
			{on ? <Check className="size-3" strokeWidth={3} /> : null}
		</span>
	);
}

const ROW_STEP: Readonly<Record<string, 1 | -1>> = {
	ArrowDown: 1,
	ArrowUp: -1,
};

/** ↑ / ↓ move the cursor between the rows' checkboxes and stop at the first and the last (spec §4: popover rows). */
export function stepRow(event: KeyboardEvent<HTMLInputElement>) {
	const step = ROW_STEP[event.key];
	if (!step || !isPlain(event)) return;
	const list = event.currentTarget.closest("ul");
	if (!list) return;
	const inputs = Array.from(
		list.querySelectorAll<HTMLInputElement>("input[data-per-run]"),
	);
	const index = inputs.indexOf(event.currentTarget);
	if (index < 0) return;
	event.preventDefault();
	const next = inputs[Math.min(inputs.length - 1, Math.max(0, index + step))];
	next.focus();
	next.closest("li")?.scrollIntoView?.({ block: "nearest" });
}

function Row({
	row,
	back,
	large,
	onToggle,
}: Readonly<{
	row: AfterRunRow;
	back: string;
	large: boolean;
	onToggle: (name: string, on: boolean) => void;
}>) {
	const backId = useId();
	return (
		<li>
			<label
				className={cn(
					"relative flex items-center gap-2 rounded-md px-2 hover:bg-row-hover",
					large ? "min-h-11" : "h-8",
				)}
			>
				<input
					type="checkbox"
					className="peer sr-only"
					data-per-run={row.name}
					checked={row.on}
					aria-describedby={backId}
					onKeyDown={stepRow}
					onChange={(event) => onToggle(row.name, event.target.checked)}
				/>
				<Box on={row.on} />
				<span className="min-w-0 flex-1 truncate text-[13px] text-foreground leading-4.5">
					{row.label}
				</span>
				<span
					id={backId}
					title={back}
					className="max-w-[55%] shrink-0 truncate text-muted-foreground text-xs leading-4"
				>
					{back}
				</span>
			</label>
		</li>
	);
}

const SECONDARY =
	"rounded-lg border border-border bg-card font-medium text-[13px] text-foreground leading-4.5 hover:bg-row-hover";

function FilterInput({
	value,
	large,
	onChange,
}: Readonly<{
	value: string;
	large: boolean;
	onChange: (value: string) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const label = t("interfaces:workbench.dock.panelFilter", "Filter fields");
	return (
		<input
			type="text"
			value={value}
			aria-label={label}
			placeholder={label}
			onChange={(event) => onChange(event.target.value)}
			className={cn(
				"w-full rounded-lg border border-input bg-card px-2.5 text-foreground placeholder:text-muted-foreground hover:border-border-strong",
				FOCUS_RING,
				large ? "h-11 text-base" : "h-8 text-[13px] leading-4.5",
			)}
		/>
	);
}

export interface PerRunPanelProps {
	readonly rows: readonly AfterRunRow[];
	readonly words: ShortWords;
	readonly showMake: boolean;
	readonly showFilter: boolean;
	/** Touch metrics: 44 px rows and buttons, 16 px filter text. */
	readonly large: boolean;
	readonly actions: Pick<
		FormSessionActions,
		"setPerRun" | "perRunFilesAndDates" | "uncheckAllPerRun" | "closeOverlay"
	>;
	readonly Title: TextSlot;
	readonly Lead: TextSlot;
}

function RowList({
	rows,
	words,
	large,
	onToggle,
}: Readonly<{
	rows: readonly AfterRunRow[];
	words: ShortWords;
	large: boolean;
	onToggle: (name: string, on: boolean) => void;
}>) {
	const { t } = useTranslation("interfaces");
	return (
		<ul className="-mx-1 min-h-0 flex-1 overflow-y-auto px-1 [scrollbar-color:var(--border-strong)_transparent] [scrollbar-width:thin]">
			{rows.map((row) => (
				<Row
					key={row.name}
					row={row}
					back={backText(t, row.back, words)}
					large={large}
					onToggle={onToggle}
				/>
			))}
			{rows.length === 0 ? (
				<li className="px-2 py-2 text-muted-foreground text-xs leading-4">
					{t("interfaces:workbench.dock.panelNoMatch", "No input matches.")}
				</li>
			) : null}
		</ul>
	);
}

function PanelFooter({
	anyOn,
	large,
	actions,
}: Readonly<{
	anyOn: boolean;
	large: boolean;
	actions: PerRunPanelProps["actions"];
}>) {
	const { t } = useTranslation("interfaces");
	const height = large ? "h-11" : "h-7";
	return (
		<div className="flex h-10 items-center justify-between border-hairline border-t">
			<button
				type="button"
				aria-disabled={!anyOn}
				onClick={() => {
					if (anyOn) actions.uncheckAllPerRun();
				}}
				className={cn(
					"rounded-lg px-2.5 font-medium text-[13px] text-ink-2 leading-4.5 hover:bg-row-hover aria-disabled:cursor-not-allowed aria-disabled:opacity-50",
					FOCUS_RING,
					height,
				)}
			>
				{t("interfaces:workbench.dock.panelUncheckAll", "Uncheck all")}
			</button>
			<button
				type="button"
				onClick={() => actions.closeOverlay()}
				className={cn(SECONDARY, "px-3", FOCUS_RING, height)}
			>
				{t("interfaces:workbench.dock.panelDone", "Done")}
			</button>
		</div>
	);
}

/**
 * The "Per run" settings (spec M1 c): a row with a real checkbox per field and what it goes back to, "Make files and
 * dates per run" while nothing is checked, a filter from 12 fields, "Uncheck all" and "Done". A change applies at once.
 */
export function PerRunPanel({
	rows,
	words,
	showMake,
	showFilter,
	large,
	actions,
	Title,
	Lead,
}: Readonly<PerRunPanelProps>) {
	const { t } = useTranslation("interfaces");
	const [query, setQuery] = useState("");
	const rootRef = useRef<HTMLDivElement>(null);
	const make = () => {
		actions.perRunFilesAndDates();
		requestAnimationFrame(() => rowInput(rootRef.current, null)?.focus());
	};
	return (
		<div ref={rootRef} className="flex min-h-0 flex-1 flex-col gap-2">
			<div className="flex flex-col gap-1">
				<Title className="font-semibold text-[13px] text-foreground leading-4.5">
					{t("interfaces:workbench.dock.panelTitle", "Per run")}
				</Title>
				<Lead className="text-muted-foreground text-xs leading-4">
					{t(
						"interfaces:workbench.dock.panelLead",
						"Checked inputs go back to their starting value when a run starts, ready for the next one. Each run takes its own copy.",
					)}
				</Lead>
			</div>
			{showMake ? (
				<button
					type="button"
					onClick={make}
					className={cn(SECONDARY, FOCUS_RING, large ? "h-11" : "h-7")}
				>
					{t(
						"interfaces:workbench.dock.panelMake",
						"Make files and dates per run",
					)}
				</button>
			) : null}
			{showFilter ? (
				<FilterInput value={query} large={large} onChange={setQuery} />
			) : null}
			<RowList
				rows={matchingRows(rows, query)}
				words={words}
				large={large}
				onToggle={actions.setPerRun}
			/>
			<PanelFooter
				anyOn={rows.some((row) => row.on)}
				large={large}
				actions={actions}
			/>
		</div>
	);
}
