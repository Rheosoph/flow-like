import type { TFunction } from "i18next";
import type { NotStartedReason, RunEntry } from "../contracts";
import { diffValues } from "../model/values";
import { formatClock, formatTook, withNumberText } from "../run/format";
import { elapsedMs } from "../run/run-view";
import {
	type ChangeContext,
	type RunChanges,
	type RunPreview,
	type RunWait,
	ordinalSuffix,
} from "./rail-model";

/*
 * The words of a Runs list row: time and duration, the place in line, the third line. Module-level
 * helpers take `t` and spell the namespace in every key (PLAN §9); one literal `t()` per phrase.
 */

type InterfacesT = TFunction<"interfaces">;
type Words<K extends string> = Readonly<Record<K, (t: InterfacesT) => string>>;

const timeFormats = new Map<string, Intl.DateTimeFormat>();

/** What a run's inputs say against the run pressed before it, number inputs in the viewer's reading. */
export function runChangesOf(
	run: RunEntry,
	older: RunEntry | undefined,
	context: ChangeContext,
	number: (text: string) => string,
): RunChanges {
	if (!older) return { kind: "first" };
	const { fields } = context;
	const items = withNumberText(
		fields,
		diffValues(fields, older.copy.values, run.copy.values, context),
		number,
	);
	return items.length === 0
		? { kind: "same", n: older.n }
		: { kind: "diff", items };
}

/**
 * "invoice-RE-2026-0917.pdf" and "PO-48213" stay in one piece: a hyphen between letters or digits becomes a
 * non-breaking hyphen (U+2011), for display only (canvas `wbKeepIds`).
 */
export const keepIds = (text: string) =>
	text.replace(/([A-Za-z0-9])-(?=[A-Za-z0-9])/g, "$1\u{2011}");

/** "14:02" in the viewer's habits. */
export function timeOf(at: number, locale: string) {
	let format = timeFormats.get(locale);
	if (!format) {
		try {
			format = new Intl.DateTimeFormat(locale, { timeStyle: "short" });
		} catch {
			format = new Intl.DateTimeFormat("en", { timeStyle: "short" });
		}
		timeFormats.set(locale, format);
	}
	return format.format(at);
}

const IN_LINE: Readonly<
	Record<"st" | "nd" | "rd" | "th", (t: InterfacesT, n: number) => string>
> = {
	st: (t, n) =>
		t("interfaces:workbench.rail.runs.inLine.st", "{{n}}st in line", { n }),
	nd: (t, n) =>
		t("interfaces:workbench.rail.runs.inLine.nd", "{{n}}nd in line", { n }),
	rd: (t, n) =>
		t("interfaces:workbench.rail.runs.inLine.rd", "{{n}}rd in line", { n }),
	th: (t, n) =>
		t("interfaces:workbench.rail.runs.inLine.th", "{{n}}th in line", { n }),
};

export function waitText(t: InterfacesT, wait: RunWait) {
	if (wait.kind === "noPlace")
		return t(
			"interfaces:workbench.rail.runs.noPlace",
			"waiting for a free place",
		);
	return IN_LINE[ordinalSuffix(wait.place)](t, wait.place);
}

const CLOCK_RUNNING = new Set<RunEntry["status"]>([
	"running",
	"streaming",
	"asking",
]);
const ENDED = new Set<RunEntry["status"]>([
	"done",
	"empty",
	"failed",
	"stopped",
]);

/** A run whose clock is ticking: the row redraws once a second while one is listed. */
export const hasLiveClock = (run: Pick<RunEntry, "status">) =>
	CLOCK_RUNNING.has(run.status);

/** The duration part of the time column: the live clock, how long it took, or nothing. */
export function durationText(run: RunEntry, now: number): string | null {
	if (hasLiveClock(run)) return formatClock(elapsedMs(run, now));
	if (ENDED.has(run.status) && run.startedAt !== null)
		return formatTook(elapsedMs(run, now));
	return null;
}

const NOT_STARTED: Words<NotStartedReason> = {
	removedFromQueue: (t) =>
		t(
			"interfaces:workbench.rail.runs.notStarted.removedFromQueue",
			"Taken out of the queue.",
		),
	formClosed: (t) =>
		t(
			"interfaces:workbench.rail.runs.notStarted.formClosed",
			"The form was closed before its turn.",
		),
	fileNotSent: (t) =>
		t(
			"interfaces:workbench.rail.runs.notStarted.fileNotSent",
			"Its file was not sent.",
		),
	declined: (t) =>
		t(
			"interfaces:workbench.rail.runs.notStarted.declined",
			"It was not approved.",
		),
	promptCancelled: (t) =>
		t(
			"interfaces:workbench.rail.runs.notStarted.promptCancelled",
			"A question before the run was cancelled.",
		),
};

type PreviewWords = {
	readonly [K in RunPreview["kind"]]: (
		t: InterfacesT,
		preview: Extract<RunPreview, { kind: K }>,
	) => string;
};

const PREVIEW: PreviewWords = {
	text: (_t, preview) => preview.text,
	failedAt: (t, { step }) =>
		t("interfaces:workbench.rail.runs.failedAt", "Failed at “{{title}}”.", {
			title: step.title,
		}),
	stoppedAt: (t, { step }) =>
		t("interfaces:workbench.rail.runs.stoppedAt", "Stopped at “{{title}}”.", {
			title: step.title,
		}),
	notStarted: (t, { reason }) => NOT_STARTED[reason](t),
	unknown: (t) =>
		t(
			"interfaces:workbench.rail.runs.unknown",
			"This run was going when the page closed.",
		),
	nothing: (t) =>
		t("interfaces:workbench.rail.runs.nothing", "This run returned nothing."),
};

/** A run of this session whose stream ended without a result; "unknown" otherwise means the page closed. */
const lostText = (t: InterfacesT) =>
	t(
		"interfaces:workbench.rail.runs.lost",
		"The connection ended before this run reported a result.",
	);

export function previewText(
	t: InterfacesT,
	preview: RunPreview,
	origin: RunEntry["origin"] = "history",
) {
	if (preview.kind === "unknown" && origin === "session") return lostText(t);
	const words = PREVIEW[preview.kind] as (
		t: InterfacesT,
		preview: RunPreview,
	) => string;
	return words(t, preview);
}
