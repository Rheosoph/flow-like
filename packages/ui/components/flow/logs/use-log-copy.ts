import { useTranslation } from "@flow-like/locales";
import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogMetadata } from "../../../lib/schema/flow/log-metadata";
import type { ILogQuery } from "../../../lib/schema/flow/log-query";
import type { IBoardState } from "../../../state/backend-state/board-state";
import {
	COPY_LIMIT,
	type ICopyEntry,
	formatJson,
	formatMarkdown,
	formatText,
} from "./copy-format";
import { PAGE_SIZE } from "./log-page-cache";

export type ICopyFormat = "text" | "json" | "markdown";

function render(
	entries: readonly ICopyEntry[],
	format: ICopyFormat,
	title?: string,
) {
	if (format === "json") return formatJson(entries);
	if (format === "markdown") return formatMarkdown(entries, title);
	return formatText(entries);
}

export function useLogCopy({
	boardState,
	meta,
	entryOf,
	title,
}: {
	boardState: IBoardState;
	meta: ILogMetadata;
	entryOf: (log: ILog) => ICopyEntry;
	title?: string;
}) {
	const { t } = useTranslation("flow");
	const [copying, setCopying] = useState(false);
	const latest = useRef({ boardState, meta, entryOf, title });
	latest.current = { boardState, meta, entryOf, title };

	const fail = useCallback(
		(error: unknown) =>
			toast.error(
				t("logViewCopyFailed", "Couldn't copy the logs: {{error}}", {
					error: error instanceof Error ? error.message : String(error),
				}),
			),
		[t],
	);

	const write = useCallback(
		async (logs: readonly ILog[], format: ICopyFormat, total = logs.length) => {
			const { entryOf, title } = latest.current;
			try {
				await navigator.clipboard.writeText(
					render(logs.map(entryOf), format, title),
				);
				toast.success(
					logs.length < total
						? t(
								"logViewCopiedFirst",
								"Copied the first {{count, number}} of {{total, number}} logs",
								{ count: logs.length, total },
							)
						: t("logViewCopied", "Copied {{count, number}} logs", {
								count: logs.length,
							}),
				);
			} catch (error) {
				fail(error);
			}
		},
		[t, fail],
	);

	const copyLogs = useCallback(
		(logs: readonly ILog[], format: ICopyFormat) => write(logs, format),
		[write],
	);

	const copyText = useCallback(
		async (text: string) => {
			try {
				await navigator.clipboard.writeText(text);
				toast.success(t("logViewCopiedSelection", "Copied the selected text"));
			} catch (error) {
				fail(error);
			}
		},
		[t, fail],
	);

	const copyVisible = useCallback(
		async (query: ILogQuery | undefined, count: number) => {
			if (!query || count <= 0) return;
			const { boardState, meta } = latest.current;
			const limit = Math.min(count, COPY_LIMIT);
			setCopying(true);
			try {
				const rows: ILog[] = [];
				for (let offset = 0; offset < limit; offset += PAGE_SIZE) {
					const page = await boardState.queryRunLogs(
						meta,
						query,
						offset,
						Math.min(PAGE_SIZE, limit - offset),
					);
					rows.push(...page);
					if (page.length === 0) break;
				}
				await write(rows, "text", count);
			} catch (error) {
				fail(error);
			} finally {
				setCopying(false);
			}
		},
		[write, fail],
	);

	return { copying, copyLogs, copyText, copyVisible };
}
