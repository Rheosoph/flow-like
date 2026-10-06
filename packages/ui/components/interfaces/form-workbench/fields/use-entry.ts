import {
	type ChangeEvent,
	type KeyboardEvent,
	type RefObject,
	useCallback,
	useRef,
} from "react";
import type { EditHow, RecentValue } from "../contracts";
import { completion, recentValues } from "../model/recent";
import type { ControlProps } from "./bind";
import {
	type SuggestionState,
	displayWith,
	editHow,
	useSuggestion,
} from "./completion";
import { isComposing, isPlainEnter } from "./keys";

/*
 * What a one-line text entry does besides typing (spec M2, M6): the inline suggestion, the recent-values list,
 * and the keys that drive both. Shared by the text field and the entry of a list field. The element it drives is
 * a textarea or an input.
 */

type El = HTMLTextAreaElement | HTMLInputElement;

export interface EntryOptions {
	readonly props: ControlProps;
	/** The text in the entry: the rail's value, or a list field's pending entry. */
	readonly typed: string;
	/** Values already in the field (a list's chips): never suggested. */
	readonly exclude?: readonly string[];
	/** The text holds a line break: ↵ adds a line, there is no suggestion and no list. */
	readonly multiline?: boolean;
	/** The text changed by typing. */
	readonly commit: (next: string, how: EditHow) => void;
	/** ↵ in a plain entry (nothing open, no suggestion): the field's own rule. Returns whether it took the key. */
	readonly onEnter: (event: KeyboardEvent) => boolean;
}

export interface Entry<E extends El> {
	readonly element: RefObject<E | null>;
	readonly display: string;
	readonly suggestion: SuggestionState;
	readonly canRecall: boolean;
	readonly hasRecent: boolean;
	/** The input has combobox semantics: it can offer recent values and has some. */
	readonly combobox: boolean;
	readonly listOpen: boolean;
	readonly items: readonly RecentValue[];
	readonly active: number;
	onChange(event: ChangeEvent<E>): void;
	onKeyDown(event: KeyboardEvent<E>): void;
	onBlur(): void;
	onMouseUp(): void;
	remember(event: { currentTarget: E }): void;
	fill(value: string): void;
}

interface Context {
	readonly options: EntryOptions;
	readonly suggestion: SuggestionState;
	readonly items: readonly RecentValue[];
	readonly active: number;
	readonly listOpen: boolean;
	readonly canRecall: boolean;
	readonly hasRecent: boolean;
	readonly fill: (value: string) => void;
}

type KeyHandler = (event: KeyboardEvent<El>, context: Context) => void;

const keyOf = (context: Context) => context.options.props.field.key;

function takeSuggestion(context: Context) {
	const { suggestion, options } = context;
	if (suggestion.suggestion === null) return false;
	options.commit(suggestion.suggestion, "inPlace");
	suggestion.drop(suggestion.suggestion.length);
	return true;
}

const onEnter: KeyHandler = (event, context) => {
	const { suggestion, listOpen, items, active, options } = context;
	if (event.shiftKey || !isPlainEnter(event) || options.multiline) return;
	if (listOpen && items[active]) {
		event.preventDefault();
		context.fill(items[active].value);
		return;
	}
	suggestion.drop();
	options.onEnter(event);
};

const onEscape: KeyHandler = (event, context) => {
	const { props } = context.options;
	if (context.listOpen) {
		event.preventDefault();
		event.stopPropagation();
		props.actions.closeList();
		return;
	}
	if (context.suggestion.suggestion === null) return;
	event.preventDefault();
	event.stopPropagation();
	context.suggestion.drop();
};

const onArrowDown: KeyHandler = (event, context) => {
	const { props, multiline } = context.options;
	if (multiline || event.shiftKey || event.altKey || !context.canRecall) return;
	if (context.listOpen) {
		event.preventDefault();
		if (context.active < context.items.length - 1) props.actions.moveList(1);
		return;
	}
	if (!context.hasRecent) return;
	event.preventDefault();
	props.actions.openList("recent", keyOf(context));
};

const onArrowUp: KeyHandler = (event, context) => {
	if (!context.listOpen) {
		context.suggestion.drop();
		return;
	}
	event.preventDefault();
	if (context.active > 0) context.options.props.actions.moveList(-1);
};

const onTake: KeyHandler = (event, context) => {
	if (event.shiftKey || event.altKey || event.metaKey || event.ctrlKey) return;
	if (takeSuggestion(context)) event.preventDefault();
};

const onHome: KeyHandler = (_event, context) => context.suggestion.drop(0);

const onLeft: KeyHandler = (_event, context) =>
	context.suggestion.drop(context.options.typed.length);

const onTab: KeyHandler = (_event, context) => context.suggestion.drop();

const onDelete: KeyHandler = (event, context) => {
	if (!context.listOpen || !context.items[context.active]) return;
	event.preventDefault();
	context.options.props.actions.forgetRecent(
		keyOf(context),
		context.items[context.active].value,
	);
};

const KEYS: Readonly<Record<string, KeyHandler>> = {
	Enter: onEnter,
	Escape: onEscape,
	ArrowDown: onArrowDown,
	ArrowUp: onArrowUp,
	ArrowRight: onTake,
	End: onTake,
	ArrowLeft: onLeft,
	Home: onHome,
	Tab: onTab,
	Delete: onDelete,
};

export function useEntry<E extends El>(options: EntryOptions): Entry<E> {
	const { props, typed } = options;
	const { field, actions, list } = props;
	const element = useRef<E | null>(null);
	const multiline = options.multiline === true;
	const recall = multiline ? null : props.recallFor(field.key);
	const exclude = options.exclude ?? [];
	const find = (text: string) =>
		recall
			? completion(recall.source, field, text, recall.forgotten, exclude)
			: null;
	const suggestion = useSuggestion(element, typed, find);
	const listOpen =
		recall !== null && list?.kind === "recent" && list.key === field.key;
	const items =
		recall && listOpen
			? recentValues(recall.source, field, {
					query: typed,
					forgotten: recall.forgotten,
					exclude,
				})
			: [];
	const hasRecent = recall
		? recentValues(recall.source, field, {
				forgotten: recall.forgotten,
				exclude,
				max: 1,
			}).length > 0
		: false;
	const active = Math.min(list?.active ?? -1, items.length - 1);
	const fill = useCallback(
		(value: string) => {
			options.commit(value, "replace");
			suggestion.drop();
			actions.closeList();
		},
		[options, suggestion, actions],
	);
	const context: Context = {
		options,
		suggestion,
		items,
		active,
		listOpen,
		canRecall: recall !== null,
		hasRecent,
		fill,
	};
	return {
		element,
		display: displayWith(typed, suggestion.suggestion),
		suggestion,
		canRecall: recall !== null,
		hasRecent,
		combobox: recall !== null && hasRecent,
		listOpen: listOpen && items.length > 0,
		items,
		active,
		onChange: (event) => {
			const next = event.target.value;
			const native = event.nativeEvent as InputEvent;
			const how = editHow(typed, suggestion.selection.current);
			if (native.isComposing || (native.inputType ?? "").startsWith("delete"))
				suggestion.drop();
			else suggestion.arm(next);
			options.commit(next, how);
		},
		onKeyDown: (event) => {
			suggestion.remember(event.currentTarget);
			if (isComposing(event)) return;
			KEYS[event.key]?.(event, context);
		},
		onBlur: () => {
			suggestion.drop();
			if (listOpen) actions.closeList();
		},
		onMouseUp: () => {
			if (suggestion.suggestion !== null) suggestion.drop();
		},
		remember: (event) => suggestion.remember(event.currentTarget),
		fill,
	};
}
