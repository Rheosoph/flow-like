"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy } from "lucide-react";
import { toast } from "sonner";
import type { QueryColumn } from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import { type ColumnKind, cellToString, isNullish } from "./column-types";
import { ResultDetailValue } from "./result-value";

export interface InspectedResultCell {
	column: QueryColumn;
	kind: ColumnKind;
	value: unknown;
}

export function CellInspectorDialog({
	cell,
	appId,
	onOpenChange,
}: Readonly<{
	cell: InspectedResultCell | null;
	appId?: string;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<Dialog open={cell !== null} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[85vh] flex-col overflow-hidden sm:max-w-4xl">
				<DialogHeader className="shrink-0 pr-6">
					<DialogTitle className="break-all">
						{t("previewName", "Preview {{name}}", { name: cell?.column.name })}
					</DialogTitle>
					<DialogDescription>
						{cell?.column.type_name || cell?.kind} ·{" "}
						{t("readOnly", "Read only")}
					</DialogDescription>
				</DialogHeader>
				{cell && (
					<div className="min-h-0 min-w-0 overflow-auto py-1">
						<ResultDetailValue
							value={cell.value}
							kind={cell.kind}
							name={cell.column.name}
							metadata={cell.column.metadata}
							appId={appId}
						/>
					</div>
				)}
				{cell &&
					!isNullish(cell.value) &&
					typeof cell.value !== "object" &&
					(typeof cell.value !== "string" ||
						cell.kind === "number" ||
						cell.kind === "boolean" ||
						cell.kind === "temporal" ||
						cell.kind === "user" ||
						cell.kind === "file") && (
						<Button
							variant="outline"
							size="sm"
							className="w-fit shrink-0 gap-1.5"
							onClick={async () => {
								try {
									await navigator.clipboard.writeText(cellToString(cell.value));
									toast.success(t("copiedCell", "Copied cell"));
								} catch {
									toast.error(t("copyFailed", "Could not copy to clipboard"));
								}
							}}
						>
							<Copy className="size-3.5" />
							{t("copyCellValue", "Copy cell value")}
						</Button>
					)}
			</DialogContent>
		</Dialog>
	);
}
