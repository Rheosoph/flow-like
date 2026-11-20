"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckCircle2,
	KeyRound,
	Loader2,
	Pencil,
	Plug,
	Plus,
	Server,
	Trash2,
	XCircle,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import { apiErrorMessage, isMissingRouteError } from "../../../lib/api-error";
import { getErrorMessage } from "../../../lib/error-message";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { asArray } from "../../../lib/response-shape";
import { useBackend } from "../../../state/backend-state";
import type {
	ExternalDataSource,
	ExternalDataSourceCheck,
} from "../../../state/backend-state/data-source-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import {
	ExternalDatabaseDialog,
	QUERYABLE_DRIVERS,
	driverSpecs,
} from "./external-database-dialog";

const SHOWN_TABLES = 8;

interface CheckState {
	readonly running: boolean;
	readonly result?: ExternalDataSourceCheck;
}

/** Where a source points, in one line: `db.example.com:5432 · sales.public`. */
function locationOf(source: ExternalDataSource): string {
	const config = source.config ?? {};
	const text = (key: string) => {
		const value = config[key];
		return typeof value === "string" || typeof value === "number"
			? String(value)
			: "";
	};
	switch (source.driver) {
		case "bigquery":
			return [text("project_id"), text("dataset")].filter(Boolean).join(".");
		case "athena":
			return [text("region"), text("database")].filter(Boolean).join(" · ");
		default: {
			const target = [text("database"), text("schema")]
				.filter(Boolean)
				.join(".");
			return [`${text("host")}:${text("port")}`, target]
				.filter(Boolean)
				.join(" · ");
		}
	}
}

export function ExternalDatabasesSection({
	appId,
}: Readonly<{ appId: string }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const permissions = useAppPermissions(appId);
	const canRead = permissions.can(RolePermissions.ReadDatabase);
	const canManage = permissions.can(RolePermissions.Admin);
	const offline = useInvoke(backend.isOffline, backend, [appId], !!appId);
	const online = offline.data === false;
	const sources = useInvoke(
		backend.dataSourceState.listDataSources,
		backend.dataSourceState,
		[appId],
		canRead && online,
	);
	const specs = useMemo(() => driverSpecs(t), [t]);

	const [dialogOpen, setDialogOpen] = useState(false);
	const [editing, setEditing] = useState<ExternalDataSource | null>(null);
	const [deleting, setDeleting] = useState<ExternalDataSource | null>(null);
	const [deletePending, setDeletePending] = useState(false);
	const [checks, setChecks] = useState<Record<string, CheckState>>({});

	const refresh = useCallback(
		() => invalidate(backend.dataSourceState.listDataSources, [appId]),
		[appId, backend.dataSourceState, invalidate],
	);

	const openCreate = useCallback(() => {
		setEditing(null);
		setDialogOpen(true);
	}, []);

	const openEdit = useCallback((source: ExternalDataSource) => {
		setEditing(source);
		setDialogOpen(true);
	}, []);

	const check = useCallback(
		async (source: ExternalDataSource) => {
			setChecks((previous) => ({
				...previous,
				[source.id]: { running: true },
			}));
			let result: ExternalDataSourceCheck;
			try {
				result = await backend.dataSourceState.checkDataSource(
					appId,
					source.id,
				);
			} catch (error) {
				result = {
					ok: false,
					tables: [],
					error: isMissingRouteError(error)
						? t(
								"externalDbCheckUnavailable",
								"This hub cannot check connections yet.",
							)
						: apiErrorMessage(error, getErrorMessage(error)),
				};
			}
			setChecks((previous) => ({
				...previous,
				[source.id]: { running: false, result },
			}));
			await refresh();
		},
		[appId, backend.dataSourceState, refresh, t],
	);

	const confirmDelete = useCallback(async () => {
		if (!deleting) return;
		setDeletePending(true);
		try {
			await backend.dataSourceState.deleteDataSource(appId, deleting.id);
			toast.success(
				t("externalDbRemoved", "Removed {{name}}", { name: deleting.name }),
			);
			setDeleting(null);
			await refresh();
		} catch (error) {
			toast.error(
				t("externalDbRemoveFailed", "Could not remove {{name}}: {{error}}", {
					name: deleting.name,
					error: apiErrorMessage(error, getErrorMessage(error)),
				}),
			);
		} finally {
			setDeletePending(false);
		}
	}, [appId, backend.dataSourceState, deleting, refresh, t]);

	const list = asArray(sources.data);
	const listError = sources.error;

	return (
		<div className="space-y-4 pt-4">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div className="min-w-0">
					<h2 className="flex items-center gap-2 font-semibold">
						<Server className="h-4 w-4" />
						{t("externalDatabases", "External databases")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t(
							"externalDatabasesDescription",
							"PostgreSQL, CockroachDB, MySQL, MariaDB and ClickHouse tables that this app's queries read as schema.table. Credentials are stored encrypted.",
						)}
					</p>
				</div>
				{online && canRead && (
					<Button
						size="sm"
						disabled={!canManage}
						title={
							canManage
								? undefined
								: t(
										"externalDbAdminOnly",
										"Only admins can connect external databases.",
									)
						}
						onClick={openCreate}
					>
						<Plus className="h-4 w-4" />
						{t("externalDbConnectDatabase", "Connect database")}
					</Button>
				)}
			</div>

			{offline.data === true ? (
				<Notice>
					{t(
						"externalDbNeedsOnlineApp",
						"External databases are read through the Flow-Like hub, so they need an online app.",
					)}
				</Notice>
			) : !canRead ? (
				<Notice>
					{t(
						"externalDbNoReadAccess",
						"Your role cannot read this app's databases.",
					)}
				</Notice>
			) : listError ? (
				<Notice tone="error">
					{isMissingRouteError(listError)
						? t(
								"externalDbUnsupportedHub",
								"This hub does not support external databases yet.",
							)
						: apiErrorMessage(listError, getErrorMessage(listError))}
				</Notice>
			) : sources.isLoading || offline.isLoading ? (
				<div className="flex items-center gap-2 text-sm text-muted-foreground">
					<Loader2 className="h-4 w-4 animate-spin" />
					{t("externalDbLoading", "Loading external databases…")}
				</div>
			) : list.length === 0 ? (
				<Notice>
					{t(
						"externalDbEmpty",
						"No external databases yet. Connect one to join its tables with this app's data in SQL.",
					)}
				</Notice>
			) : (
				<div className="grid grid-cols-1 gap-4 @xl/ds:grid-cols-2 @4xl/ds:grid-cols-3">
					{list.map((source) => (
						<ExternalDatabaseCard
							key={source.id}
							source={source}
							driverLabel={specs[source.driver]?.label ?? source.driver}
							check={checks[source.id]}
							canManage={canManage}
							onCheck={() => check(source)}
							onEdit={() => openEdit(source)}
							onDelete={() => setDeleting(source)}
						/>
					))}
				</div>
			)}

			<ExternalDatabaseDialog
				open={dialogOpen}
				onOpenChange={setDialogOpen}
				appId={appId}
				source={editing}
				onSaved={() => {
					void refresh();
				}}
			/>

			<AlertDialog
				open={!!deleting}
				onOpenChange={(open) => {
					if (!open && !deletePending) setDeleting(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("externalDbRemoveTitle", "Remove {{name}}?", {
								name: deleting?.name ?? "",
							})}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"externalDbRemoveDescription",
								"Queries that read {{alias}}.<table> stop working, and the stored credentials are deleted. The database itself is not touched.",
								{ alias: deleting?.alias ?? "" },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel disabled={deletePending}>
							{t("cancel", "Cancel")}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-white hover:bg-destructive/90"
							disabled={deletePending}
							onClick={(event) => {
								event.preventDefault();
								void confirmDelete();
							}}
						>
							{deletePending && <Loader2 className="h-4 w-4 animate-spin" />}
							{t("externalDbRemove", "Remove")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

function Notice({
	children,
	tone = "muted",
}: Readonly<{ children: ReactNode; tone?: "muted" | "error" }>) {
	return (
		<div
			className={
				tone === "error"
					? "rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive"
					: "rounded-lg border border-dashed px-4 py-3 text-sm text-muted-foreground"
			}
		>
			{children}
		</div>
	);
}

function ExternalDatabaseCard({
	source,
	driverLabel,
	check,
	canManage,
	onCheck,
	onEdit,
	onDelete,
}: Readonly<{
	source: ExternalDataSource;
	driverLabel: string;
	check?: CheckState;
	canManage: boolean;
	onCheck: () => void;
	onEdit: () => void;
	onDelete: () => void;
}>) {
	const { t } = useTranslation("settings");
	const queryable = QUERYABLE_DRIVERS.has(source.driver);
	const lastStatus = source.last_check_status;
	const result = check?.result;

	return (
		<Card className="flex flex-col gap-4 p-5">
			<div className="flex items-start justify-between gap-3">
				<div className="flex min-w-0 items-center gap-3">
					<div className="shrink-0 rounded-xl bg-primary/10 p-2.5">
						<Server className="h-5 w-5 text-primary" />
					</div>
					<div className="min-w-0">
						<h3 className="truncate text-sm font-semibold leading-tight">
							{source.name}
						</h3>
						<p className="truncate text-xs text-muted-foreground">
							{locationOf(source)}
						</p>
					</div>
				</div>
				<Badge variant="outline" className="shrink-0 font-mono text-[10px]">
					{source.alias}
				</Badge>
			</div>

			<div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
				<Badge variant="secondary" className="text-[10px]">
					{driverLabel}
				</Badge>
				{!queryable && (
					<Badge variant="outline" className="text-[10px]">
						{t("externalDbQuerySupportComing", "Query support coming")}
					</Badge>
				)}
				<span className="flex items-center gap-1">
					<KeyRound className="h-3 w-3" />
					{source.secret_fields.length > 0
						? t("externalDbCredentialsStoredShort", "Credentials stored")
						: t("externalDbNoCredentials", "No credentials")}
				</span>
			</div>

			{result ? (
				<CheckResult alias={source.alias} result={result} />
			) : lastStatus ? (
				<p
					className={
						lastStatus === "ok"
							? "flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400"
							: "line-clamp-2 text-xs text-destructive"
					}
					title={lastStatus === "ok" ? undefined : lastStatus}
				>
					{lastStatus === "ok" ? (
						<>
							<CheckCircle2 className="h-3.5 w-3.5" />
							{t("externalDbLastCheckOk", "Connected at the last check")}
						</>
					) : (
						t("externalDbLastCheckFailed", "Last check failed: {{error}}", {
							error: lastStatus,
						})
					)}
				</p>
			) : null}

			<div className="mt-auto flex flex-wrap items-center gap-2 border-t pt-3">
				<Button
					variant="outline"
					size="sm"
					onClick={onCheck}
					disabled={!queryable || check?.running}
					title={
						queryable
							? undefined
							: t(
									"externalDbCheckComing",
									"Connection checks for this database come with its query support.",
								)
					}
				>
					{check?.running ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<Plug className="h-4 w-4" />
					)}
					{t("externalDbCheckConnection", "Check connection")}
				</Button>
				<div className="ml-auto flex gap-1">
					<Button
						variant="ghost"
						size="icon"
						onClick={onEdit}
						disabled={!canManage}
						title={t("externalDbEdit", "Edit")}
						aria-label={t("externalDbEdit", "Edit")}
					>
						<Pencil className="h-4 w-4" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						onClick={onDelete}
						disabled={!canManage}
						title={t("externalDbRemove", "Remove")}
						aria-label={t("externalDbRemove", "Remove")}
					>
						<Trash2 className="h-4 w-4" />
					</Button>
				</div>
			</div>
		</Card>
	);
}

function CheckResult({
	alias,
	result,
}: Readonly<{ alias: string; result: ExternalDataSourceCheck }>) {
	const { t } = useTranslation("settings");
	const tables = asArray(result.tables);
	if (!result.ok) {
		return (
			<p className="flex items-start gap-1.5 text-xs text-destructive">
				<XCircle className="mt-px h-3.5 w-3.5 shrink-0" />
				<span className="line-clamp-3" title={result.error ?? undefined}>
					{result.error ??
						t("externalDbCheckFailed", "The connection check failed.")}
				</span>
			</p>
		);
	}
	return (
		<div className="space-y-2">
			<p className="flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
				<CheckCircle2 className="h-3.5 w-3.5" />
				{t("externalDbCheckOk", {
					defaultValue_one: "Connected · {{count}} table",
					defaultValue_other: "Connected · {{count}} tables",
					count: tables.length,
				})}
			</p>
			{tables.length > 0 && (
				<div className="flex flex-wrap gap-1">
					{tables.slice(0, SHOWN_TABLES).map((table) => (
						<Badge
							key={table}
							variant="outline"
							className="font-mono text-[10px]"
						>
							{alias}.{table}
						</Badge>
					))}
					{tables.length > SHOWN_TABLES && (
						<Badge variant="outline" className="text-[10px]">
							{t("externalDbMoreTables", "+{{number}} more", {
								number: tables.length - SHOWN_TABLES,
							})}
						</Badge>
					)}
				</div>
			)}
		</div>
	);
}
