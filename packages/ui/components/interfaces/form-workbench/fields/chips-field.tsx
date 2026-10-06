"use client";

import { useTranslation } from "@flow-like/locales";
import { X } from "lucide-react";
import {
	type KeyboardEvent,
	type ReactElement,
	type RefObject,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type { EditHow, FieldProblem } from "../contracts";
import { readDate } from "../model/dates";
import { listOf, valueAt } from "../model/values";
import type { ControlProps } from "./bind";
import {
	type ChipTarget,
	chipMove,
	chipText,
	entersChips,
	itemsFrom,
} from "./chips-model";
import { BOX_FOCUS, DIFFERS_EDGE } from "./control-style";
import { handleEnter } from "./enter";
import { Message } from "./field-frame";
import { isComposing } from "./keys";
import { problemText } from "./problem-text";
import { RecentList } from "./recent-list";
import { type Entry, useEntry } from "./use-entry";

/*
 * A list of values (spec M2, M6): chips with an entry. ↵ or a comma adds, ↵ on an empty entry moves on, ← in an empty
 * entry goes into the chips. The entry has the inline suggestion and the recent list of a text field.
 */

type Item = "text" | "number" | "date";

interface ChipsModel {
	readonly list: readonly string[];
	readonly entryText: string;
	readonly problem: FieldProblem | null;
	commit(next: string, how: EditHow): void;
	add(text: string): boolean;
	removeAt(index: number): void;
}

function useChips(props: ControlProps): ChipsModel {
	const { field, rail, actions, viewer, today } = props;
	const list = listOf(valueAt(rail.values, field.key));
	const [entryText, setEntryText] = useState("");
	const [problem, setProblem] = useState<FieldProblem | null>(null);
	const read = (typed: string) => {
		const anchor = props.dateAnchorFor(field.key);
		const reading = readDate(typed, anchor, today, viewer.dateLocale);
		return reading && reading.iso !== "" ? reading.iso : null;
	};
	const add = (text: string) => {
		const items = itemsFrom(text, field, read);
		if (items === null) {
			setProblem({ code: "date" });
			return false;
		}
		const fresh = items.filter(
			(item) => field.valueType !== "HashSet" || !list.includes(item),
		);
		if (fresh.length > 0) actions.setValue(field.key, [...list, ...fresh]);
		setEntryText("");
		setProblem(null);
		return true;
	};
	return {
		list,
		entryText,
		problem,
		add,
		commit(next) {
			if (next.includes(",")) add(next);
			else {
				setEntryText(next);
				setProblem(null);
			}
		},
		removeAt(index) {
			actions.setValue(
				field.key,
				list.filter((_, at) => at !== index),
			);
		},
	};
}

interface ChipFocus {
	/** Focus lands here after the next render (a chip was removed). */
	later(target: ChipTarget): void;
	now(target: ChipTarget): void;
}

function useChipFocus(
	chips: RefObject<(HTMLSpanElement | null)[]>,
	entry: RefObject<HTMLInputElement | null>,
	count: number,
): ChipFocus {
	const pending = useRef<ChipTarget | null>(null);
	const land = (target: ChipTarget) => {
		const node = target === "entry" ? entry.current : chips.current[target];
		(node ?? entry.current)?.focus();
	};
	// biome-ignore lint/correctness/useExhaustiveDependencies: runs when the chips change
	useLayoutEffect(() => {
		const target = pending.current;
		pending.current = null;
		if (target !== null) land(target);
	}, [count]);
	return {
		later(target) {
			pending.current = target;
		},
		now: land,
	};
}

function Chip({
	text,
	touch,
	remove,
	register,
	onKeyDown,
	onRemove,
}: Readonly<{
	text: string;
	touch: boolean;
	remove: string;
	register: (node: HTMLSpanElement | null) => void;
	onKeyDown: (event: KeyboardEvent<HTMLSpanElement>) => void;
	onRemove: () => void;
}>) {
	return (
		<span
			ref={register}
			tabIndex={-1}
			onKeyDown={onKeyDown}
			className={cx(
				"inline-flex items-center gap-px rounded-md bg-secondary pr-0.5 pl-2 text-[12.5px]/4 font-medium text-secondary-foreground focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring",
				touch
					? "min-h-10 whitespace-normal wrap-anywhere"
					: "h-6.5 whitespace-nowrap",
			)}
		>
			<span>{text}</span>
			<button
				type="button"
				tabIndex={-1}
				aria-label={remove}
				onClick={onRemove}
				className={cx(
					"relative inline-flex shrink-0 items-center justify-center rounded-md p-0 text-muted-foreground hover:bg-row-hover hover:text-foreground",
					touch
						? "size-10 after:absolute after:top-1/2 after:left-1/2 after:size-11 after:-translate-x-1/2 after:-translate-y-1/2 after:content-['']"
						: "size-5",
				)}
			>
				<X aria-hidden className="size-3" strokeWidth={2.25} />
			</button>
		</span>
	);
}

interface View {
	readonly props: ControlProps;
	readonly model: ChipsModel;
	readonly entry: Entry<HTMLInputElement>;
	readonly chips: RefObject<(HTMLSpanElement | null)[]>;
	readonly entryEl: RefObject<HTMLInputElement | null>;
	readonly focus: ChipFocus;
	readonly anchor: RefObject<HTMLFieldSetElement | null>;
	readonly listId: string;
	optionId(index: number): string;
}

/** One chip: ←/→ between chips, ⌫ or Delete removes it, ↵ moves on. */
function chipKeys(view: View, index: number) {
	const { props, model, focus } = view;
	return (event: KeyboardEvent<HTMLSpanElement>) => {
		if (isComposing(event)) return;
		if (handleEnter(event, props.field.key, props.actions)) return;
		const move = chipMove(event.key, index, model.list.length);
		if (!move) return;
		event.preventDefault();
		if (move.kind === "focus") focus.now(move.to);
		else {
			focus.later(move.after);
			model.removeAt(index);
		}
	};
}

function useEntryPlaceholder(view: View) {
	const { t } = useTranslation("interfaces");
	if (view.model.list.length > 0) return undefined;
	return view.entry.canRecall && view.entry.hasRecent
		? t(
				"workbench.field.recentPlaceholder",
				"Type, or press ↓ for recent values",
			)
		: t("workbench.field.chips.placeholder", "Type and press Enter");
}

function EntryInput({ view }: Readonly<{ view: View }>) {
	const { props, entry, model } = view;
	const { bind, disabled, field, actions } = props;
	const placeholder = useEntryPlaceholder(view);
	const open = entry.listOpen;
	const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
		const key = event.key;
		if (
			!isComposing(event) &&
			entersChips(key, model.entryText, model.list.length)
		) {
			event.preventDefault();
			view.focus.now(model.list.length - 1);
			return;
		}
		entry.onKeyDown(event);
	};
	return (
		<input
			ref={(node) => {
				view.entryEl.current = node;
				entry.element.current = node;
			}}
			id={bind.ids.control}
			type="text"
			autoComplete="off"
			value={entry.display}
			disabled={disabled}
			placeholder={placeholder}
			enterKeyHint={props.enterHint(field.key)}
			aria-invalid={bind.invalid || undefined}
			aria-describedby={bind.describedBy()}
			{...(entry.combobox
				? {
						role: "combobox",
						"aria-autocomplete": "both" as const,
						"aria-expanded": open,
						"aria-controls": open ? view.listId : undefined,
						"aria-activedescendant":
							open && entry.active >= 0
								? view.optionId(entry.active)
								: undefined,
					}
				: {})}
			{...bind.focus}
			onChange={entry.onChange}
			onKeyDown={onKeyDown}
			onMouseUp={entry.onMouseUp}
			onPaste={entry.remember}
			onCut={entry.remember}
			onBlur={() => {
				entry.onBlur();
				if (model.entryText.trim() !== "") model.add(model.entryText);
				else actions.blurField(field.key);
			}}
			className={cx(
				"min-w-16 flex-[1_1_64px] border-0 bg-transparent px-1.25 text-foreground outline-none placeholder:text-muted-foreground",
				bind.touch ? "h-11 text-base" : "h-6.5 text-[13.5px]",
			)}
		/>
	);
}

function ChipsBox({ view }: Readonly<{ view: View }>) {
	const { t } = useTranslation("interfaces");
	const { props, model } = view;
	const { bind, viewer, field, disabled } = props;
	const kind: Item = field.itemKind ?? "text";
	return (
		<fieldset
			ref={view.anchor}
			aria-labelledby={bind.ids.label}
			onMouseDown={(event) => {
				if (event.target !== event.currentTarget) return;
				event.preventDefault();
				view.entryEl.current?.focus();
			}}
			className={cx(
				"m-0 flex min-w-0 flex-wrap items-center gap-1 rounded-lg border border-input bg-card text-foreground hover:border-border-strong",
				bind.touch ? "min-h-11 px-1 py-0.75" : "min-h-9 px-1.25 py-1",
				bind.invalid && "border-critical-line hover:border-critical",
				bind.markers.differs && DIFFERS_EDGE,
				BOX_FOCUS,
				disabled && "opacity-60",
			)}
		>
			{model.list.map((item, index) => (
				<Chip
					key={`${index}-${item}`}
					text={chipText(kind, item, viewer.locale)}
					touch={bind.touch}
					remove={t("workbench.field.chips.remove", "Remove {{item}}", {
						item,
					})}
					register={(node) => {
						view.chips.current[index] = node;
					}}
					onKeyDown={chipKeys(view, index)}
					onRemove={() => {
						view.focus.later("entry");
						model.removeAt(index);
					}}
				/>
			))}
			<EntryInput view={view} />
		</fieldset>
	);
}

/** The refs, the model, the entry engine and the ids one list field shares between its parts. */
function useView(props: ControlProps): View {
	const { field, actions, bind } = props;
	const model = useChips(props);
	const chips = useRef<(HTMLSpanElement | null)[]>([]);
	const entryEl = useRef<HTMLInputElement | null>(null);
	const anchor = useRef<HTMLFieldSetElement>(null);
	const focus = useChipFocus(chips, entryEl, model.list.length);
	const onEnter = (event: KeyboardEvent) => {
		if (model.entryText.trim() === "")
			return handleEnter(event, field.key, actions);
		event.preventDefault();
		model.add(model.entryText);
		return true;
	};
	const entry = useEntry<HTMLInputElement>({
		props,
		typed: model.entryText,
		exclude: model.list,
		commit: model.commit,
		onEnter,
	});
	return {
		props,
		model,
		entry,
		chips,
		entryEl,
		focus,
		anchor,
		listId: `${bind.ids.control}-list`,
		optionId: (index) => `${bind.ids.control}-option-${index}`,
	};
}

function Overlay({ view, box }: Readonly<{ view: View; box: ReactElement }>) {
	const { props, entry } = view;
	const { field, actions, viewer } = props;
	return (
		<RecentList
			open={entry.listOpen}
			items={entry.items}
			active={entry.active}
			listId={view.listId}
			optionId={view.optionId}
			label={field.label}
			viewer={viewer}
			anchorRef={view.anchor}
			anchor={box}
			onFill={entry.fill}
			onDontSave={() => actions.dontSave(field.key)}
			onClose={() => actions.closeList()}
		/>
	);
}

export function ChipsField(props: ControlProps) {
	const { t } = useTranslation("interfaces");
	const { field, bind, viewer } = props;
	const view = useView(props);
	const { problem } = view.model;
	const message = problem ? problemText({ t, field, viewer, problem }) : null;
	return (
		<>
			<Overlay view={view} box={<ChipsBox view={view} />} />
			{message ? (
				<Message id={`${bind.ids.control}-local`}>{message}</Message>
			) : null}
		</>
	);
}
