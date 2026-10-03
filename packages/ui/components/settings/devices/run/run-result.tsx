"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy } from "lucide-react";
import { formatCountdown } from "../../../../lib/date";
import type { EventRun } from "../../../../lib/device-management/event-run";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { InlineResult } from "../primitives/inline-result";
import { useCopy } from "../primitives/use-copy";
import { type RunNames, failureSentence } from "./run-copy";

/** How long a run took by the device's clock; undefined when a time is missing. */
export function runTook(run: EventRun): string | undefined {
	if (run.startedAt === undefined || run.finishedAt === undefined)
		return undefined;
	return formatCountdown(run.finishedAt - run.startedAt);
}

/** The output as text: a JSON value indented, a text as it came. */
export function outputText(run: EventRun): string | undefined {
	const output = run.output;
	if (!output) return undefined;
	return "json" in output
		? (JSON.stringify(output.json, null, 2) ?? "null")
		: output.text;
}

/** The outcome sentence of an ended run. */
export function outcomeSentence(
	t: DevicesT,
	run: EventRun,
	names: RunNames,
): string {
	if (run.run === "succeeded")
		return t("devices:runNow.result.succeeded", "The run succeeded.");
	const took = runTook(run);
	return failureSentence(t, run.code, names, {
		...(run.fields ? { fields: run.fields } : {}),
		...(took ? { took } : {}),
	});
}

function OutputBox({ text }: Readonly<{ text: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<div data-run-output="" className="flex min-w-0 flex-col gap-1.5">
			<div className="flex items-center gap-2">
				<span className="min-w-0 flex-1 text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
					{t("runNow.result.output", "Result")}
				</span>
				<DvButton
					size="xs"
					variant="ghost"
					icon={copied ? Check : Copy}
					onClick={() => void copy(text)}
				>
					{copied
						? t("runNow.result.copied", "Copied")
						: t("runNow.result.copy", "Copy")}
				</DvButton>
			</div>
			<pre className="max-h-64 min-w-0 overflow-auto rounded-lg border border-border bg-surface-sunken px-3 py-2 font-mono text-xs/5 whitespace-pre-wrap wrap-anywhere text-foreground">
				{text}
			</pre>
		</div>
	);
}

/** An ended run: the outcome, how long it took, and the output the device still has. */
export function RunResult({
	run,
	names,
}: Readonly<{ run: EventRun; names: RunNames }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const succeeded = run.run === "succeeded";
	const text = outputText(run);
	const took = runTook(run);
	const notes = [
		run.truncated
			? t(
					"runNow.result.truncated",
					"Shortened: the full result is larger than 8 KiB.",
				)
			: null,
		run.outputGone
			? t(
					"runNow.result.gone",
					"The result is no longer on {{device}} (its agent restarted).",
					names,
				)
			: null,
		succeeded && !text && !run.outputGone
			? t("runNow.result.none", "The run returned no result.")
			: null,
		run.attachments
			? t("runNow.result.attachments", {
					count: run.attachments,
					defaultValue_one:
						"It also made {{count, number}} file or image. It isn't shown here.",
					defaultValue_other:
						"It also made {{count, number}} files or images. They aren't shown here.",
				})
			: null,
	].filter((note): note is string => Boolean(note));
	return (
		<div data-run-result={run.run} className="flex min-w-0 flex-col gap-2.5">
			<InlineResult tone={succeeded ? "good" : "critical"}>
				{outcomeSentence(t, run, names)}
			</InlineResult>
			{took || run.finishedAt ? (
				<p className="text-xs text-muted-foreground">
					{took && run.finishedAt
						? t("runNow.result.took", "Took {{took}} · finished {{time}}", {
								took,
								time: time.at(run.finishedAt),
							})
						: run.finishedAt
							? t("runNow.result.finished", "Finished {{time}}", {
									time: time.at(run.finishedAt),
								})
							: null}
				</p>
			) : null}
			{text !== undefined ? <OutputBox text={text} /> : null}
			{notes.map((note) => (
				<p key={note} className="text-xs text-ink-2">
					{note}
				</p>
			))}
		</div>
	);
}
