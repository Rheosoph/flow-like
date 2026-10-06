import { useSyncExternalStore } from "react";
import type {
	FieldControlProps,
	FieldKey,
	FormSessionState,
	WorkbenchField,
	WorkbenchLayout,
} from "../contracts";
import { dateAnchorOf, readDate } from "../model/dates";
import { enterHint } from "../model/keyboard";
import { fieldMarkers } from "../model/markers";
import { recallFor } from "../model/recent";
import { blockedNames } from "../model/validate";
import { withValue } from "../model/values";
import {
	type ActionCall,
	type FakeActions,
	fakeActions,
} from "../testing/fake-actions";
import { FIXTURE_TODAY, fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT } from "../testing/layouts";
import { FieldControl } from "./field-control";

/*
 * Test support for the field controls: a live field whose state answers the few actions a control relies on
 * (typing, lists, commit), records every call, and renders the real `FieldControl` from a fixture. Not part of the
 * product bundle; only DOM tests import it, after `installWorkbenchDom()`.
 */

type Update = (state: FormSessionState) => FormSessionState;

const without = <T,>(record: Readonly<Record<string, T>>, key: string) => {
	const next = { ...record };
	delete next[key];
	return next;
};

function flatten(fields: readonly WorkbenchField[]): WorkbenchField[] {
	return fields.flatMap((field) => [field, ...flatten(field.props)]);
}

function commitDate(state: FormSessionState, key: FieldKey): FormSessionState {
	const text = state.rail.texts[key];
	if (text === undefined) return state;
	const reading = readDate(
		text,
		dateAnchorOf(state, key, FIXTURE_TODAY),
		FIXTURE_TODAY,
		state.form.viewer.dateLocale,
	);
	const texts = without(state.rail.texts, key);
	if (!reading)
		return {
			...state,
			rail: {
				...state.rail,
				problems: { ...state.rail.problems, [key]: { code: "date" } },
			},
		};
	return {
		...state,
		rail: {
			...state.rail,
			texts,
			values: withValue(state.rail.values, key, reading.iso),
		},
	};
}

type Apply = (state: FormSessionState, call: ActionCall) => FormSessionState;

const rail = (
	state: FormSessionState,
	patch: Partial<FormSessionState["rail"]>,
) => ({
	...state,
	rail: { ...state.rail, ...patch },
});

const onSetValue: Apply = (state, call) => {
	if (call.name !== "setValue") return state;
	const [key, value] = call.args;
	return rail(state, {
		values: withValue(state.rail.values, key, value),
		texts: without(state.rail.texts, key),
		problems: without(state.rail.problems, key),
	});
};

const onSetText: Apply = (state, call) => {
	if (call.name !== "setText") return state;
	const [key, text] = call.args;
	return rail(state, {
		texts: { ...state.rail.texts, [key]: text },
		problems: without(state.rail.problems, key),
	});
};

const onCommit: Apply = (state, call) =>
	call.name === "commitText" || call.name === "blurField"
		? commitDate(state, call.args[0])
		: state;

const onOpenList: Apply = (state, call) => {
	if (call.name !== "openList") return state;
	const [kind, key] = call.args;
	const list = { kind, key, active: kind === "recent" ? 0 : -1 };
	return { ...state, view: { ...state.view, list } };
};

const onCloseList: Apply = (state) => ({
	...state,
	view: { ...state.view, list: null },
});

const onMoveList: Apply = (state, call) => {
	const { list } = state.view;
	if (call.name !== "moveList" || !list) return state;
	const active = Math.max(-1, list.active + call.args[0]);
	return { ...state, view: { ...state.view, list: { ...list, active } } };
};

const HANDLERS: Readonly<Partial<Record<ActionCall["name"], Apply>>> = {
	setValue: onSetValue,
	setText: onSetText,
	commitText: onCommit,
	blurField: onCommit,
	openList: onOpenList,
	closeList: onCloseList,
	moveList: onMoveList,
};

function applyCall(
	state: FormSessionState,
	call: ActionCall,
): FormSessionState {
	const handler = HANDLERS[call.name];
	return handler ? handler(state, call) : state;
}

/** A fixture state under a store the controls write to; `calls` and `state` are what a test asserts on. */
export class LiveStore {
	state: FormSessionState;
	readonly fake: FakeActions;
	readonly layout: WorkbenchLayout;
	readonly today = FIXTURE_TODAY;
	private readonly listeners = new Set<() => void>();

	constructor(state: FormSessionState, layout: WorkbenchLayout) {
		this.state = state;
		this.layout = layout;
		this.fake = fakeActions({
			onCall: (call) => this.update((current) => applyCall(current, call)),
		});
	}

	get calls() {
		return this.fake.calls;
	}

	update(change: Update) {
		this.state = change(this.state);
		for (const listener of this.listeners) listener();
	}

	subscribe = (listener: () => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	snapshot = () => this.state;
}

export function liveStore(
	name: Parameters<typeof fixture>[0],
	options: {
		readonly layout?: WorkbenchLayout;
		readonly change?: Update;
	} = {},
) {
	const base = fixture(name);
	const state = options.change ? options.change(base) : base;
	return new LiveStore(state, options.layout ?? DESKTOP_LAYOUT);
}

/** The props a rail would hand one field of the live state (the model's own derivations, as `Rail` computes them). */
export function propsFor(
	store: LiveStore,
	name: string,
	overrides: Partial<FieldControlProps> = {},
): FieldControlProps {
	const { state, layout } = store;
	const all = flatten(state.form.fields);
	const byKey = (key: FieldKey) =>
		all.find((field) => field.key === key) as WorkbenchField;
	const field = all.find(
		(item) => item.name === name || item.key === name,
	) as WorkbenchField;
	const blocked = blockedNames(state.form.fields, state.form.host);
	return {
		field,
		rail: state.rail,
		markersFor: (key) => fieldMarkers(state, byKey(key)),
		recallFor: (key) => recallFor(state, byKey(key), layout.finePointer),
		dateAnchorFor: (key) => dateAnchorOf(state, key, store.today),
		enterHint: (key) =>
			enterHint(state.form.fields, state.rail.values, key, blocked),
		blocked: blocked.includes(field.name),
		list: state.view.list,
		host: state.form.host,
		viewer: state.form.viewer,
		layout,
		today: store.today,
		disabled: false,
		actions: store.fake.actions,
		...overrides,
	};
}

interface LiveFieldProps {
	readonly store: LiveStore;
	readonly name: string;
	readonly overrides?: Partial<FieldControlProps>;
}

/** `FieldControl` for one field of a live store; re-renders as the store changes. */
export function LiveField(props: Readonly<LiveFieldProps>) {
	const { store, name, overrides } = props;
	useSyncExternalStore(store.subscribe, store.snapshot);
	return <FieldControl {...propsFor(store, name, overrides)} />;
}

/** A fixture state with one field changed (kind, options, flags): for kinds and shapes no fixture has. */
export const patchField =
	(name: string, patch: Partial<WorkbenchField>) =>
	(state: FormSessionState): FormSessionState => ({
		...state,
		form: {
			...state.form,
			fields: state.form.fields.map((field) =>
				field.name === name ? { ...field, ...patch } : field,
			),
		},
	});

/** A fixture state whose rail holds other values, texts or problems. */
export const patchRail =
	(patch: Partial<FormSessionState["rail"]>) =>
	(state: FormSessionState): FormSessionState => ({
		...state,
		rail: { ...state.rail, ...patch },
	});
