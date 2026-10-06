import type {
	FormSessionActions,
	FormSessionState,
	RouteNav,
	WorkbenchLayout,
	WorkbenchViewProps,
} from "../contracts";
import { DESKTOP_LAYOUT } from "./layouts";

/*
 * Recording stand-ins for `FormSessionActions` and `RouteNav` (PLAN §12.6). UI lanes render a fixture
 * with these and assert on the calls; nothing here changes the state.
 */

export type ActionName = keyof FormSessionActions;

/** One recorded call: the action's name and its arguments, typed per action. */
export type ActionCall = {
	[K in ActionName]: {
		readonly name: K;
		readonly args: Parameters<FormSessionActions[K]>;
	};
}[ActionName];

/** Every action of the contract; a new contract action fails the type check here until it is listed. */
const ACTION_NAMES: Readonly<Record<ActionName, true>> = {
	setValue: true,
	setText: true,
	commitText: true,
	blurField: true,
	resetField: true,
	resetAll: true,
	pickFiles: true,
	removeFile: true,
	retryFile: true,
	removeNextFile: true,
	clearNextFiles: true,
	addLeftOut: true,
	run: true,
	enter: true,
	runAgain: true,
	stop: true,
	removeFromQueue: true,
	clearQueue: true,
	resumeQueue: true,
	useInputs: true,
	removeRun: true,
	selectRun: true,
	pinRun: true,
	setRailTab: true,
	setPane: true,
	setFilter: true,
	openOverlay: true,
	closeOverlay: true,
	openList: true,
	moveList: true,
	closeList: true,
	setPerRun: true,
	uncheckAllPerRun: true,
	perRunFilesAndDates: true,
	answerQuestion: true,
	forgetRecent: true,
	dontSave: true,
	applyPreset: true,
	savePreset: true,
	updatePreset: true,
	deletePreset: true,
	resetToPreset: true,
	undo: true,
	respondInteraction: true,
	dismissMessage: true,
	railKey: true,
	focusHandled: true,
	markSeen: true,
	setLayout: true,
};

/** The names of every action, in contract order. */
export const ACTION_NAME_LIST = Object.keys(ACTION_NAMES) as ActionName[];

export interface FakeActionsOptions {
	/** Called after each call is recorded (the visual harness applies `setLayout` here). */
	readonly onCall?: (call: ActionCall) => void;
}

export interface FakeActions {
	readonly actions: FormSessionActions;
	/** Every call so far, oldest first. */
	readonly calls: ActionCall[];
	/** The arguments of each call to one action, oldest first. */
	argsOf<K extends ActionName>(name: K): Parameters<FormSessionActions[K]>[];
	/** Forget the recorded calls. */
	clear(): void;
}

/** `FormSessionActions` that only record their calls; returns `{ actions, calls }`. */
export function fakeActions(options: FakeActionsOptions = {}): FakeActions {
	const calls: ActionCall[] = [];
	const recorder =
		(name: ActionName) =>
		(...args: unknown[]) => {
			const call = { name, args } as ActionCall;
			calls.push(call);
			options.onCall?.(call);
		};
	const actions = Object.fromEntries(
		ACTION_NAME_LIST.map((name) => [name, recorder(name)]),
	) as unknown as FormSessionActions;
	return {
		actions,
		calls,
		argsOf<K extends ActionName>(name: K) {
			return calls
				.filter((call) => call.name === name)
				.map((call) => call.args as Parameters<FormSessionActions[K]>);
		},
		clear() {
			calls.length = 0;
		},
	};
}

/** A `RouteNav` that records every route it was asked to go to. */
export interface FakeRoutes extends RouteNav {
	readonly gone: string[];
}

export function fakeRoutes(
	labels: Readonly<Record<string, string>> = {},
): FakeRoutes {
	const gone: string[] = [];
	return {
		labels,
		gone,
		go(route) {
			gone.push(route);
		},
	};
}

export interface FakeView extends FakeActions {
	readonly props: WorkbenchViewProps;
	readonly routes: FakeRoutes;
}

/**
 * Props for Rail, Dock (plus `variant`) or Stage: the state with the given layout (also written to
 * `state.layout`, as the shell does), recording actions and routes.
 */
export function fakeView(
	state: FormSessionState,
	options: {
		readonly layout?: WorkbenchLayout;
		readonly routeLabels?: Readonly<Record<string, string>>;
		readonly onCall?: (call: ActionCall) => void;
	} = {},
): FakeView {
	const layout = options.layout ?? state.layout ?? DESKTOP_LAYOUT;
	const fake = fakeActions({ onCall: options.onCall });
	const routes = fakeRoutes(options.routeLabels);
	return {
		...fake,
		routes,
		props: {
			state: { ...state, layout },
			actions: fake.actions,
			layout,
			routes,
		},
	};
}
