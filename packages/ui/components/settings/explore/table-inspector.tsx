"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangle, Database, RefreshCw } from "lucide-react";
import type React from "react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { cn } from "../../../lib";
import { getErrorMessage } from "../../../lib/error-message";
import { useBackend } from "../../../state/backend-state";
import { IIndexType } from "../../../state/backend-state/db-state";
import { Button } from "../../ui/button";
import LanceDBExplorer from "../../ui/lance-viewer";

export const DEFAULT_TABLE_PAGE_SIZE = 25;

const INDEX_TYPES: Record<string, IIndexType> = {
	fulltext: IIndexType.FullText,
	btree: IIndexType.BTree,
	bitmap: IIndexType.Bitmap,
	labellist: IIndexType.LabelList,
	auto: IIndexType.Auto,
};

export interface TableInspectorProps {
	appId: string;
	table: string;
	userScoped?: boolean;
	/** Controlled page (1-based). Falls back to internal state when omitted. */
	page?: number;
	pageSize?: number;
	onPageChange?: (page: number, pageSize: number) => void;
	className?: string;
	/** Hosted inside another chrome: drop the fullscreen escape hatch. */
	embedded?: boolean;
	/** Extra header actions, rendered next to the explorer toolbar. */
	children?: React.ReactNode;
}

export function TableInspector({
	appId,
	table,
	userScoped,
	page,
	pageSize,
	onPageChange,
	className,
	embedded,
	children,
}: Readonly<TableInspectorProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();

	const [internalPage, setInternalPage] = useState(page ?? 1);
	const [internalPageSize, setInternalPageSize] = useState(
		pageSize ?? DEFAULT_TABLE_PAGE_SIZE,
	);

	const scopeKey = `${appId}:${table}:${userScoped ? "user" : "app"}`;
	const [lastScopeKey, setLastScopeKey] = useState(scopeKey);
	if (lastScopeKey !== scopeKey) {
		// A host that swaps tables under one instance would otherwise carry the
		// previous table's page number into an offset the new table may not have.
		setLastScopeKey(scopeKey);
		setInternalPage(page ?? 1);
	}

	const activePage = Math.max(1, page ?? internalPage);
	const activePageSize = pageSize ?? internalPageSize;
	const offset = (activePage - 1) * activePageSize;

	// Backends without a database (the website's empty state) throw on every
	// call, so nothing may be requested before a table is actually selected.
	const enabled = Boolean(appId) && Boolean(table);

	const schema = useInvoke(
		backend.dbState.getSchema,
		backend.dbState,
		[appId, table, userScoped],
		enabled,
	);
	const count = useInvoke(
		backend.dbState.countItems,
		backend.dbState,
		[appId, table, userScoped],
		enabled,
	);
	const list = useInvoke(
		backend.dbState.listItems,
		backend.dbState,
		[appId, table, offset, activePageSize, userScoped],
		enabled,
	);

	const handlePageRequest = useCallback(
		(args: { page: number; pageSize: number }) => {
			setInternalPage(args.page);
			setInternalPageSize(args.pageSize);
			onPageChange?.(args.page, args.pageSize);
		},
		[onPageChange],
	);

	const handleRefresh = useCallback(() => {
		schema.refetch();
		count.refetch();
		list.refetch();
	}, [schema.refetch, count.refetch, list.refetch]);

	const handleOptimize = useCallback(
		async (keepVersions = true) => {
			try {
				await backend.dbState.optimize(appId, table, keepVersions, userScoped);
				toast.success(t("optimizedTable", "Optimized table"));
				handleRefresh();
			} catch (err) {
				toast.error(
					t("optimizeFailedMessage", "Optimize failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	const handleUpdateItem = useCallback(
		async (filter: string, updates: Record<string, unknown>) => {
			try {
				await backend.dbState.updateItem(
					appId,
					table,
					filter,
					updates,
					userScoped,
				);
				handleRefresh();
			} catch (err) {
				toast.error(
					t("updateFailedMessage", "Update failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	const handleDropColumns = useCallback(
		async (columns: string[]) => {
			try {
				await backend.dbState.dropColumns(appId, table, columns, userScoped);
				toast.success(
					t("droppedColumns", {
						defaultValue_one: "Dropped column",
						defaultValue_other: "Dropped columns",
						count: columns.length,
					}),
				);
				handleRefresh();
			} catch (err) {
				toast.error(
					t("dropColumnFailedMessage", "Drop column failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	const handleAddColumn = useCallback(
		async (name: string, sqlExpression: string) => {
			try {
				await backend.dbState.addColumn(
					appId,
					table,
					{ name, sql_expression: sqlExpression },
					userScoped,
				);
				toast.success(
					t("addedColumnName", 'Added column "{{name}}"', { name }),
				);
				handleRefresh();
			} catch (err) {
				toast.error(
					t("addColumnFailedMessage", "Add column failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	const handleAlterColumn = useCallback(
		async (column: string, nullable: boolean) => {
			try {
				await backend.dbState.alterColumn(
					appId,
					table,
					column,
					nullable,
					userScoped,
				);
				toast.success(
					t("alteredColumnName", 'Altered column "{{name}}"', { name: column }),
				);
				handleRefresh();
			} catch (err) {
				toast.error(
					t("alterColumnFailedMessage", "Alter column failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	const handleGetIndices = useCallback(
		async () => backend.dbState.getIndices(appId, table, userScoped),
		[backend.dbState, appId, table, userScoped],
	);

	const handleDropIndex = useCallback(
		async (indexName: string) => {
			try {
				await backend.dbState.dropIndex(appId, table, indexName, userScoped);
				toast.success(
					t("droppedIndexName", 'Dropped index "{{name}}"', {
						name: indexName,
					}),
				);
				handleRefresh();
			} catch (err) {
				toast.error(
					t("dropIndexFailedMessage", "Drop index failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	const handleBuildIndex = useCallback(
		async (column: string, indexType: string) => {
			try {
				await backend.dbState.buildIndex(
					appId,
					table,
					column,
					INDEX_TYPES[indexType.toLowerCase()] ?? IIndexType.Auto,
					undefined,
					userScoped,
				);
				toast.success(
					t("builtIndexOnColumn", 'Built index on "{{column}}"', { column }),
				);
				handleRefresh();
			} catch (err) {
				toast.error(
					t("buildIndexFailedMessage", "Build index failed: {{message}}", {
						message: getErrorMessage(err),
					}),
				);
				throw err;
			}
		},
		[backend.dbState, appId, table, userScoped, handleRefresh, t],
	);

	// Column layouts are stored per table; without the app they collide across
	// every project that happens to use the same table name.
	const settingsScope = userScoped ? `${appId}:user` : appId;

	const containerCls = cn(
		"flex flex-col h-full grow max-h-full min-w-0",
		className,
	);

	if (!enabled) {
		return (
			<TableInspectorNotice
				className={containerCls}
				title={t("selectATable", "Select a table")}
				description={t(
					"chooseATableToInspectItsRowsSchemaAndIndexes",
					"Choose a table to inspect its rows, schema, and indexes.",
				)}
			>
				{children}
			</TableInspectorNotice>
		);
	}

	const loadError = schema.error ?? list.error;
	if (loadError && (!schema.data || !list.data)) {
		return (
			<TableInspectorNotice
				className={containerCls}
				destructive
				title={t("couldNotOpenTableName", 'Could not open "{{name}}"', {
					name: table,
				})}
				description={getErrorMessage(
					loadError,
					t("common:unknownError", "Unknown error"),
				)}
				onRetry={handleRefresh}
			>
				{children}
			</TableInspectorNotice>
		);
	}

	if (!schema.data || !list.data) {
		return (
			<TableInspectorSkeleton className={containerCls} tableName={table}>
				{children}
			</TableInspectorSkeleton>
		);
	}

	return (
		<div className={containerCls}>
			<LanceDBExplorer
				appId={appId}
				total={count.data}
				tableName={table}
				settingsScope={settingsScope}
				allowFullscreen={!embedded}
				arrowSchema={schema.data}
				rows={list.data}
				initialPage={activePage}
				initialPageSize={activePageSize}
				onPageRequest={handlePageRequest}
				loading={list.isLoading}
				error={list.error?.message}
				onRefresh={handleRefresh}
				onOptimize={handleOptimize}
				onUpdateItem={handleUpdateItem}
				onDropColumns={handleDropColumns}
				onAddColumn={handleAddColumn}
				onAlterColumn={handleAlterColumn}
				onGetIndices={handleGetIndices}
				onDropIndex={handleDropIndex}
				onBuildIndex={handleBuildIndex}
			>
				{children}
			</LanceDBExplorer>
		</div>
	);
}

const LOADING_COLUMN_KEYS = ["one", "two", "three", "four", "five"];
const LOADING_ROW_KEYS = [
	"one",
	"two",
	"three",
	"four",
	"five",
	"six",
	"seven",
	"eight",
];

const TableInspectorHeader: React.FC<{
	tableName?: string;
	children?: React.ReactNode;
}> = ({ tableName, children }) => (
	<div className="flex items-center gap-3">
		{children}
		{tableName && (
			<div className="flex items-center gap-2 min-w-0">
				<Database className="h-5 w-5 text-muted-foreground animate-pulse shrink-0" />
				<span className="text-sm font-medium truncate">{tableName}</span>
			</div>
		)}
	</div>
);

const TableInspectorSkeleton: React.FC<{
	tableName: string;
	className?: string;
	children?: React.ReactNode;
}> = ({ tableName, className, children }) => (
	<div className={cn(className, "p-4 gap-4")}>
		<TableInspectorHeader tableName={tableName}>
			{children}
		</TableInspectorHeader>
		<div className="flex items-center gap-2">
			<div className="h-9 w-24 bg-muted animate-pulse rounded" />
			<div className="h-9 flex-1 bg-muted animate-pulse rounded" />
			<div className="h-9 w-20 bg-muted animate-pulse rounded" />
		</div>
		<div className="flex-1 rounded border overflow-hidden">
			<div className="h-10 bg-muted/60 border-b flex items-center gap-4 px-4">
				{LOADING_COLUMN_KEYS.map((key, index) => (
					<div
						key={key}
						className="h-4 bg-muted animate-pulse rounded"
						style={{ width: `${60 + index * 20}px` }}
					/>
				))}
			</div>
			{LOADING_ROW_KEYS.map((rowKey, rowIndex) => (
				<div
					key={rowKey}
					className="h-10 border-b flex items-center gap-4 px-4"
				>
					{LOADING_COLUMN_KEYS.map((columnKey, columnIndex) => (
						<div
							key={`${rowKey}-${columnKey}`}
							className="h-3.5 bg-muted/50 animate-pulse rounded"
							style={{ width: `${40 + ((rowIndex + columnIndex) % 4) * 25}px` }}
						/>
					))}
				</div>
			))}
		</div>
		<div className="flex items-center justify-between shrink-0">
			<div className="h-4 w-32 bg-muted animate-pulse rounded" />
			<div className="flex gap-1">
				<div className="h-8 w-8 bg-muted animate-pulse rounded" />
				<div className="h-8 w-8 bg-muted animate-pulse rounded" />
			</div>
		</div>
	</div>
);

const TableInspectorNotice: React.FC<{
	title: string;
	description: string;
	className?: string;
	destructive?: boolean;
	onRetry?: () => void;
	children?: React.ReactNode;
}> = ({ title, description, className, destructive, onRetry, children }) => {
	const { t } = useTranslation("settings");
	return (
		<div className={cn(className, "p-4 gap-4")}>
			<TableInspectorHeader>{children}</TableInspectorHeader>
			<div className="flex flex-1 items-center justify-center">
				<div className="max-w-md rounded-lg border bg-card p-8 text-center">
					{destructive ? (
						<AlertTriangle className="mx-auto mb-4 h-10 w-10 text-destructive" />
					) : (
						<Database className="mx-auto mb-4 h-10 w-10 text-muted-foreground" />
					)}
					<h3 className="mb-2 text-lg font-semibold">{title}</h3>
					<p className="text-sm text-muted-foreground break-words">
						{description}
					</p>
					{onRetry && (
						<Button className="mt-5" variant="outline" onClick={onRetry}>
							<RefreshCw className="mr-2 h-4 w-4" />
							{t("retry", "Retry")}
						</Button>
					)}
				</div>
			</div>
		</div>
	);
};
