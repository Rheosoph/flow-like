"use client";

import { useTranslation } from "@flow-like/locales";
import { Braces, Copy } from "lucide-react";
import { toast } from "sonner";
import { cn } from "../../../../lib/utils";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import { ScrollArea } from "../../../ui/scroll-area";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "../../../ui/sheet";
import {
	type ColumnKind,
	cellToString,
	classifyColumn,
	isNullish,
} from "./column-types";
import { ResultDetailValue } from "./result-value";

export function RowInspectorSheet({
	row,
	columns,
	appId,
	kindOf = classifyColumn,
	onOpenChange,
}: Readonly<{
	row: Record<string, unknown> | null;
	columns: QueryColumn[];
	appId?: string;
	/** The kind the table showed the column as, so the drawer reads its cells alike. */
	kindOf?: (column: QueryColumn) => ColumnKind;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("settings");
	const copyRow = () => {
		if (!row) return;
		void navigator.clipboard.writeText(JSON.stringify(row, null, 2));
		toast.success("Copied row as JSON");
	};

	return (
		<Sheet open={row !== null} onOpenChange={onOpenChange}>
			<SheetContent className="w-full gap-0 p-0 sm:max-w-md">
				<SheetHeader className="border-b">
					<SheetTitle>{t("rowDetails", "Row details")}</SheetTitle>
					<SheetDescription>
						{t("countColumns", {
							defaultValue_one: "{{count}} column",
							defaultValue_other: "{{count}} columns",
							count: columns.length,
						})}
					</SheetDescription>
					<Button
						variant="outline"
						size="sm"
						className="mt-1 w-fit gap-1.5"
						onClick={copyRow}
					>
						<Braces className="h-3.5 w-3.5" /> {t("copyAsJson", "Copy as JSON")}
					</Button>
				</SheetHeader>
				<ScrollArea className="h-[calc(100%-8rem)]">
					<dl className="divide-y">
						{row &&
							columns.map((column) => {
								const value = row[column.name];
								const kind = kindOf(column);
								return (
									<div
										key={column.name}
										className="group grid grid-cols-[1fr_auto] gap-2 px-4 py-2.5"
									>
										<div className="min-w-0">
											<dt className="flex items-center gap-1.5 font-mono text-xs font-medium">
												<span className="truncate">{column.name}</span>
												<span className="shrink-0 text-[10px] font-normal text-muted-foreground">
													{column.type_name || kind}
												</span>
											</dt>
											<dd
												className={cn(
													"mt-0.5 break-words font-mono text-sm",
													isNullish(value) && "italic text-muted-foreground/50",
													kind === "number" && "tabular-nums",
												)}
											>
												{isNullish(value) ? (
													"NULL"
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
										{!isNullish(value) && (
											<Button
												variant="ghost"
												size="icon"
												className="h-7 w-7 shrink-0 self-start text-muted-foreground opacity-0 focus-visible:opacity-100 group-hover:opacity-100"
												aria-label={t("copyName", "Copy {{name}}", {
													name: column.name,
												})}
												onClick={() => {
													void navigator.clipboard.writeText(
														cellToString(value),
													);
													toast.success(`Copied ${column.name}`);
												}}
											>
												<Copy className="h-3.5 w-3.5" />
											</Button>
										)}
									</div>
								);
							})}
					</dl>
				</ScrollArea>
			</SheetContent>
		</Sheet>
	);
}
