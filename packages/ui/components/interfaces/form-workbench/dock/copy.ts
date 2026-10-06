/*
 * The dock's words (spec M1, M3, M5, M6, S1, S4, F). Pure helpers that take `t`; every key is a literal
 * `interfaces:workbench.dock.*` (PLAN §9), so the extractor keeps them. Plurals pass `count` and the singular default.
 */
import type { TFunction } from "i18next";
import {
	type AfterRunBack,
	type DockMessage,
	FORM_LIMITS,
	type FailureLine,
	type HoldInfo,
	type ShortWords,
	type StartMessage,
} from "../contracts";
import { formatBytes } from "../run/format";

export type InterfacesT = TFunction<"interfaces">;

/** "A, B and C" in the app's language (`Intl.ListFormat`). */
export type ListJoiner = (items: readonly string[]) => string;

export interface CopyContext {
	readonly mac: boolean;
	readonly decimalSign: "." | ",";
	readonly warnBytes: number;
	readonly list: ListJoiner;
}

const KEYS = {
	mac: { run: "⌘↵", runLeave: "⇧⌘↵", stop: "⌘.", shortcuts: "⌘/" },
	other: {
		run: "Ctrl+Enter",
		runLeave: "Ctrl+Shift+Enter",
		stop: "Ctrl+.",
		shortcuts: "Ctrl+/",
	},
} as const;

/** The key names the dock prints: ⌘ on macOS, Ctrl elsewhere. */
export const keysOf = (mac: boolean) => (mac ? KEYS.mac : KEYS.other);

/** English reads "A, B and C": `en-GB` has no comma before the last item, `en` has. */
const listLocaleOf = (language: string) =>
	language.toLowerCase() === "en" ? "en-GB" : language;

export function listJoinerOf(language: string): ListJoiner {
	let formatter: Intl.ListFormat;
	try {
		formatter = new Intl.ListFormat(listLocaleOf(language), {
			style: "long",
			type: "conjunction",
		});
	} catch {
		formatter = new Intl.ListFormat("en-GB", {
			style: "long",
			type: "conjunction",
		});
	}
	return (items) => formatter.format(items);
}

/** `flpJoin`: "A", "A and B", "A, B and C", "A, B and 2 more" past `max`. */
export function joinLabels(
	t: InterfacesT,
	list: ListJoiner,
	labels: readonly string[],
	max = 3,
) {
	if (labels.length <= max) return list(labels);
	const more = t("interfaces:workbench.dock.more", "{{count}} more", {
		count: labels.length - max + 1,
	});
	return list([...labels.slice(0, max - 1), more]);
}

/** A size limit as the repo words it: MB, floored to one decimal ("1.4 MB", "3.5 MB", "35 MB"). */
export function limitText(bytes: number, decimalSign: "." | ",") {
	const mb = Math.floor((bytes / 1_048_576) * 10) / 10;
	const text = Number.isInteger(mb) ? String(mb) : mb.toFixed(1);
	return `${decimalSign === "," ? text.replace(".", ",") : text} MB`;
}

// ─── Run controls ───────────────────────────────────────────────────────────

export const runLabel = (t: InterfacesT) =>
	t("interfaces:workbench.dock.run", "Run");

export const runAgainLabel = (t: InterfacesT) =>
	t("interfaces:workbench.dock.runAgain", "Run again");

/** "Run · ⌘↵", with the ⇧⌘↵ sentence while anything is per run. */
export function runTitle(
	t: InterfacesT,
	label: string,
	mac: boolean,
	anyPerRun: boolean,
) {
	const keys = keysOf(mac);
	if (!anyPerRun)
		return t("interfaces:workbench.dock.runTitle", "{{label}} · {{keys}}", {
			label,
			keys: keys.run,
		});
	return t(
		"interfaces:workbench.dock.runTitlePerRun",
		"{{label}} · {{keys}}. Run and leave the inputs as they are · {{asIs}}",
		{ label, keys: keys.run, asIs: keys.runLeave },
	);
}

export const capButton = (t: InterfacesT) =>
	t("interfaces:workbench.dock.capButton", "Wait for a run to end");

/** `flpQuickCapText`: the sentence for the button's title, the phone's status line and the live region. */
export const capSentence = (t: InterfacesT, running: number) =>
	t(
		"interfaces:workbench.dock.capSentence",
		"{{count}} runs are going. You can run again when one ends.",
		{
			count: running,
			defaultValue_one:
				"{{count}} run is going. You can run again when it ends.",
		},
	);

export const stopLabel = (t: InterfacesT) =>
	t("interfaces:workbench.dock.stop", "Stop");

export const stopAria = (t: InterfacesT, n: number) =>
	t("interfaces:workbench.dock.stopRun", "Stop run {{n}}", { n });

export const stopTitle = (t: InterfacesT, n: number, mac: boolean) =>
	t("interfaces:workbench.dock.stopTitle", "Stop run {{n}} · {{keys}}", {
		n,
		keys: keysOf(mac).stop,
	});

// ─── Failure and hold ───────────────────────────────────────────────────────

/** `flpFailText`: "Run 18 failed at step 2: Run OCR." */
export function failureText(t: InterfacesT, failure: FailureLine) {
	const { n, step } = failure;
	if (!step)
		return t("interfaces:workbench.dock.failed", "Run {{n}} failed.", { n });
	if (!step.title)
		return t(
			"interfaces:workbench.dock.failedAtStep",
			"Run {{n}} failed at step {{step}}.",
			{ n, step: step.number },
		);
	return t(
		"interfaces:workbench.dock.failedAtStepTitle",
		"Run {{n}} failed at step {{step}}: {{title}}.",
		{ n, step: step.number, title: step.title },
	);
}

/** `flpHoldText`: "Runs 18 and 19 failed at step 2: Run OCR. The queue is on hold." */
export function holdText(t: InterfacesT, hold: HoldInfo) {
	const [first, second] = hold.runs;
	const { number, title } = hold.step;
	if (!title)
		return t(
			"interfaces:workbench.dock.hold",
			"Runs {{first}} and {{second}} failed at step {{step}}. The queue is on hold.",
			{ first, second, step: number },
		);
	return t(
		"interfaces:workbench.dock.holdTitle",
		"Runs {{first}} and {{second}} failed at step {{step}}: {{title}}. The queue is on hold.",
		{ first, second, step: number, title },
	);
}

// ─── Messages ───────────────────────────────────────────────────────────────

const START_LEAD: Readonly<
	Record<StartMessage["state"], (t: InterfacesT, start: StartMessage) => string>
> = {
	started: (t, start) =>
		t("interfaces:workbench.dock.startStarted", "Run {{n}} started", {
			n: start.n,
		}),
	queued: (t, start) =>
		t("interfaces:workbench.dock.startQueued", "Run {{n}} is queued", {
			n: start.n,
		}),
	sending: (t, start) =>
		t(
			"interfaces:workbench.dock.startSending",
			"Run {{n}} starts when its files are sent",
			{
				n: start.n,
				count: start.files,
				defaultValue_one: "Run {{n}} starts when its file is sent",
			},
		),
};

/** `flpStartMessage`: "Run 15 started: invoice-RE-2026-0918.pdf, 18 Sep 2026." */
function startText(t: InterfacesT, start: StartMessage) {
	const lead = START_LEAD[start.state](t, start);
	if (start.leftAsIs)
		return t(
			"interfaces:workbench.dock.startLeftAsIs",
			"{{lead}}. Inputs left as they are this time.",
			{ lead },
		);
	if (start.pairs.length === 0)
		return t("interfaces:workbench.dock.startPlain", "{{lead}}.", { lead });
	const text = t(
		"interfaces:workbench.dock.startPairs",
		"{{lead}}: {{pairs}}.",
		{
			lead,
			pairs: start.pairs.join(", "),
		},
	);
	if (!start.lastFile) return text;
	const last = t(
		"interfaces:workbench.dock.startLastFile",
		"That was the last file.",
	);
	return `${text} ${last}`;
}

function leftOutText(
	t: InterfacesT,
	message: Extract<DockMessage, { kind: "leftOut" }>,
) {
	const [first] = message.files;
	if (message.files.length === 1 && first)
		return t(
			"interfaces:workbench.dock.leftOutOne",
			"{{name}} was already sent in run {{n}} and was left out.",
			{ name: first.slot.name, n: first.n },
		);
	return t(
		"interfaces:workbench.dock.leftOutMany",
		"{{count}} files were already sent in earlier runs and were left out.",
		{
			count: message.files.length,
			defaultValue_one:
				"{{count}} file was already sent in an earlier run and was left out.",
		},
	);
}

function inputsInText(
	t: InterfacesT,
	message: Extract<DockMessage, { kind: "inputsIn" }>,
	context: CopyContext,
) {
	const sentences = [
		t("interfaces:workbench.dock.inputsIn", "Run {{n}}'s inputs are in.", {
			n: message.n,
		}),
	];
	if (message.pickAgain > 0)
		sentences.push(
			t(
				"interfaces:workbench.dock.inputsInPick",
				"Pick {{count}} files again.",
				{
					count: message.pickAgain,
					defaultValue_one: "Pick 1 file again.",
				},
			),
		);
	if (message.enterAgain.length > 0)
		sentences.push(
			t("interfaces:workbench.dock.inputsInEnter", "Enter {{labels}} again.", {
				labels: joinLabels(t, context.list, message.enterAgain),
			}),
		);
	if (message.misfit > 0)
		sentences.push(
			t(
				"interfaces:workbench.dock.inputsInMisfit",
				"{{count}} inputs no longer fit this form.",
				{
					count: message.misfit,
					defaultValue_one: "1 input no longer fits this form.",
				},
			),
		);
	return sentences.join(" ");
}

function clearedText(
	t: InterfacesT,
	message: Extract<DockMessage, { kind: "queueCleared" }>,
) {
	const runs =
		message.runs > 0
			? t(
					"interfaces:workbench.dock.clearedRuns",
					"{{count}} runs did not start",
					{ count: message.runs, defaultValue_one: "1 run did not start" },
				)
			: "";
	const files =
		message.files > 0
			? t(
					"interfaces:workbench.dock.clearedFiles",
					"{{count}} next files were removed",
					{
						count: message.files,
						defaultValue_one: "1 next file was removed",
					},
				)
			: "";
	if (runs && files)
		return t(
			"interfaces:workbench.dock.clearedBoth",
			"Queue cleared. {{runs}} and {{files}}.",
			{ runs, files },
		);
	if (!runs && !files)
		return t("interfaces:workbench.dock.cleared", "Queue cleared.");
	return t(
		"interfaces:workbench.dock.clearedPart",
		"Queue cleared. {{part}}.",
		{
			part: runs || files,
		},
	);
}

function presetAppliedText(
	t: InterfacesT,
	message: Extract<DockMessage, { kind: "presetApplied" }>,
) {
	if (message.misfit === 0)
		return t("interfaces:workbench.dock.presetApplied", "{{name}} applied.", {
			name: message.name,
		});
	return t(
		"interfaces:workbench.dock.presetAppliedMisfit",
		"{{name}} applied. {{count}} saved inputs no longer fit this form.",
		{
			name: message.name,
			count: message.misfit,
			defaultValue_one:
				"{{name}} applied. 1 saved input no longer fits this form.",
		},
	);
}

function fileRefusedText(
	t: InterfacesT,
	message: Extract<DockMessage, { kind: "fileRefused" }>,
	context: CopyContext,
) {
	const values = {
		name: message.name,
		limit: limitText(message.limitBytes, context.decimalSign),
	};
	if (message.host === "hosted")
		return t(
			"interfaces:workbench.dock.fileRefusedLink",
			"{{name}} is larger than {{limit}}, the most this link can send. It was not added.",
			values,
		);
	return t(
		"interfaces:workbench.dock.fileRefusedPage",
		"{{name}} is larger than {{limit}}, the most this device page accepts. It was not added.",
		values,
	);
}

type MessageKind = DockMessage["kind"];
type MessageText<K extends MessageKind> = (
	t: InterfacesT,
	message: Extract<DockMessage, { kind: K }>,
	context: CopyContext,
) => string;

const MESSAGE_TEXT: { readonly [K in MessageKind]: MessageText<K> } = {
	start: (t, message) => startText(t, message.start),
	sameInputs: (t, message, context) =>
		t(
			"interfaces:workbench.dock.sameInputs",
			"Same inputs as run {{n}}. {{keys}} runs it again.",
			{ n: message.n, keys: keysOf(context.mac).run },
		),
	perRunNow: (t, message, context) =>
		t("interfaces:workbench.dock.perRunNow", "{{labels}} are now per run.", {
			labels: joinLabels(t, context.list, message.labels),
			count: message.labels.length,
			defaultValue_one: "{{labels}} is now per run.",
		}),
	fieldReset: (t, message) =>
		t("interfaces:workbench.dock.fieldReset", "{{label}} reset.", {
			label: message.label,
		}),
	fileRemoved: (t, message) =>
		t("interfaces:workbench.dock.fileRemoved", "{{name}} removed.", {
			name: message.name,
		}),
	leftOut: leftOutText,
	presetApplied: presetAppliedText,
	presetUpdated: (t, message) =>
		t("interfaces:workbench.dock.presetUpdated", "{{name}} updated.", {
			name: message.name,
		}),
	presetSaved: (t, message) =>
		t("interfaces:workbench.dock.presetSaved", "Saved as {{name}}.", {
			name: message.name,
		}),
	presetDeleted: (t, message) =>
		t("interfaces:workbench.dock.presetDeleted", "{{name}} deleted.", {
			name: message.name,
		}),
	resetTo: (t, message) =>
		message.presetName === null
			? t(
					"interfaces:workbench.dock.resetDefaults",
					"Inputs reset to their defaults.",
				)
			: t("interfaces:workbench.dock.resetTo", "Inputs reset to {{name}}.", {
					name: message.presetName,
				}),
	inputsIn: inputsInText,
	noSave: (t, message) =>
		t(
			"interfaces:workbench.dock.noSave",
			"{{label}} is no longer saved on this device. Earlier runs no longer show it.",
			{ label: message.label },
		),
	nextFilesRemoved: (t, message) =>
		t(
			"interfaces:workbench.dock.nextFilesRemoved",
			"{{count}} next files removed.",
			{ count: message.count, defaultValue_one: "1 next file removed." },
		),
	nextFilesCapped: (t) =>
		t(
			"interfaces:workbench.dock.nextFilesCapped",
			"Only the first {{max}} files were added. A field takes up to {{max}} at a time.",
			{ max: FORM_LIMITS.nextFiles },
		),
	queueCleared: clearedText,
	fileRefused: fileRefusedText,
	fileWarning: (t, message, context) =>
		t(
			"interfaces:workbench.dock.fileWarning",
			"{{name}} is larger than {{limit}}. Sending it may fail.",
			{
				name: message.name,
				limit: limitText(context.warnBytes, context.decimalSign),
			},
		),
	oneFileOnly: (t, message) =>
		t(
			"interfaces:workbench.dock.oneFileOnly",
			"Only {{name}} was added. This page sends one file per run.",
			{ name: message.name },
		),
	requestTooLarge: (t, message, context) =>
		t(
			"interfaces:workbench.dock.requestTooLarge",
			"These files are too large to send together from this page ({{total}} of {{limit}}).",
			{
				total: formatBytes(message.totalBytes, context.decimalSign),
				limit: limitText(message.limitBytes, context.decimalSign),
			},
		),
	answerFailed: (t, message) =>
		t(
			"interfaces:workbench.dock.answerFailed",
			"Your answer to run {{n}} could not be sent. Try again.",
			{ n: message.n },
		),
};

type AnyMessageText = (
	t: InterfacesT,
	message: DockMessage,
	context: CopyContext,
) => string;

/** The sentence of one dock message. */
export const messageText = (
	t: InterfacesT,
	message: DockMessage,
	context: CopyContext,
) => (MESSAGE_TEXT[message.kind] as AnyMessageText)(t, message, context);

// ─── After-run line ─────────────────────────────────────────────────────────

/** `flpAfterRunSummary`: "No input is per run", "1 input is per run", "3 inputs are per run". */
export const afterRunSummary = (t: InterfacesT, count: number) =>
	count === 0
		? t("interfaces:workbench.dock.afterRunNone", "No input is per run")
		: t("interfaces:workbench.dock.afterRun", "{{count}} inputs are per run", {
				count,
				defaultValue_one: "1 input is per run",
			});

export const afterRunChange = (t: InterfacesT) =>
	t("interfaces:workbench.dock.afterRunChange", "Change");

export const afterRunChangeLabel = (t: InterfacesT) =>
	t(
		"interfaces:workbench.dock.afterRunChangeLabel",
		"Change which inputs are per run",
	);

export const shortcutsLabel = (t: InterfacesT, mac: boolean) =>
	t("interfaces:workbench.dock.shortcuts", "Keyboard shortcuts ({{keys}})", {
		keys: keysOf(mac).shortcuts,
	});

// ─── Questions, buttons, panel ──────────────────────────────────────────────

export const offerText = (t: InterfacesT, labels: string) =>
	t("interfaces:workbench.dock.offer", "Make {{labels}} per run?", { labels });

export const seriesEndText = (t: InterfacesT, labels: string) =>
	t("interfaces:workbench.dock.seriesEnd", "Should {{labels}} stay per run?", {
		labels,
	});

const BACK_TEXT: {
	readonly [K in AfterRunBack["kind"]]: (
		t: InterfacesT,
		back: Extract<AfterRunBack, { kind: K }>,
		words: ShortWords,
	) => string;
} = {
	nextFile: (t) => t("interfaces:workbench.dock.nextFile", "Next file"),
	empty: (t) => t("interfaces:workbench.dock.backEmpty", "Empty"),
	on: (t, _back, words) =>
		t("interfaces:workbench.dock.back", "Back to {{value}}", {
			value: words.on,
		}),
	off: (t, _back, words) =>
		t("interfaces:workbench.dock.back", "Back to {{value}}", {
			value: words.off,
		}),
	objectDefault: (t) =>
		t("interfaces:workbench.dock.backObject", "Back to its starting value"),
	value: (t, back) =>
		t("interfaces:workbench.dock.back", "Back to {{value}}", {
			value: back.text,
		}),
};

type AnyBackText = (
	t: InterfacesT,
	back: AfterRunBack,
	words: ShortWords,
) => string;

/** The right side of a "Per run" row: what the field goes back to. */
export const backText = (
	t: InterfacesT,
	back: AfterRunBack,
	words: ShortWords,
) => (BACK_TEXT[back.kind] as AnyBackText)(t, back, words);
