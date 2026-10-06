"use client";

import { useTranslation } from "@flow-like/locales";
import { Info } from "lucide-react";
import {
	type KeyboardEvent,
	type RefObject,
	useLayoutEffect,
	useRef,
} from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type {
	FileSlot,
	FormSessionActions,
	LeftOutFile,
	WorkbenchField,
} from "../contracts";
import { nextLineParts } from "../model/next-files";
import { textButton } from "./control-style";
import { ProgressBar, RemoveButton, SlotIcon, useSlotMeta } from "./file-row";
import { isComposing, isPlain, swallowRepeat } from "./keys";

/*
 * Next files (spec M3): the "Next: … · 4 more" line under a one-file field with Show / Hide, and the list in the rail's
 * flow. The list is driven by the keys of its rows: ↑/↓ move, ⌫ or Delete removes, Esc closes and the cursor goes
 * back to Hide. Rows are focused by hand (`tabIndex -1`); the list is no Tab stop of its own.
 */

type ListActions = Pick<
	FormSessionActions,
	| "openList"
	| "closeList"
	| "removeNextFile"
	| "clearNextFiles"
	| "retryFile"
	| "addLeftOut"
>;

export interface NextFilesProps {
	readonly field: WorkbenchField;
	readonly next: readonly FileSlot[];
	readonly leftOut: readonly LeftOutFile[];
	readonly open: boolean;
	readonly touch: boolean;
	readonly decimalSign: "." | ",";
	readonly listId: string;
	readonly actions: ListActions;
	readonly toggle: RefObject<HTMLButtonElement | null>;
	readonly rows: RefObject<(HTMLElement | null)[]>;
	/** The list is gone (emptied, closed): the cursor goes back to the file field. */
	readonly onGone: () => void;
}

function Toggle({ props }: Readonly<{ props: NextFilesProps }>) {
	const { t } = useTranslation("interfaces");
	const { open, next, actions, field } = props;
	return (
		<button
			ref={props.toggle}
			type="button"
			aria-expanded={open}
			aria-controls={props.listId}
			aria-label={
				open
					? t("workbench.field.file.hideAria", "Hide the next files")
					: t("workbench.field.file.showAria", "Show the next files ({{n}})", {
							n: next.length,
						})
			}
			onClick={() =>
				open ? actions.closeList() : actions.openList("nextFiles", field.key)
			}
			onKeyDown={(event) => {
				if (isComposing(event) || event.key !== "ArrowDown" || !isPlain(event))
					return;
				event.preventDefault();
				if (open) props.rows.current[0]?.focus();
				else actions.openList("nextFiles", field.key);
			}}
			className={textButton(props.touch)}
		>
			{open
				? t("workbench.field.file.hide", "Hide")
				: t("workbench.field.file.show", "Show")}
		</button>
	);
}

/** "Next: invoice-RE-2026-0919.pdf · 8 more" with Show / Hide at the right. */
export function NextLine({ props }: Readonly<{ props: NextFilesProps }>) {
	const { t } = useTranslation("interfaces");
	const parts = nextLineParts(props.next);
	if (!parts) return null;
	return (
		<div className="flex min-h-4 items-center gap-1.5 text-xs/4">
			<span className="flex min-w-0 flex-1 items-baseline">
				<span className="shrink-0 whitespace-pre text-muted-foreground">
					{t("workbench.field.file.nextLead", "Next:")}{" "}
				</span>
				<span className="min-w-0 truncate font-mono text-ink-2">
					{parts.name}
				</span>
				{parts.more > 0 ? (
					<span className="shrink-0 whitespace-pre text-muted-foreground">
						{" · "}
						{t("workbench.field.file.nextMore", "{{n}} more", {
							n: parts.more,
						})}
					</span>
				) : null}
			</span>
			<Toggle props={props} />
		</div>
	);
}

function LeftOutNote({ props }: Readonly<{ props: NextFilesProps }>) {
	const { t } = useTranslation("interfaces");
	const { leftOut, actions, field } = props;
	if (leftOut.length === 0) return null;
	const one = leftOut.length === 1;
	return (
		<div className="flex min-h-8 items-center gap-2 border-b border-hairline py-1 pr-1 pl-2.5">
			<Info aria-hidden className="size-3.5 shrink-0 text-info" />
			<span className="min-w-0 flex-1 text-xs/4 text-ink-2">
				{one
					? t(
							"workbench.field.file.leftOutOne",
							"{{name}} was already sent in run {{n}} and was left out.",
							{ name: leftOut[0].slot.name, n: leftOut[0].n },
						)
					: t(
							"workbench.field.file.leftOutMany",
							"{{n}} files were already sent in earlier runs and were left out.",
							{ n: leftOut.length },
						)}
			</span>
			<button
				type="button"
				onClick={() => actions.addLeftOut(field.name)}
				onKeyDown={swallowRepeat}
				className={textButton(props.touch)}
			>
				{one
					? t("workbench.field.file.addIt", "Add it")
					: t("workbench.field.file.addThem", "Add them")}
			</button>
		</div>
	);
}

/** After a row is removed the cursor goes to a neighbour; with none left it goes back to the file field. */
function useRowFocus(props: NextFilesProps) {
	const pending = useRef<number | null>(null);
	const count = props.next.length;
	// biome-ignore lint/correctness/useExhaustiveDependencies: runs when the rows change
	useLayoutEffect(() => {
		const index = pending.current;
		pending.current = null;
		if (index === null) return;
		const row = props.rows.current[Math.min(index, count - 1)];
		if (count > 0 && row) row.focus();
		else props.onGone();
	}, [count]);
	return (index: number) => {
		pending.current = index;
	};
}

function Row({
	props,
	slot,
	index,
	later,
}: Readonly<{
	props: NextFilesProps;
	slot: FileSlot;
	index: number;
	later: (index: number) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const { actions, field, touch } = props;
	const meta = useSlotMeta(slot, props.decimalSign);
	const focusAt = (at: number) => props.rows.current[at]?.focus();
	const remove = () => {
		later(index);
		actions.removeNextFile(field.name, slot.id);
	};
	const onKeyDown = (event: KeyboardEvent<HTMLLIElement>) => {
		if (isComposing(event)) return;
		const last = props.next.length - 1;
		const moves: Readonly<Record<string, () => void>> = {
			ArrowDown: () => focusAt(Math.min(index + 1, last)),
			ArrowUp: () =>
				index === 0 ? props.toggle.current?.focus() : focusAt(index - 1),
			Home: () => focusAt(0),
			End: () => focusAt(last),
			Escape: () => {
				event.stopPropagation();
				actions.closeList();
				props.toggle.current?.focus();
			},
			Backspace: remove,
			Delete: remove,
		};
		const act = moves[event.key];
		if (!act || (event.key.length > 1 && !isPlain(event))) return;
		event.preventDefault();
		act();
	};
	return (
		<li
			ref={(node) => {
				props.rows.current[index] = node;
			}}
			tabIndex={-1}
			onKeyDown={onKeyDown}
			className={cx(
				"relative flex items-center gap-2 py-0 pr-1 pl-2.5 outline-none focus-visible:bg-row-selected",
				touch ? "min-h-12" : "h-8",
			)}
		>
			<SlotIcon slot={slot} />
			<span
				title={slot.name}
				className="min-w-0 flex-1 truncate font-mono text-[12.5px]/4"
			>
				{slot.name}
			</span>
			<span
				className={cx(
					"shrink-0 font-mono text-xs/4 tabular-nums [word-spacing:-0.25em]",
					meta.critical ? "text-critical" : "text-muted-foreground",
				)}
			>
				{meta.text}
			</span>
			{slot.state === "failed" ? (
				<button
					type="button"
					onClick={() => actions.retryFile(slot.id)}
					onKeyDown={swallowRepeat}
					aria-label={t(
						"workbench.field.file.tryAgainAria",
						"Try sending {{name}} again",
						{
							name: slot.name,
						},
					)}
					className={textButton(touch)}
				>
					{t("workbench.field.file.tryAgain", "Try again")}
				</button>
			) : null}
			<RemoveButton
				label={t(
					"workbench.field.file.removeFromNext",
					"Remove {{name}} from the next files",
					{ name: slot.name },
				)}
				touch={touch}
				small
				onRemove={remove}
			/>
			<ProgressBar slot={slot} />
		</li>
	);
}

/** The open list: the left-out note, six rows and then a scroll, "Remove all next files". */
export function NextList({ props }: Readonly<{ props: NextFilesProps }>) {
	const { t } = useTranslation("interfaces");
	const later = useRowFocus(props);
	if (!props.open || props.next.length === 0) return null;
	return (
		<div
			id={props.listId}
			className="overflow-hidden rounded-lg border border-border bg-card"
		>
			<LeftOutNote props={props} />
			<ul
				aria-label={t("workbench.field.file.listAria", "Next files")}
				className={cx(
					"m-0 list-none divide-y divide-hairline overflow-y-auto p-0",
					props.touch ? "max-h-72" : "max-h-48",
				)}
			>
				{props.next.map((slot, index) => (
					<Row
						key={slot.id}
						props={props}
						slot={slot}
						index={index}
						later={later}
					/>
				))}
			</ul>
			<div className="border-t border-hairline">
				<button
					type="button"
					onClick={() => {
						later(0);
						props.actions.clearNextFiles(props.field.name);
					}}
					className={cx(
						"flex w-full items-center px-2.5 text-left text-[12.5px]/4.5 text-ink-2 hover:bg-row-hover",
						props.touch ? "min-h-11" : "h-8",
					)}
				>
					{t("workbench.field.file.removeAllNext", "Remove all next files")}
				</button>
			</div>
		</div>
	);
}
