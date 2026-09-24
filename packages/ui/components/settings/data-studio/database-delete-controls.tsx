"use client";

import { useQueryClient } from "@tanstack/react-query";
import { Loader2, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../../lib/error-message";
import { useBackend } from "../../../state/backend-state";
import type { IDatabaseSelector } from "../../../state/backend-state/db-state";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { DropdownMenuItem } from "../../ui/dropdown-menu";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { Textarea } from "../../ui/textarea";
import { isDatabaseSnapshot } from "./database-reference";

export type DatabaseDeleteOperation = "rows" | "table";

export function databaseDeleteOperations(
	selector: IDatabaseSelector,
	canDropTable: boolean,
): DatabaseDeleteOperation[] {
	return [
		...(isDatabaseSnapshot(selector) ? [] : (["rows"] as const)),
		...(canDropTable ? (["table"] as const) : []),
	];
}

export function DatabaseDeleteMenuItems({
	operations,
	onSelect,
}: {
	operations: readonly DatabaseDeleteOperation[];
	onSelect: (operation: DatabaseDeleteOperation) => void;
}) {
	return operations.map((operation) => (
		<DropdownMenuItem
			key={operation}
			variant="destructive"
			onSelect={() => onSelect(operation)}
		>
			<Trash2 />
			{operation === "rows" ? "Delete rows…" : "Delete table…"}
		</DropdownMenuItem>
	));
}

export function DatabaseDeleteDialog({
	operation: requested,
	onOperationChange,
	appId,
	table,
	userScoped,
	selector,
	onChanged,
	onTableDeleted,
}: {
	operation: DatabaseDeleteOperation | null;
	onOperationChange: (operation: DatabaseDeleteOperation | null) => void;
	appId: string;
	table: string;
	userScoped?: boolean;
	selector: IDatabaseSelector;
	onChanged: () => void;
	onTableDeleted?: () => void;
}) {
	const backend = useBackend();
	const cache = useQueryClient();
	const [lastRequested, setLastRequested] = useState(requested);
	// Keeps the closing dialog's copy stable while it animates out.
	const [operation, setOperation] = useState(requested);
	const [filter, setFilter] = useState("");
	const [confirmation, setConfirmation] = useState("");
	const [preview, setPreview] = useState<{
		filter: string;
		rows: unknown[];
	} | null>(null);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	if (requested !== lastRequested) {
		setLastRequested(requested);
		if (requested) {
			setOperation(requested);
			setFilter("");
			setConfirmation("");
			setPreview(null);
			setError(null);
		}
	}
	const snapshot = isDatabaseSnapshot(selector);
	const branch = selector.branch ?? "main";
	const previewRows = async () => {
		if (!filter.trim() || snapshot || busy) return;
		setBusy(true);
		setError(null);
		try {
			const rows = await backend.dbState.queryItems(
				appId,
				table,
				{ filter: filter.trim() },
				0,
				11,
				userScoped,
				selector,
			);
			setPreview({ filter: filter.trim(), rows });
		} catch (err) {
			setPreview(null);
			setError(getErrorMessage(err));
		} finally {
			setBusy(false);
		}
	};
	const execute = async () => {
		if (busy || confirmation !== table || !operation) return;
		if (
			operation === "rows" &&
			(snapshot ||
				!preview ||
				preview.filter !== filter.trim() ||
				!preview.rows.length)
		)
			return;
		setBusy(true);
		setError(null);
		try {
			if (operation === "rows") {
				await backend.dbState.removeItems(
					appId,
					table,
					filter.trim(),
					userScoped,
					selector,
				);
				toast.success(`Deleted matching rows from ${table} on ${branch}`);
			} else {
				const result = await backend.dbState.dropTable(
					appId,
					table,
					userScoped,
				);
				const details = [
					result.ontologies.length
						? `Updated ontologies: ${result.ontologies.join(", ")}.`
						: "",
					result.saved_queries.length
						? `Saved queries still reference this table: ${result.saved_queries.join(", ")}.`
						: "",
					...result.warnings,
				]
					.filter(Boolean)
					.join(" ");
				toast.success(`Deleted table ${table}`, {
					description: details || undefined,
				});
			}
			onOperationChange(null);
			if (operation === "table") onTableDeleted?.();
			// Leave a dropped table before its old queries are refreshed.
			// A table drop also changes source listings and generated table nodes.
			await cache.invalidateQueries({
				predicate: (query) => query.queryKey[1] === appId,
			});
			if (operation === "rows") onChanged();
		} catch (err) {
			setError(getErrorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	return (
		<Dialog
			open={requested !== null}
			onOpenChange={(value) => {
				if (!value && !busy) onOperationChange(null);
			}}
		>
			<DialogContent className="max-h-[85vh] overflow-y-auto">
				<DialogHeader>
					<DialogTitle>
						{operation === "table"
							? `Delete table ${table}?`
							: `Delete rows from ${table}`}
					</DialogTitle>
					<DialogDescription>
						{operation === "table"
							? `Permanently delete the entire ${userScoped ? "user" : "project"} table, including every branch, version, tag, row and index. Ontology mappings will be updated; saved queries may stop working. This cannot be undone.`
							: `Delete every row matching your filter from the ${branch} branch of this ${userScoped ? "user" : "project"} table. Other branches and retained historical versions are unchanged.`}
					</DialogDescription>
				</DialogHeader>
				{operation === "rows" && (
					<div className="space-y-3">
						<div className="space-y-2">
							<Label htmlFor="database-delete-filter">
								Row filter (SQL WHERE expression)
							</Label>
							<Textarea
								id="database-delete-filter"
								value={filter}
								disabled={busy}
								onChange={(event) => {
									setFilter(event.target.value);
									setPreview(null);
									setConfirmation("");
								}}
								placeholder="customer_id = 'customer-123'"
							/>
							<p className="text-xs text-muted-foreground">
								Use an exact identifier to delete one row, or a condition for a
								group of rows. The filter is applied to all matching rows when
								you confirm.
							</p>
						</div>
						<Button
							variant="outline"
							disabled={busy || !filter.trim()}
							onClick={() => void previewRows()}
						>
							Preview matching rows
						</Button>
						{preview && (
							<div className="space-y-2">
								<p className="text-sm">
									{preview.rows.length === 0
										? "No matching rows."
										: preview.rows.length > 10
											? "Showing the first 10 matches. All matches will be deleted."
											: `${preview.rows.length} matching row${preview.rows.length === 1 ? "" : "s"} in this preview.`}
								</p>
								{preview.rows.length > 0 && (
									<pre className="max-h-48 overflow-auto rounded-md bg-muted p-3 text-xs">
										{JSON.stringify(preview.rows.slice(0, 10), null, 2)}
									</pre>
								)}
								<p className="text-xs text-muted-foreground">
									Rows may change after the preview. Save a tagged snapshot from
									History & branches if you need to retain the current data.
								</p>
							</div>
						)}
					</div>
				)}
				<div className="space-y-2">
					<Label htmlFor="database-delete-confirmation">
						Type {table} to confirm
					</Label>
					<Input
						id="database-delete-confirmation"
						value={confirmation}
						disabled={busy}
						onChange={(event) => setConfirmation(event.target.value)}
						autoComplete="off"
					/>
				</div>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<DialogFooter>
					<Button
						variant="outline"
						disabled={busy}
						onClick={() => onOperationChange(null)}
					>
						Cancel
					</Button>
					<Button
						variant="destructive"
						disabled={
							busy ||
							confirmation !== table ||
							(operation === "rows" &&
								(!preview?.rows.length || preview.filter !== filter.trim()))
						}
						onClick={() => void execute()}
					>
						{busy && <Loader2 className="h-4 w-4 animate-spin" />}
						{operation === "table"
							? "Delete entire table"
							: "Delete matching rows"}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
