import { useCallback } from "react";
import { useCopy } from "../../../settings/devices/primitives/use-copy";
import type { RunEntry } from "../contracts";
import { type ResultFormat, toResultModel } from "../run/result-view";
import type { CopyKind } from "./bar-actions";

/** What "Copy answer" / "Copy result" puts on the clipboard: the answer's text, or a result as text (its raw JSON for an object). */
export function copyTextOf(
	run: Pick<RunEntry, "output">,
	kind: CopyKind | null,
	format: ResultFormat,
) {
	const output = run.output;
	if (!output || kind === null) return "";
	if (kind === "answer") return output.answer;
	return output.result
		? toResultModel(output.result.value, format).copyText
		: "";
}

/** The copy action of one run's bar and menu; `copied` is true for a moment afterwards ("Copied"). */
export function useCopyRun(
	run: RunEntry,
	kind: CopyKind | null,
	format: ResultFormat,
) {
	const { copied, copy } = useCopy();
	const copyNow = useCallback(() => {
		const text = copyTextOf(run, kind, format);
		if (text !== "") void copy(text);
	}, [run, kind, format, copy]);
	return { copied, copyNow };
}
