"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronLeft, ChevronRight, Copy } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { cn } from "../../../../lib/utils";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import { classifyResultColumn } from "./column-types";
import { ResultDetailValue } from "./result-value";

export function QueryResultRecord({
	columns,
	rows,
	appId,
}: Readonly<{
	columns: QueryColumn[];
	rows: Record<string, unknown>[];
	appId?: string;
}>) {
	const { t } = useTranslation("settings");
	const [position, setPosition] = useState({ rows, index: 0 });
	const index =
		position.rows === rows ? Math.min(position.index, rows.length - 1) : 0;
	const row = rows[index];
	const scrollRef = useRef<HTMLDivElement>(null);
	const kinds = useMemo(
		() => columns.map((column) => classifyResultColumn(column, rows, appId)),
		[columns, rows, appId],
	);

	useEffect(() => {
		if (row && scrollRef.current) scrollRef.current.scrollTop = 0;
	}, [row]);

	const copyRow = async () => {
		try {
			await navigator.clipboard.writeText(JSON.stringify(row, null, 2));
			toast.success(t("copiedRowAsJson", "Copied row as JSON"));
		} catch {
			toast.error(
				t("couldNotCopyToClipboard", "Could not copy to the clipboard."),
			);
		}
	};

	if (!row) return null;

	return (
		<div className="@container/record flex h-full min-h-0 min-w-0 flex-col">
			<div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b bg-muted/10 px-3 py-2">
				<nav
					aria-label={t("browseResultRows", "Browse result rows")}
					className="flex min-w-0 items-center gap-1"
				>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						aria-label={t("previousRow", "Previous row")}
						disabled={index === 0}
						onClick={() => setPosition({ rows, index: index - 1 })}
					>
						<ChevronLeft className="h-4 w-4" />
					</Button>
					<output
						className="min-w-20 px-1 text-center text-xs tabular-nums text-muted-foreground"
						aria-live="polite"
					>
						{t("resultRecordPosition", "Row {{index}} of {{total}}", {
							index: index + 1,
							total: rows.length.toLocaleString(),
						})}
					</output>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						aria-label={t("nextRow", "Next row")}
						disabled={index >= rows.length - 1}
						onClick={() => setPosition({ rows, index: index + 1 })}
					>
						<ChevronRight className="h-4 w-4" />
					</Button>
				</nav>
				<Button
					variant="ghost"
					size="sm"
					className="h-7 gap-1.5 text-xs"
					onClick={() => void copyRow()}
				>
					<Copy className="h-3.5 w-3.5" />
					{t("copyRow", "Copy row")}
				</Button>
			</div>
			<div ref={scrollRef} className="min-h-0 flex-1 overflow-auto">
				<dl className="mx-auto w-full max-w-5xl divide-y px-4 @xl/record:px-6">
					{columns.map((column, columnIndex) => {
						const value = row[column.name];
						const kind = kinds[columnIndex];
						// Short scalar labels need no format toolbar. Rich text and structured
						// values use the same readers as the table's cell inspector.
						const plainText =
							kind === "text" &&
							typeof value === "string" &&
							value.length > 0 &&
							value.length <= 160 &&
							!/[\r\n]|(?:^|\s)(?:#{1,6}\s|[-+*]\s|_[^_]+_(?:\s|$)|\*[^*]+\*(?:\s|$))|\*\*|__|~~|`|\[[^\]]*\]\(|<[^>]+>|^[{\[]/.test(
								value,
							);
						const scalar =
							plainText ||
							value == null ||
							kind === "number" ||
							kind === "boolean" ||
							kind === "temporal";
						return (
							<div
								key={`${index}:${column.name}`}
								className={cn(
									"grid min-w-0 gap-2 @xl/record:grid-cols-[minmax(0,160px)_minmax(0,1fr)] @xl/record:gap-6",
									scalar
										? "grid-cols-[92px_minmax(0,1fr)] py-3"
										: "grid-cols-1 py-4",
								)}
							>
								<dt className="min-w-0 pt-0.5" title={column.type_name || kind}>
									<span className="block break-words font-mono text-xs font-medium">
										{column.name}
									</span>
								</dt>
								<dd className="min-w-0 break-words text-sm leading-relaxed [overflow-wrap:anywhere]">
									{plainText ? (
										<span
											className={
												/^(id|.*_id|uuid|key)$/i.test(column.name)
													? "font-mono text-xs"
													: undefined
											}
										>
											{value}
										</span>
									) : (
										<ResultDetailValue
											kind={kind}
											name={column.name}
											value={value}
											appId={appId}
											metadata={column.metadata}
										/>
									)}
								</dd>
							</div>
						);
					})}
				</dl>
			</div>
		</div>
	);
}
