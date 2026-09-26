"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CloudDownloadIcon,
	KeyRoundIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { useId, useState } from "react";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvoke } from "../../../hooks/use-invoke";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { useBackend } from "../../../state/backend-state";
import type {
	IOfflineTableState,
	IOfflineWritesState,
	OfflineTablePurpose,
} from "../../../state/backend-state/offline-writes-state";
import { Badge } from "../../ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Label } from "../../ui/label";
import { Progress } from "../../ui/progress";
import { Skeleton } from "../../ui/skeleton";
import { Switch } from "../../ui/switch";
import {
	EnableOfflineTableDialog,
	type OfflineTableTarget,
} from "./enable-offline-table-dialog";
import {
	type OfflineTableRow as TableRowModel,
	availabilityLine,
	isRegistered,
	isSupportedTableName,
	mergeTableRows,
	tableDetailLines,
	tableStatusLabel,
} from "./offline-access-logic";

function errorText(cause: unknown): string {
	return cause instanceof Error ? cause.message : String(cause);
}

function PrefetchControl({
	appId,
	table,
	state,
	onChanged,
}: Readonly<{
	appId: string;
	table: IOfflineTableState;
	state: IOfflineWritesState;
	onChanged: () => void;
}>) {
	const { t } = useTranslation("settings");
	const id = useId();
	const [pending, setPending] = useState<boolean>();
	const [error, setError] = useState<string>();

	const toggle = async (prefetch: boolean) => {
		setPending(prefetch);
		setError(undefined);
		try {
			await state.setPrefetch(appId, table.purpose, table.table, prefetch);
			onChanged();
		} catch (cause) {
			setError(errorText(cause));
		} finally {
			setPending(undefined);
		}
	};

	const progress =
		table.downloading && table.totalBytes
			? Math.min(100, (table.cachedBytes / table.totalBytes) * 100)
			: undefined;

	return (
		<div className="space-y-1.5">
			<div className="flex items-center gap-2">
				<Switch
					id={id}
					checked={pending ?? table.prefetch}
					disabled={pending !== undefined}
					onCheckedChange={(checked) => void toggle(checked)}
				/>
				<Label htmlFor={id} className="flex items-center gap-1.5 font-normal">
					<CloudDownloadIcon className="size-4" />
					{t(
						"settings:offlineAccess.downloadEverything",
						"Download everything",
					)}
				</Label>
			</div>
			<p className="text-xs text-muted-foreground">
				{availabilityLine(t, table)}
			</p>
			{progress !== undefined && <Progress value={progress} className="h-1" />}
			{error && (
				<p role="alert" className="text-xs text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}

export function OfflineTableRow({
	appId,
	row,
	state,
	enableBlocked,
	onEnable,
	onChanged,
}: Readonly<{
	appId: string;
	row: TableRowModel;
	state: IOfflineWritesState;
	enableBlocked: boolean;
	onEnable: (target: OfflineTableTarget) => void;
	onChanged: () => void;
}>) {
	const { t } = useTranslation("settings");
	const switchId = useId();
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const configured = row.state;
	const supported = isSupportedTableName(row.table);

	const disable = async (table: IOfflineTableState) => {
		if (table.pendingCount > 0) {
			setError(
				t("settings:offlineAccess.disableBlockedPending", {
					count: table.pendingCount,
					defaultValue_one:
						"{{count}} change is still waiting to sync. Sync or skip it first.",
					defaultValue_other:
						"{{count}} changes are still waiting to sync. Sync or skip them first.",
				}),
			);
			return;
		}
		setBusy(true);
		setError(undefined);
		try {
			await state.removeTable(appId, table.purpose, table.table);
			onChanged();
		} catch (cause) {
			setError(errorText(cause));
		} finally {
			setBusy(false);
		}
	};

	return (
		<li className="space-y-2 p-3">
			<div className="flex items-start gap-3">
				<Switch
					id={switchId}
					className="mt-0.5"
					checked={!!configured}
					disabled={busy || !supported || (!configured && enableBlocked)}
					aria-label={
						configured
							? t("settings:offlineAccess.disableAction", "Turn off")
							: t("settings:offlineAccess.enableConfirmAction", "Turn on")
					}
					onCheckedChange={(checked) => {
						if (checked) {
							onEnable({ purpose: row.purpose, table: row.table });
						} else if (configured) {
							void disable(configured);
						}
					}}
				/>
				<div className="min-w-0 flex-1 space-y-1">
					<div className="flex flex-wrap items-center gap-2">
						<Label htmlFor={switchId} className="break-all font-medium">
							{row.table}
						</Label>
						{configured && (
							<Badge
								variant={
									configured.status === "error" || configured.remoteMissing
										? "destructive"
										: "secondary"
								}
							>
								{tableStatusLabel(t, configured)}
							</Badge>
						)}
						{configured && (
							<span className="flex items-center gap-1 text-xs text-muted-foreground">
								<KeyRoundIcon className="size-3" />
								{t("settings:offlineAccess.keyColumn", "Key column")}:{" "}
								<code>{configured.primaryKey}</code>
							</span>
						)}
					</div>
					{!supported && (
						<p className="text-xs text-muted-foreground">
							{t(
								"settings:offlineAccess.unsupportedTableName",
								"Tables with dots or special characters in their name cannot be available offline.",
							)}
						</p>
					)}
					{configured &&
						tableDetailLines(t, configured).map((line) => (
							<p key={line} className="text-xs text-muted-foreground">
								{line}
							</p>
						))}
					{configured?.keyIndexed === false && (
						<p className="flex items-center gap-1 text-xs text-muted-foreground">
							<TriangleAlertIcon className="size-3" />
							{t(
								"settings:offlineAccess.keyNotIndexed",
								"The key column has no index, so changes read the whole table.",
							)}
						</p>
					)}
					{configured && isRegistered(configured) && (
						<PrefetchControl
							appId={appId}
							table={configured}
							state={state}
							onChanged={onChanged}
						/>
					)}
					{error && (
						<p role="alert" className="text-xs text-destructive">
							{error}
						</p>
					)}
				</div>
			</div>
		</li>
	);
}

function TableGroup({
	title,
	rows,
	loading,
	listFailed,
	children,
}: Readonly<{
	title: string;
	rows: TableRowModel[];
	loading: boolean;
	listFailed: boolean;
	children: (row: TableRowModel) => React.ReactNode;
}>) {
	const { t } = useTranslation("settings");
	return (
		<section className="space-y-2">
			<h4 className="text-sm font-medium">{title}</h4>
			{loading ? (
				<Skeleton className="h-12 w-full" />
			) : (
				rows.length > 0 && (
					<ul className="divide-y rounded-md border">{rows.map(children)}</ul>
				)
			)}
			{listFailed && (
				<p className="text-xs text-muted-foreground">
					{t(
						"settings:offlineAccess.tablesListUnavailable",
						"Connect to the hub to see all tables.",
					)}
				</p>
			)}
		</section>
	);
}

export function OfflineTablesCard({
	appId,
	state,
	tables,
	enableBlocked,
	onChanged,
}: Readonly<{
	appId: string;
	state: IOfflineWritesState;
	tables: IOfflineTableState[];
	enableBlocked: boolean;
	onChanged: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const permissions = useAppPermissions(appId);
	const canReadProject = permissions.can(RolePermissions.ReadFiles);
	const [target, setTarget] = useState<OfflineTableTarget | null>(null);

	const userTables = useInvoke(
		backend.dbState.listTablesUser,
		backend.dbState,
		[appId],
	);
	const projectTables = useInvoke(
		backend.dbState.listTables,
		backend.dbState,
		[appId],
		canReadProject,
	);

	const group = (
		purpose: OfflineTablePurpose,
		query: { data?: string[]; isError: boolean; isLoading: boolean },
	) => ({
		rows: mergeTableRows(
			purpose,
			query.isError ? undefined : query.data,
			tables,
		),
		listFailed: query.isError,
		loading: query.isLoading,
	});
	const configuredProject = tables.some((table) => table.purpose === "storage");
	const projectGroup = canReadProject
		? group("storage", projectTables)
		: configuredProject
			? group("storage", { isError: false, isLoading: false })
			: undefined;
	const userGroup = group("user", userTables);

	const renderRow = (row: TableRowModel) => (
		<OfflineTableRow
			key={`${row.purpose}:${row.table}`}
			appId={appId}
			row={row}
			state={state}
			enableBlocked={enableBlocked}
			onEnable={setTarget}
			onChanged={onChanged}
		/>
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle>
					{t("settings:offlineAccess.tablesTitle", "Tables available offline")}
				</CardTitle>
				<CardDescription>
					{t(
						"settings:offlineAccess.tablesDescription",
						"Flows on this device read these tables from a local copy that downloads data as it is used. Changes are queued and sent to the cloud in order.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-5">
				{projectGroup && (
					<TableGroup
						title={t("settings:offlineAccess.projectTables", "Project tables")}
						{...projectGroup}
					>
						{renderRow}
					</TableGroup>
				)}
				<TableGroup
					title={t("settings:offlineAccess.userTables", "Your tables")}
					{...userGroup}
				>
					{renderRow}
				</TableGroup>
			</CardContent>
			<EnableOfflineTableDialog
				appId={appId}
				target={target}
				state={state}
				onOpenChange={(open) => {
					if (!open) setTarget(null);
				}}
				onEnabled={onChanged}
			/>
		</Card>
	);
}
