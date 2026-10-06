/*
 * The dock's status line as something to draw: text, tone, icon and the buttons at its right end. One builder per
 * `DockLine` kind, in a table of arrow functions with literal keys (PLAN §9). The buttons close over the session
 * actions, so the components only render.
 */
import type {
	DockMessage,
	DockMessageEntry,
	FormSessionActions,
	FormSessionState,
	ShortWords,
	StartMessage,
} from "../contracts";
import { pairing } from "../model/per-run";
import {
	type CopyContext,
	type InterfacesT,
	capSentence,
	failureText,
	holdText,
	joinLabels,
	messageText,
	offerText,
	seriesEndText,
} from "./copy";
import { type PhoneLine, leftOutFieldOf, secretCheckOf } from "./dock-view";

export type LineTone = "info" | "critical" | "ink" | "muted";
export type LineIcon = "none" | "spinner" | "alert" | "bar";

export interface LineAction {
	readonly id: string;
	readonly label: string;
	readonly ariaLabel?: string;
	readonly run: () => void;
}

export interface LineView {
	readonly kind: PhoneLine["kind"];
	readonly tone: LineTone;
	readonly icon: LineIcon;
	/** Weight 500 (a state or an answer to a key press) against 400 (a passive count). */
	readonly strong: boolean;
	readonly text: string;
	readonly actions: readonly LineAction[];
	/** The ▲▼ buttons' names; null for lines that have none. */
	readonly steps: { readonly prev: string; readonly next: string } | null;
}

export interface LineContext {
	readonly t: InterfacesT;
	readonly copy: CopyContext;
	/** The viewer's short value words: a start message names its run's values with them. */
	readonly words: ShortWords;
	readonly actions: FormSessionActions;
	readonly state: FormSessionState;
}

type Parts = Partial<LineView> & Pick<LineView, "tone" | "text">;

const view = (kind: PhoneLine["kind"], parts: Parts): LineView => ({
	icon: "none",
	strong: false,
	actions: [],
	steps: null,
	...parts,
	kind,
});

const undoAction = ({ t, actions }: LineContext): LineAction => ({
	id: "undo",
	label: t("interfaces:workbench.dock.undo", "Undo"),
	run: () => actions.undo(),
});

const clearQueueAction = ({ t, actions }: LineContext): LineAction => ({
	id: "clear",
	label: t("interfaces:workbench.dock.clearQueue", "Clear queue"),
	run: () => actions.clearQueue(),
});

const yesNo = ({ t, actions }: LineContext): readonly LineAction[] => [
	{
		id: "yes",
		label: t("interfaces:workbench.dock.yes", "Yes"),
		run: () => actions.answerQuestion(true),
	},
	{
		id: "no",
		label: t("interfaces:workbench.dock.no", "No"),
		run: () => actions.answerQuestion(false),
	},
];

function leftOutAction(
	entry: DockMessageEntry,
	context: LineContext,
): readonly LineAction[] {
	const { message } = entry;
	if (message.kind !== "leftOut") return [];
	const name = leftOutFieldOf(context.state, message.files);
	if (name === null) return [];
	const { t, actions } = context;
	return [
		{
			id: "add",
			label:
				message.files.length === 1
					? t("interfaces:workbench.dock.addIt", "Add it")
					: t("interfaces:workbench.dock.addThem", "Add them"),
			run: () => actions.addLeftOut(name),
		},
	];
}

function messageActions(entry: DockMessageEntry, context: LineContext) {
	if (entry.message.kind === "leftOut") return leftOutAction(entry, context);
	return entry.undo ? [undoAction(context)] : [];
}

/** The start message's pairing in the viewer's words, read again from its run's copy (the stored one while the run is gone). */
function startInWords(start: StartMessage, context: LineContext): StartMessage {
	const { state, words } = context;
	const run = state.runs.find((item) => item.id === start.runId);
	if (!run) return start;
	const pairs = pairing(
		state.form.fields,
		run.copy.values,
		run.copy.perRun,
		secretCheckOf(state.memory.prefs.noSave),
		words,
	);
	return { ...start, pairs };
}

const shownMessage = (
	message: DockMessage,
	context: LineContext,
): DockMessage =>
	message.kind === "start"
		? { kind: "start", start: startInWords(message.start, context) }
		: message;

/** While the queue is on hold, a line that outranks the hold keeps its "Resume queue" reachable. */
const resumeWhileHeld = ({
	t,
	actions,
	state,
}: LineContext): readonly LineAction[] =>
	state.queue.hold && state.form.fields.length > 0
		? [
				{
					id: "resume",
					label: t("interfaces:workbench.dock.resumeQueue", "Resume queue"),
					run: () => actions.resumeQueue(),
				},
			]
		: [];

type Builder<K extends PhoneLine["kind"]> = (
	line: Extract<PhoneLine, { kind: K }>,
	context: LineContext,
) => LineView;

const LINE_VIEW: { readonly [K in PhoneLine["kind"]]: Builder<K> } = {
	sending: (line, { t }) =>
		view("sending", {
			tone: "info",
			icon: "spinner",
			strong: true,
			text:
				line.total === 1
					? t("interfaces:workbench.dock.sendingOne", "Sending 1 file")
					: t(
							"interfaces:workbench.dock.sending",
							"Sending {{current}} of {{total}} files",
							{ current: line.current, total: line.total },
						),
		}),
	problems: (line, context) => {
		const { t } = context;
		return view("problems", {
			tone: "critical",
			icon: "alert",
			strong: true,
			text: t(
				"interfaces:workbench.dock.problems",
				"{{count}} fields need a look",
				{ count: line.count, defaultValue_one: "1 field needs a look" },
			),
			actions: resumeWhileHeld(context),
			steps: {
				prev: t(
					"interfaces:workbench.dock.problemsPrev",
					"Previous field that needs a look",
				),
				next: t(
					"interfaces:workbench.dock.problemsNext",
					"Next field that needs a look",
				),
			},
		});
	},
	hold: (line, context) =>
		view("hold", {
			tone: "critical",
			icon: "alert",
			strong: true,
			text: holdText(context.t, line.hold),
			actions: [
				{
					id: "resume",
					label: context.t("interfaces:workbench.dock.resume", "Resume"),
					run: () => context.actions.resumeQueue(),
				},
				clearQueueAction(context),
			],
		}),
	message: (line, context) => {
		const { message } = line.entry;
		const failed = message.kind === "answerFailed";
		return view("message", {
			tone: failed ? "critical" : "ink",
			icon: failed ? "alert" : "none",
			strong: true,
			text: messageText(
				context.t,
				shownMessage(message, context),
				context.copy,
			),
			actions: messageActions(line.entry, context),
		});
	},
	failure: (line, { t, actions }) => {
		const [first] = line.runs;
		return view("failure", {
			tone: "critical",
			icon: "alert",
			strong: true,
			text:
				line.runs.length === 1
					? failureText(t, first)
					: t("interfaces:workbench.dock.failures", "{{count}} runs failed.", {
							count: line.runs.length,
							defaultValue_one: "{{count}} run failed.",
						}),
			actions: [
				{
					id: "show",
					label: t("interfaces:workbench.dock.show", "Show"),
					ariaLabel: t("interfaces:workbench.dock.showRun", "Show run {{n}}", {
						n: first.n,
					}),
					run: () => actions.selectRun(first.runId, "show"),
				},
			],
		});
	},
	question: (line, context) => {
		const { names, kind } = line.question;
		const labels = names.map(
			(name) =>
				context.state.form.fields.find((field) => field.name === name)?.label ??
				name,
		);
		const joined = joinLabels(context.t, context.copy.list, labels);
		return view("question", {
			tone: "ink",
			strong: true,
			text:
				kind === "offer"
					? offerText(context.t, joined)
					: seriesEndText(context.t, joined),
			actions: yesNo(context),
		});
	},
	queue: (line, context) =>
		view("queue", {
			tone: "info",
			icon: "spinner",
			strong: true,
			text:
				line.queued > 0
					? context.t(
							"interfaces:workbench.dock.queue",
							"{{running}} running · {{queued}} queued",
							{ running: line.running, queued: line.queued },
						)
					: context.t(
							"interfaces:workbench.dock.queueRunning",
							"{{running}} running",
							{ running: line.running },
						),
			actions: line.queued > 0 ? [clearQueueAction(context)] : [],
		}),
	blocked: (line, { t }) =>
		view("blocked", {
			tone: "critical",
			icon: "alert",
			strong: true,
			text: t(
				"interfaces:workbench.dock.blocked",
				"{{label}} can't be sent from this page.",
				{ label: line.label },
			),
		}),
	compared: (line, { t }) =>
		line.changes > 0
			? view("compared", {
					tone: "ink",
					icon: "bar",
					strong: true,
					text: t(
						"interfaces:workbench.dock.changes",
						"{{count}} changes since run {{n}}",
						{
							count: line.changes,
							n: line.n,
							defaultValue_one: "1 change since run {{n}}",
						},
					),
				})
			: view("compared", {
					tone: "muted",
					text: t(
						"interfaces:workbench.dock.sameAsRun",
						"Same inputs as run {{n}}",
						{ n: line.n },
					),
				}),
	missing: (line, { t }) =>
		view("missing", {
			tone: "muted",
			text: t(
				"interfaces:workbench.dock.missing",
				"{{count}} fields to fill in",
				{ count: line.count, defaultValue_one: "1 field to fill in" },
			),
			steps: {
				prev: t(
					"interfaces:workbench.dock.missingPrev",
					"Previous field to fill in",
				),
				next: t(
					"interfaces:workbench.dock.missingNext",
					"Next field to fill in",
				),
			},
		}),
	ready: (_line, { t }) =>
		view("ready", {
			tone: "muted",
			text: t("interfaces:workbench.dock.ready", "Ready"),
		}),
	capped: (line, { t }) =>
		view("capped", {
			tone: "ink",
			strong: true,
			text: capSentence(t, line.running),
		}),
};

type AnyBuilder = (line: PhoneLine, context: LineContext) => LineView;

/** The line as drawn: its words, tone, icon and buttons. */
export const lineViewOf = (line: PhoneLine, context: LineContext) =>
	(LINE_VIEW[line.kind] as AnyBuilder)(line, context);
