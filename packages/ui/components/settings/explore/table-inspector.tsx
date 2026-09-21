"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangle, Database, RefreshCw } from "lucide-react";
import type React from "react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { cn } from "../../../lib";
import { getErrorMessage } from "../../../lib/error-message";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { useBackend } from "../../../state/backend-state";
import {
	type IDatabaseSelector,
	parseIndexType,
} from "../../../state/backend-state/db-state";
import { Button } from "../../ui/button";
import LanceDBExplorer from "../../ui/lance-viewer";
import { DatabaseDeleteControls } from "../data-studio/database-delete-controls";
import { DatabaseHistoryControls } from "../data-studio/database-history-controls";
import {
	databaseSelectorKey,
	isDatabaseSnapshot,
} from "../data-studio/database-reference";
import { SectionLockedPanel } from "../permission/permission-gate";
import { PermissionNotice } from "../permission/permission-notice";

const READ_DATA = [RolePermissions.ReadFiles, RolePermissions.ReadDatabase];
const WRITE_DATA = [RolePermissions.WriteFiles, RolePermissions.WriteDatabase];

export const DEFAULT_TABLE_PAGE_SIZE = 25;

export interface TableInspectorProps {
	appId: string;
	table: string;
	userScoped?: boolean;
	selector?: IDatabaseSelector;
	onSelectorChange?: (selector: IDatabaseSelector) => void;
	onTableDeleted?: () => void;
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

export function TableInspector(props: Readonly<TableInspectorProps>) {
	return (
		<TableInspectorView
			key={`${props.appId}:${props.table}:${props.userScoped ?? false}`}
			{...props}
		/>
	);
}

function TableInspectorView(props: Readonly<TableInspectorProps>) {
	const permissions = useAppPermissions(props.appId);
	const backend = useBackend();
	const [internalSelector, setInternalSelector] = useState<IDatabaseSelector>(
		{},
	);
	const selector = props.selector ?? internalSelector;
	const referenceHistory = useInvoke(
		backend.dbState.databaseHistory,
		backend.dbState,
		[props.appId, props.table, props.userScoped, selector],
		Boolean(props.appId && props.table) && permissions.can(...READ_DATA),
	);
	// Resolve a movable tag once so rows, schema and count use the same version.
	const resolvedSelector =
		selector.tag && referenceHistory.data?.reference
			? {
					branch: referenceHistory.data.reference.branch,
					version: referenceHistory.data.reference.version,
					read_only: selector.read_only,
				}
			: selector;
	const select = (next: IDatabaseSelector) => {
		setInternalSelector(next);
		props.onSelectorChange?.(next);
		if (!props.onSelectorChange)
			props.onPageChange?.(1, props.pageSize ?? DEFAULT_TABLE_PAGE_SIZE);
	};
	return (
		<div
			className={cn("flex h-full min-h-0 min-w-0 flex-col", props.className)}
		>
			{props.appId && props.table && permissions.can(...READ_DATA) && (
				<DatabaseHistoryControls
					key={databaseSelectorKey(selector)}
					appId={props.appId}
					table={props.table}
					userScoped={props.userScoped}
					selector={selector}
					canWrite={permissions.can(...WRITE_DATA)}
					onSelect={select}
				/>
			)}
			{(referenceHistory.error ||
				(selector.tag && !referenceHistory.data?.reference)) &&
			permissions.can(...READ_DATA) ? (
				<div className="space-y-2 p-4 text-sm">
					{props.children}
					<p role={referenceHistory.error ? "alert" : "status"}>
						{referenceHistory.error
							? getErrorMessage(referenceHistory.error)
							: "Resolving tagged snapshot…"}
					</p>
					{referenceHistory.error && (
						<Button
							variant="outline"
							onClick={() => void referenceHistory.refetch()}
						>
							Retry
						</Button>
					)}
				</div>
			) : (
				<TableInspectorData
					key={databaseSelectorKey(resolvedSelector)}
					{...props}
					className="min-h-0"
					selector={resolvedSelector}
				/>
			)}
		</div>
	);
}

function TableInspectorData({
	appId,
	table,
	userScoped,
	selector,
	onTableDeleted,
	page,
	pageSize,
	onPageChange,
	className,
	embedded,
	children,
}: Readonly<TableInspectorProps>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const permissions = useAppPermissions(appId);
	const canRead = permissions.can(...READ_DATA);
	const canWrite =
		permissions.can(...WRITE_DATA) && !isDatabaseSnapshot(selector);

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
	const hasTarget = Boolean(appId) && Boolean(table);
	const enabled = hasTarget && canRead;

	const schema = useInvoke(
		backend.dbState.getSchema,
		backend.dbState,
		[appId, table, userScoped, selector],
		enabled,
	);
	const count = useInvoke(
		backend.dbState.countItems,
		backend.dbState,
		[appId, table, userScoped, selector],
		enabled,
	);
	const list = useInvoke(
		backend.dbState.listItems,
		backend.dbState,
		[appId, table, offset, activePageSize, userScoped, selector],
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
		void invalidate(backend.dbState.databaseHistory, [appId, table]);
	}, [
		schema.refetch,
		count.refetch,
		list.refetch,
		invalidate,
		backend.dbState,
		appId,
		table,
	]);

	const handleOptimize = useCallback(
		async (keepVersions = true) => {
			try {
				await backend.dbState.optimize(
					appId,
					table,
					keepVersions,
					userScoped,
					selector,
				);
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
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
					selector,
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
	);

	const handleDropColumns = useCallback(
		async (columns: string[]) => {
			try {
				await backend.dbState.dropColumns(
					appId,
					table,
					columns,
					userScoped,
					selector,
				);
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
	);

	const handleAddColumn = useCallback(
		async (name: string, sqlExpression: string) => {
			try {
				await backend.dbState.addColumn(
					appId,
					table,
					{ name, sql_expression: sqlExpression },
					userScoped,
					selector,
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
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
					selector,
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
	);

	const handleGetIndices = useCallback(
		async () => backend.dbState.getIndices(appId, table, userScoped, selector),
		[backend.dbState, appId, table, userScoped, selector],
	);

	const handleDropIndex = useCallback(
		async (indexName: string) => {
			try {
				await backend.dbState.dropIndex(
					appId,
					table,
					indexName,
					userScoped,
					selector,
				);
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
	);

	const handleBuildIndex = useCallback(
		async (column: string, indexType: string) => {
			try {
				await backend.dbState.buildIndex(
					appId,
					table,
					column,
					parseIndexType(indexType),
					undefined,
					userScoped,
					selector,
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
		[backend.dbState, appId, table, userScoped, selector, handleRefresh, t],
	);

	// Column layouts are stored per table; without the app they collide across
	// every project that happens to use the same table name.
	const settingsScope = userScoped ? `${appId}:user` : appId;

	const containerCls = cn(
		"flex flex-col h-full grow max-h-full min-w-0",
		className,
	);

	if (!hasTarget) {
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

	if (permissions.isLoading) {
		return (
			<TableInspectorSkeleton className={containerCls} tableName={table}>
				{children}
			</TableInspectorSkeleton>
		);
	}

	// An empty grid reads as "this table has no rows", which is the opposite of
	// "you may not read it" — so a denial replaces the grid instead of filling it.
	if (!canRead) {
		return (
			<div className={cn(containerCls, "p-4 gap-4")}>
				<TableInspectorHeader tableName={table}>
					{children}
				</TableInspectorHeader>
				<SectionLockedPanel
					feature={t("tableData", "Table data")}
					description={t(
						"yourRoleCannotReadThisProjectsTables",
						"Your role cannot read this project's tables, so this table's rows, schema and indexes stay hidden.",
					)}
					missing={READ_DATA}
					roleName={permissions.roleName}
				/>
			</div>
		);
	}

	const loadError = schema.error ?? list.error ?? count.error;
	if (loadError) {
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
			{!permissions.can(...WRITE_DATA) && (
				<PermissionNotice
					tone="readOnly"
					className="mb-3 shrink-0"
					title={t("tableIsReadOnly", "This table is read-only for you")}
					description={t(
						"editingRowsColumnsAndIndexesNeedsWriteAccess",
						"Editing rows, adding or altering columns, building indexes and optimizing are hidden because your role cannot write this project's data.",
					)}
					missing={WRITE_DATA}
				/>
			)}
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
				onOptimize={canWrite ? handleOptimize : undefined}
				onUpdateItem={canWrite ? handleUpdateItem : undefined}
				onDropColumns={canWrite ? handleDropColumns : undefined}
				onAddColumn={canWrite ? handleAddColumn : undefined}
				onAlterColumn={canWrite ? handleAlterColumn : undefined}
				onGetIndices={handleGetIndices}
				onDropIndex={canWrite ? handleDropIndex : undefined}
				onBuildIndex={canWrite ? handleBuildIndex : undefined}
			>
				{children}
				{permissions.can(...WRITE_DATA) && (
					<DatabaseDeleteControls
						appId={appId}
						table={table}
						userScoped={userScoped}
						selector={selector ?? {}}
						onChanged={handleRefresh}
						onTableDeleted={onTableDeleted}
					/>
				)}
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
