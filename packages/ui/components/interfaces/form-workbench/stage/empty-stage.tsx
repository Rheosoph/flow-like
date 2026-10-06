"use client";

import { useTranslation } from "@flow-like/locales";
import type { FormModel } from "../contracts";

/** What the empty stage says: press Run, the first run waits for its files, or it waits for a place. */
export type EmptyHint = "press" | "sending" | "queued";

/**
 * Before the first run of a form with inputs (BRIEF-A "Three fields"): on a wide box the description (else
 * "Nothing has run yet") and what to do; on a phone's Output pane the short form. A form without inputs gets
 * its first-run card from the shell, not from here.
 */
export function EmptyStage({
	form,
	narrow,
	hint,
}: Readonly<{
	form: Pick<FormModel, "description">;
	narrow: boolean;
	hint: EmptyHint;
}>) {
	const { t } = useTranslation("interfaces");
	const title = t("workbench.stage.empty.title", "Nothing has run yet");
	const hints: Readonly<Record<EmptyHint, string>> = {
		press: narrow
			? t(
					"workbench.stage.empty.hintShort",
					"Fill in the inputs and press Run.",
				)
			: t(
					"workbench.stage.empty.hint",
					"Fill in the inputs and press Run. Each run opens here in its own tab.",
				),
		sending: t(
			"workbench.stage.empty.hintSending",
			"Your first run opens here once the files are sent.",
		),
		queued: t(
			"workbench.stage.empty.hintQueued",
			"Your first run opens here as soon as it starts.",
		),
	};
	if (narrow) {
		return (
			<div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-1 px-4 py-8 text-center">
				<p className="m-0 font-semibold text-[15px]/[22px]">{title}</p>
				<p className="m-0 text-balance text-muted-foreground text-sm/5">
					{hints[hint]}
				</p>
			</div>
		);
	}
	return (
		<div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-8 py-8">
			<div className="m-auto w-full max-w-120 pb-14">
				<div className="flex flex-col items-center gap-2.5 text-center">
					{form.description ? (
						<p className="m-0 text-balance text-[15px]/6 text-ink-2">
							{form.description}
						</p>
					) : (
						<p className="m-0 font-semibold text-[15px]/[22px]">{title}</p>
					)}
					<p className="mt-2 w-full max-w-88 text-balance border-hairline border-t pt-3.5 text-[13.5px]/5 text-muted-foreground">
						{hints[hint]}
					</p>
				</div>
			</div>
		</div>
	);
}
