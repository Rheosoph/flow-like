/*
 * The words of the run bar, the strip and the notes, one literal `t()` call per phrase so the
 * extractor keeps every key (PLAN §9). The models hand over descriptors; nothing here is data-built.
 */
import type { TFunction } from "i18next";
import type { FailureKind, NotStartedReason, StepRef } from "../contracts";
import { formatClock, formatTook } from "../run/format";
import type { RepeatBlocker } from "../run/run-view";
import { statusWord } from "../status-look";
import type { MenuItem } from "./bar-actions";
import type { BarDetail, BarLead, BarView } from "./bar-model";
import type { QueuedWhy } from "./notes-model";
import type { SectionKey } from "./pane-model";

export type InterfacesT = TFunction<"interfaces">;

// ─── Run bar ────────────────────────────────────────────────────────────────

const doneIn = (t: InterfacesT, ms: number) =>
	t("interfaces:workbench.stage.bar.doneIn", "Done in {{time}}", {
		time: formatTook(ms),
	});

const failedAfter = (t: InterfacesT, ms: number) =>
	t("interfaces:workbench.stage.bar.failedAfter", "Failed after {{time}}", {
		time: formatTook(ms),
	});

const stoppedAt = (t: InterfacesT, ms: number) =>
	t("interfaces:workbench.stage.bar.stoppedAt", "Stopped at {{time}}", {
		time: formatClock(ms),
	});

function leadOf(t: InterfacesT, lead: BarLead, view: BarView) {
	switch (lead.kind) {
		case "doneIn":
			return doneIn(t, lead.ms);
		case "failedAfter":
			return failedAfter(t, lead.ms);
		case "stoppedAt":
			return stoppedAt(t, lead.ms);
		default:
			return statusWord(t, view.status);
	}
}

/** "Running", "Done in 48 s", "Failed after 12 s", "Stopped at 0:37", "Queued", "Not started". */
export const leadText = (t: InterfacesT, view: BarView) =>
	leadOf(t, view.lead, view);

const REASON: Readonly<Record<NotStartedReason, (t: InterfacesT) => string>> = {
	removedFromQueue: (t) =>
		t(
			"interfaces:workbench.stage.bar.reason.removedFromQueue",
			"taken out of the queue",
		),
	formClosed: (t) =>
		t(
			"interfaces:workbench.stage.bar.reason.formClosed",
			"the form was closed before its turn",
		),
	fileNotSent: (t) =>
		t(
			"interfaces:workbench.stage.bar.reason.fileNotSent",
			"its file was not sent",
		),
	declined: (t) =>
		t("interfaces:workbench.stage.bar.reason.declined", "it was not approved"),
	promptCancelled: (t) =>
		t(
			"interfaces:workbench.stage.bar.reason.promptCancelled",
			"its settings were not filled in",
		),
};

export const stepText = (t: InterfacesT, step: StepRef) =>
	t("interfaces:workbench.stage.bar.step", "Step {{number}}: {{title}}", {
		number: step.number,
		title: step.title,
	});

/** "1st in line": an ordinal plural, so every language spells its own. */
export const inLineText = (t: InterfacesT, place: number) =>
	t("interfaces:workbench.stage.bar.inLine", "{{count}}th in line", {
		count: place,
		ordinal: true,
		defaultValue_ordinal_one: "{{count}}st in line",
		defaultValue_ordinal_two: "{{count}}nd in line",
		defaultValue_ordinal_few: "{{count}}rd in line",
		defaultValue_ordinal_other: "{{count}}th in line",
	});

const sendingSoon = (t: InterfacesT, files: number) =>
	t(
		"interfaces:workbench.stage.bar.sendingSoon",
		"starts when its files are sent",
		{
			count: files,
			defaultValue_one: "starts when its file is sent",
			defaultValue_other: "starts when its files are sent",
		},
	);

const waitingForPlace = (t: InterfacesT) =>
	t(
		"interfaces:workbench.stage.bar.waitingForPlace",
		"waiting for a free place",
	);

function detailOf(t: InterfacesT, detail: BarDetail) {
	switch (detail.kind) {
		case "step":
			return stepText(t, detail.step);
		case "inLine":
			return inLineText(t, detail.place);
		case "sendingSoon":
			return sendingSoon(t, detail.files);
		case "notStarted":
			return REASON[detail.reason](t);
		default:
			return waitingForPlace(t);
	}
}

/** The part after the lead: "Step 4: Match purchase order", "1st in line", "its file was not sent". */
export const detailText = (t: InterfacesT, detail: BarDetail) =>
	detailOf(t, detail);

// ─── Notes ──────────────────────────────────────────────────────────────────

const QUEUED_NOTE: Readonly<
	Record<QueuedWhy, (t: InterfacesT, cap: number) => string>
> = {
	noPlace: (t) =>
		t(
			"interfaces:workbench.stage.note.queued.noPlace",
			"Every place your plan has is taken, also by runs in other windows or on other devices. This one starts by itself when a place is free.",
		),
	hold: (t) =>
		t(
			"interfaces:workbench.stage.note.queued.hold",
			"The queue is on hold after two runs failed at the same step. This one starts when you resume the queue.",
		),
	device: (t, cap) =>
		t(
			"interfaces:workbench.stage.note.queued.device",
			"This device runs {{cap}} at a time from this window. This one starts by itself when one of them ends.",
			{ cap },
		),
	plan: (t, cap) =>
		t(
			"interfaces:workbench.stage.note.queued.plan",
			"Your plan runs {{cap}} at a time. This one starts by itself when one of them ends.",
			{ cap },
		),
	soon: (t) =>
		t(
			"interfaces:workbench.stage.note.queued.soon",
			"This one starts by itself as soon as a place is free.",
		),
};

export const queuedNoteText = (t: InterfacesT, why: QueuedWhy, cap: number) =>
	QUEUED_NOTE[why](t, cap);

const flowFailure = (t: InterfacesT, step: StepRef | null) =>
	step
		? t(
				"interfaces:workbench.stage.note.failure.flowAt",
				"The run failed at “{{step}}”.",
				{ step: step.title },
			)
		: t("interfaces:workbench.stage.note.failure.flow", "The run failed.");

/** The reassurance for a flow that failed before any file came back: files are all the form knows about. */
const flowFailureNothingSaved = (t: InterfacesT, step: StepRef | null) =>
	step
		? t(
				"interfaces:workbench.stage.note.failure.flowAtNothingSaved",
				"The run failed at “{{step}}”. No files were saved.",
				{ step: step.title },
			)
		: t(
				"interfaces:workbench.stage.note.failure.flowNothingSaved",
				"The run failed. No files were saved.",
			);

const FAILURE_TITLE: Readonly<
	Record<
		FailureKind,
		(t: InterfacesT, step: StepRef | null, nothingSaved: boolean) => string
	>
> = {
	flow: (t, step, nothingSaved) =>
		nothingSaved ? flowFailureNothingSaved(t, step) : flowFailure(t, step),
	other: (t) =>
		t("interfaces:workbench.stage.note.failure.other", "The run failed."),
	upload: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.upload",
			"A file could not be sent.",
		),
	permission: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.permission",
			"You do not have permission to run this.",
		),
	quota: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.quota",
			"Your plan's limit was reached, so this run could not start.",
		),
	oauth: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.oauth",
			"A sign-in this app needs is missing or has expired.",
		),
	network: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.network",
			"The connection was lost before the run could finish.",
		),
	timeout: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.timeout",
			"The run took too long and was ended.",
		),
	rejected: (t) =>
		t(
			"interfaces:workbench.stage.note.failure.rejected",
			"The app did not accept this run.",
		),
};

/**
 * One plain sentence for a failure; the flow's own error text stays under Details. A flow that
 * failed before any file came back adds "No files were saved."
 */
export const failureTitle = (
	t: InterfacesT,
	failure: FailureKind,
	step: StepRef | null,
	nothingSaved = false,
) => FAILURE_TITLE[failure](t, step, nothingSaved);

/** Failures whose message is meant for the person (the server's words); a flow's error text is not. */
export const SHOWS_MESSAGE: ReadonlySet<FailureKind> = new Set<FailureKind>([
	"permission",
	"quota",
	"oauth",
	"network",
	"timeout",
	"rejected",
	"upload",
]);

// ─── Sections ───────────────────────────────────────────────────────────────

const SECTION_LABEL: Readonly<Record<SectionKey, (t: InterfacesT) => string>> =
	{
		steps: (t) => t("interfaces:workbench.stage.section.steps", "Steps"),
		answer: (t) => t("interfaces:workbench.stage.section.answer", "Answer"),
		result: (t) => t("interfaces:workbench.stage.section.result", "Result"),
		files: (t) => t("interfaces:workbench.stage.section.files", "Files"),
		inputs: (t) => t("interfaces:workbench.stage.section.inputs", "Inputs"),
	};

export const sectionLabel = (t: InterfacesT, key: SectionKey) =>
	SECTION_LABEL[key](t);

// ─── Menu ───────────────────────────────────────────────────────────────────

const repeatLabel = (t: InterfacesT, item: MenuItem) =>
	item.repeat === "tryAgain"
		? t("interfaces:workbench.stage.action.tryAgain", "Try again")
		: t("interfaces:workbench.stage.action.runAgain", "Run again");

const copyLabel = (t: InterfacesT, item: MenuItem) =>
	item.copy === "answer"
		? t("interfaces:workbench.stage.menu.copyAnswer", "Copy the answer")
		: t("interfaces:workbench.stage.menu.copyResult", "Copy the result");

const MENU_LABEL: Readonly<
	Record<MenuItem["id"], (t: InterfacesT, item: MenuItem) => string>
> = {
	pin: (t) => t("interfaces:workbench.stage.menu.pin", "Pin to compare"),
	unpin: (t) => t("interfaces:workbench.stage.menu.unpin", "Unpin"),
	repeat: repeatLabel,
	use: (t) => t("interfaces:workbench.stage.action.use", "Use these inputs"),
	savePreset: (t) =>
		t(
			"interfaces:workbench.stage.menu.savePreset",
			"Save these inputs as a preset…",
		),
	copy: copyLabel,
	remove: (t) =>
		t("interfaces:workbench.stage.menu.remove", "Remove from this device"),
};

export const menuLabel = (t: InterfacesT, item: MenuItem) =>
	MENU_LABEL[item.id](t, item);

const BLOCKER_TEXT: Readonly<
	Record<RepeatBlocker, (t: InterfacesT) => string>
> = {
	files: (t) =>
		t(
			"interfaces:workbench.stage.menu.blockedFiles",
			"Its files can no longer be sent. Use these inputs and pick them again.",
		),
	hidden: (t) =>
		t(
			"interfaces:workbench.stage.menu.blockedHidden",
			"Some of its inputs are not saved on this device. Use these inputs and enter them again.",
		),
	form: (t) =>
		t(
			"interfaces:workbench.stage.menu.blockedForm",
			"Some of its inputs no longer fit this form. Use these inputs and check them.",
		),
};

/** Why a repeat item is disabled (S4). */
export const blockerText = (t: InterfacesT, blocker: RepeatBlocker) =>
	BLOCKER_TEXT[blocker](t);
