"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronDown,
	CircleDashed,
	CircleQuestionMark,
	CircleStop,
	History,
	OctagonX,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { ProgressBar } from "../../../../settings/devices/primitives/meter";
import {
	TONE_BG,
	TONE_LINE,
	TONE_SURFACE,
	TONE_TEXT,
	cx,
} from "../../../../settings/devices/primitives/tone";
import {
	type InterfacesT,
	SHOWS_MESSAGE,
	failureTitle,
	queuedNoteText,
} from "../copy";
import type { NoteView } from "../notes-model";

type Note<K extends NoteView["kind"]> = Extract<NoteView, { kind: K }>;

/** A soft box in the unknown tone with a solid edge: queued, stopped, unknown result, an older run's receipt. */
function NoteBox({
	icon,
	children,
}: Readonly<{ icon: ReactNode; children: ReactNode }>) {
	return (
		<div
			className={cx(
				"flex items-start gap-3 rounded-lg border px-4 py-3",
				TONE_BG.unknown,
				TONE_LINE.unknown,
			)}
		>
			<span className={cx("mt-0.5 shrink-0", TONE_TEXT.unknown)}>{icon}</span>
			<div className="min-w-0 text-[13px]/[19px] text-ink-2">{children}</div>
		</div>
	);
}

function stoppedWords(t: InterfacesT, note: Note<"stopped">) {
	if (note.detached)
		return {
			lead: t(
				"interfaces:workbench.stage.note.stopped.detached",
				"You stopped waiting for this run.",
			),
			rest: t(
				"interfaces:workbench.stage.note.stopped.detachedRest",
				"It may still finish on the server, but this page will not show it.",
			),
		};
	const lead =
		note.step === null
			? t(
					"interfaces:workbench.stage.note.stopped.lead",
					"You stopped this run.",
				)
			: t(
					"interfaces:workbench.stage.note.stopped.leadStep",
					"You stopped this run during step {{number}}.",
					{ number: note.step },
				);
	if (note.kept)
		return {
			lead,
			rest: t(
				"interfaces:workbench.stage.note.stopped.kept",
				"What came back before that is kept below.",
			),
		};
	return {
		lead,
		rest: note.hasFields
			? t(
					"interfaces:workbench.stage.note.stopped.again",
					"Nothing went wrong. Run it again with the same inputs, or change one and press Run.",
				)
			: t(
					"interfaces:workbench.stage.note.stopped.againNoInputs",
					"Nothing went wrong. Press Run again to start it again.",
				),
	};
}

function StoppedNote({ note }: Readonly<{ note: Note<"stopped"> }>) {
	const { t } = useTranslation("interfaces");
	const { lead, rest } = stoppedWords(t, note);
	return (
		<NoteBox icon={<CircleStop aria-hidden className="size-4" />}>
			<span className="font-semibold text-foreground">{lead}</span> {rest}
		</NoteBox>
	);
}

function QueuedNote({ note }: Readonly<{ note: Note<"queued"> }>) {
	const { t } = useTranslation("interfaces");
	return (
		<NoteBox icon={<CircleDashed aria-hidden className="size-4" />}>
			{queuedNoteText(t, note.why, note.cap)}
		</NoteBox>
	);
}

function UnknownNote({ note }: Readonly<{ note: Note<"unknown"> }>) {
	const { t } = useTranslation("interfaces");
	return (
		<NoteBox icon={<CircleQuestionMark aria-hidden className="size-4" />}>
			{note.lost
				? t(
						"workbench.stage.note.lost",
						"The connection ended before this run reported a result.",
					)
				: t(
						"workbench.stage.note.unknown",
						"The form was closed while this run was going, so how it ended is not known.",
					)}
		</NoteBox>
	);
}

function ReceiptNote({ note }: Readonly<{ note: Note<"receipt"> }>) {
	const { t } = useTranslation("interfaces");
	return (
		<NoteBox icon={<History aria-hidden className="size-4" />}>
			<p className="m-0 text-[13px]/[19px]">
				{t(
					"workbench.stage.note.receipt",
					"This run is from an earlier visit. Its answer, result and files were not kept on this device.",
				)}
			</p>
			{note.firstLine ? (
				<p className="m-0 mt-1 text-[13px]/[19px] text-muted-foreground">
					“{note.firstLine}”
				</p>
			) : null}
		</NoteBox>
	);
}

/** "Nothing has come back yet." with the sliding bar; after 10 s "Still running. This can take a while." */
export function PendingNote({
	slow,
	centered,
}: Readonly<{ slow: boolean; centered: boolean }>) {
	const { t } = useTranslation("interfaces");
	return (
		<div
			className={cx(
				"flex flex-col gap-2.5",
				centered ? "mt-4 w-full max-w-60 items-center" : "max-w-96",
			)}
		>
			<ProgressBar
				className="w-full"
				label={t("workbench.stage.pending.label", "Nothing has come back yet")}
			/>
			<span
				className={
					centered
						? "text-[13px]/[18px] text-muted-foreground"
						: "text-sm/5 text-ink-2"
				}
			>
				{t("workbench.stage.pending.text", "Nothing has come back yet.")}
			</span>
			{slow ? (
				<span className="text-[13px]/[19px] text-muted-foreground">
					{t(
						"workbench.stage.pending.slow",
						"Still running. This can take a while.",
					)}
				</span>
			) : null}
		</div>
	);
}

export function NothingNote({ centered }: Readonly<{ centered: boolean }>) {
	const { t } = useTranslation("interfaces");
	return (
		<p
			className={cx(
				"m-0 font-medium text-sm/5",
				centered && "mt-3.5 w-full border-hairline border-t pt-4",
			)}
		>
			{t("workbench.stage.nothing", "This run returned nothing.")}
		</p>
	);
}

function FailureCard({
	note,
	touch,
}: Readonly<{ note: Note<"failure">; touch: boolean }>) {
	const { t } = useTranslation("interfaces");
	const [open, setOpen] = useState(false);
	const detail = note.detail ?? note.message;
	const shown =
		SHOWS_MESSAGE.has(note.failure) && note.message ? note.message : null;
	return (
		<div
			className={cx(
				"flex flex-col gap-2.5 rounded-lg border px-4 py-3.5",
				TONE_SURFACE.critical,
			)}
		>
			<div className="flex items-start gap-3">
				<OctagonX
					aria-hidden
					className={cx("mt-0.5 size-4 shrink-0", TONE_TEXT.critical)}
				/>
				<div className="flex min-w-0 flex-1 flex-col gap-0.75">
					<span className="font-semibold text-sm/5">
						{failureTitle(t, note.failure, note.step, note.nothingSaved)}
					</span>
					{shown ? (
						<span className="text-[13px]/[19px] text-ink-2">{shown}</span>
					) : null}
					<span className="text-[13px]/[19px] text-ink-2">
						{note.hasFields
							? t(
									"workbench.stage.note.failure.tryAgain",
									"Try again with the same inputs, or change one and press Run.",
								)
							: t(
									"workbench.stage.note.failure.tryAgainNoInputs",
									"Press Run again to try again.",
								)}
					</span>
				</div>
				{detail ? (
					<button
						type="button"
						aria-expanded={open}
						onClick={() => setOpen((on) => !on)}
						className={cx(
							"inline-flex shrink-0 items-center gap-1 rounded-lg pr-1.5 pl-2 font-medium text-[12.5px] text-ink-2 outline-ring hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1",
							touch ? "h-11" : "h-7",
						)}
					>
						{t("workbench.stage.note.failure.details", "Details")}
						<ChevronDown
							aria-hidden
							className={cx("size-3.25", open && "rotate-180")}
						/>
					</button>
				) : null}
			</div>
			{open && detail ? (
				<div
					className={cx(
						"ml-7 flex flex-col gap-1.5 rounded-lg border bg-card px-3 py-2.5",
						"border-critical-line",
					)}
				>
					<code className="whitespace-pre-wrap font-mono text-[12.5px]/[18px] wrap-anywhere">
						{detail}
					</code>
					{note.runId ? (
						<span className="text-muted-foreground text-xs/4">
							{t("workbench.stage.note.failure.runId", "Run ID")}{" "}
							<span className="font-mono">{note.runId}</span>
						</span>
					) : null}
				</div>
			) : null}
		</div>
	);
}

/** One note of a run's body; the centred card draws "pending" and "nothing" itself. */
export function NoteItem({
	note,
	touch,
}: Readonly<{ note: NoteView; touch: boolean }>) {
	switch (note.kind) {
		case "failure":
			return <FailureCard note={note} touch={touch} />;
		case "stopped":
			return <StoppedNote note={note} />;
		case "pending":
			return <PendingNote slow={note.slow} centered={false} />;
		case "nothing":
			return <NothingNote centered={false} />;
		case "queued":
			return <QueuedNote note={note} />;
		case "unknown":
			return <UnknownNote note={note} />;
		default:
			return <ReceiptNote note={note} />;
	}
}
