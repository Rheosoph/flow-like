"use client";

import { useTranslation } from "@flow-like/locales";
import { DatabaseIcon, RefreshCwIcon, TableIcon } from "lucide-react";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../../hooks";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import { Button } from "../../../ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuTrigger,
} from "../../../ui/context-menu";
import type { IEditorScope } from "../editor-documents";
import { EmptyRow, SectionHeader, TreeRow } from "./explorer-primitives";

/**
 * The app's Data Studio tables, in both scopes.
 *
 * Read-only on purpose. Dropping a table changes the node catalog the open board is
 * rendering from — overlays contribute nodes — so a destructive verb here would
 * invalidate the graph behind the user's back. Managing tables stays in Data Studio.
 */
export function TablesRoot({
	appId,
	onOpenTable,
	enabled = true,
}: Readonly<{
	appId: string;
	onOpenTable: (scope: IEditorScope, table: string) => void;
	enabled?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	// The website ships an empty backend whose `listTables` throws; the board is embedded
	// there in lessons, so the query has to stay unfired rather than fail.
	const ready = useBackendReady() && enabled && appId.length > 0;

	const appTables = useInvoke(
		backend.dbState.listTables,
		backend.dbState,
		[appId],
		ready,
	);
	const userTables = useInvoke(
		backend.dbState.listTablesUser,
		backend.dbState,
		[appId],
		ready,
	);

	const rows = useMemo(() => {
		const entries: { scope: IEditorScope; table: string }[] = [];
		for (const table of appTables.data ?? [])
			entries.push({ scope: "app", table });
		for (const table of userTables.data ?? [])
			entries.push({ scope: "user", table });
		return entries;
	}, [appTables.data, userTables.data]);

	const refresh = useCallback(() => {
		void appTables.refetch();
		void userTables.refetch();
	}, [appTables, userTables]);

	const loading = appTables.isLoading || userTables.isLoading;
	const fetching = appTables.isFetching || userTables.isFetching;

	return (
		<>
			<SectionHeader
				label={t("tables", "Tables")}
				action={
					<Button
						size="icon"
						variant="ghost"
						className="size-5 text-muted-foreground"
						title={t("refresh", "Refresh")}
						aria-label={t("refresh", "Refresh")}
						onClick={refresh}
					>
						<RefreshCwIcon
							className={fetching ? "size-3 animate-spin" : "size-3"}
						/>
					</Button>
				}
			/>
			{!loading && rows.length === 0 && (
				<EmptyRow label={t("noTablesYet", "No tables yet")} />
			)}
			{rows.map(({ scope, table }) => (
				<ContextMenu key={`${scope}:${table}`}>
					<ContextMenuTrigger asChild>
						<TreeRow
							depth={0}
							icon={scope === "user" ? <DatabaseIcon /> : <TableIcon />}
							label={
								scope === "user"
									? t("tableNameUserScoped", "{{table}} (user)", { table })
									: table
							}
							muted={scope === "user"}
							onSelect={() => onOpenTable(scope, table)}
						/>
					</ContextMenuTrigger>
					<ContextMenuContent className="w-56">
						<ContextMenuItem onSelect={() => onOpenTable(scope, table)}>
							<TableIcon className="size-3.5" />
							{t("inspectTable", "Inspect table")}
						</ContextMenuItem>
					</ContextMenuContent>
				</ContextMenu>
			))}
		</>
	);
}
