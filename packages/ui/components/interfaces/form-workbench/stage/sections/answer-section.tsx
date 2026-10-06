"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleDashed } from "lucide-react";
import { type RefObject, memo, useLayoutEffect, useMemo, useRef } from "react";
import { cx } from "../../../../settings/devices/primitives/tone";
import { StreamingTextEditor } from "../../../../ui/streaming-text-editor";
import { TextEditor } from "../../../../ui/text-editor";
import { markNumericColumns } from "../answer-table";
import { tieAnswer } from "../answer-ties";
import { sectionLabel } from "../copy";
import { Section } from "../section-label";
import { TAIL_ATTR } from "../use-body-scroll";
import { CARET, CARET_MARKER, CELL_SIDES, RHYTHM, TABLE } from "./answer-style";

/** Numeric columns are marked after every render of the answer: streaming, lazy blocks, a viewer re-render. */
function useNumericColumns(ref: RefObject<HTMLDivElement | null>) {
	useLayoutEffect(() => {
		const root = ref.current;
		if (!root) return;
		markNumericColumns(root);
		const observer = new MutationObserver(() => markNumericColumns(root));
		observer.observe(root, {
			childList: true,
			subtree: true,
			characterData: true,
		});
		return () => observer.disconnect();
	}, [ref]);
}

/**
 * The answer, in the chat's reading surface (a serif column of at most 38 rem) with the canvas's rhythm and
 * tables. While the flow writes it, the streaming editor parses incrementally; once the run has ended the
 * settled editor takes over. An answer of a run that did not finish is marked "Incomplete". The follow
 * marker sits with the text, so it adds no gap to the section. The text is tied with no-break spaces for
 * display only (`tieAnswer`); the run keeps the answer as written.
 */
export const AnswerSection = memo(function AnswerSection({
	answer,
	live,
	incomplete,
	touch = false,
}: Readonly<{
	answer: string;
	live: boolean;
	incomplete: boolean;
	touch?: boolean;
}>) {
	const { t } = useTranslation("interfaces");
	const prose = useRef<HTMLDivElement>(null);
	const shown = useMemo(() => tieAnswer(answer), [answer]);
	useNumericColumns(prose);
	return (
		<Section
			id="answer"
			label={sectionLabel(t, "answer")}
			aside={
				incomplete ? (
					<span className="inline-flex h-5 items-center gap-1.25 rounded-lg border border-unknown-line bg-unknown-bg px-1.75 font-medium text-unknown text-xs">
						<CircleDashed aria-hidden className="size-3" />
						{t("workbench.stage.answer.incomplete", "Incomplete")}
					</span>
				) : null
			}
		>
			<div>
				<div
					ref={prose}
					data-fl-chat-prose
					className={cx(
						RHYTHM,
						TABLE,
						CELL_SIDES[touch ? "touch" : "fine"],
						live && CARET,
					)}
				>
					{live ? (
						<StreamingTextEditor content={shown} />
					) : (
						<TextEditor initialContent={shown} isMarkdown editable={false} />
					)}
				</div>
				{live ? (
					<div aria-hidden {...{ [TAIL_ATTR]: "" }} className={CARET_MARKER} />
				) : null}
			</div>
			{incomplete ? (
				<p className="m-0 max-w-152 border-border-strong border-t border-dashed pt-2.5 text-[12.5px]/[18px] text-muted-foreground">
					{t(
						"workbench.stage.answer.incompleteNote",
						"The run ended here. The rest of the answer never arrived.",
					)}
				</p>
			) : null}
		</Section>
	);
});
