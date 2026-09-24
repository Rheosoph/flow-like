"use client";

import { useTranslation } from "@flow-like/locales";
import { CloudDownloadIcon, InfoIcon, TriangleAlertIcon } from "lucide-react";
import { useEffect, useId, useMemo, useState } from "react";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import type {
	IOfflineWritesState,
	OfflineTablePurpose,
} from "../../../state/backend-state/offline-writes-state";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Skeleton } from "../../ui/skeleton";
import {
	defaultKeyColumn,
	hasKeyIndex,
	isReadOnlyForTable,
	keyColumnCandidates,
	recentVersionCount,
} from "./offline-access-logic";

export interface OfflineTableTarget {
	purpose: OfflineTablePurpose;
	table: string;
}

function Notice({
	tone,
	children,
}: Readonly<{ tone: "warning" | "info"; children: React.ReactNode }>) {
	const Icon = tone === "warning" ? TriangleAlertIcon : InfoIcon;
	return (
		<p
			className={
				tone === "warning"
					? "flex items-start gap-2 text-sm text-destructive"
					: "flex items-start gap-2 text-sm text-muted-foreground"
			}
		>
			<Icon className="mt-0.5 size-4 shrink-0" />
			<span>{children}</span>
		</p>
	);
}

function EnableForm({
	appId,
	target,
	state,
	onDone,
	onCancel,
}: Readonly<{
	appId: string;
	target: OfflineTableTarget;
	state: IOfflineWritesState;
	onDone: () => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const permissions = useAppPermissions(appId);
	const prefetchId = useId();
	const userScoped = target.purpose === "user";
	const args: [string, string, boolean] = [appId, target.table, userScoped];

	const schema = useInvoke(backend.dbState.getSchema, backend.dbState, args);
	const indices = useInvoke(backend.dbState.getIndices, backend.dbState, args);
	const history = useInvoke(
		backend.dbState.databaseHistory,
		backend.dbState,
		args,
	);

	const candidates = useMemo(
		() => keyColumnCandidates(schema.data),
		[schema.data],
	);
	const [key, setKey] = useState<string>();
	const [prefetch, setPrefetch] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();

	useEffect(() => {
		setKey((current) =>
			current && candidates.includes(current)
				? current
				: defaultKeyColumn(candidates),
		);
	}, [candidates]);

	const recentChanges = useMemo(
		() => recentVersionCount(history.data?.versions ?? [], Date.now()),
		[history.data],
	);
	const keyUnindexed =
		!!key && !!indices.data && !hasKeyIndex(indices.data, key);
	const readOnly = isReadOnlyForTable(target.purpose, permissions.can);

	const confirm = async () => {
		if (!key) return;
		setBusy(true);
		setError(undefined);
		try {
			await state.setTable(appId, {
				purpose: target.purpose,
				table: target.table,
				primaryKey: key,
				prefetch,
			});
			onDone();
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setBusy(false);
		}
	};

	return (
		<>
			<div className="space-y-4">
				<div className="space-y-2">
					<Label>{t("settings:offlineAccess.keyColumn", "Key column")}</Label>
					{schema.isLoading ? (
						<Skeleton className="h-9 w-full" />
					) : candidates.length === 0 ? (
						<Notice tone="warning">
							{schema.error?.message ??
								t(
									"settings:offlineAccess.noCompatibleKey",
									"No column can serve as a key. Offline tables need a text or whole-number column with unique values.",
								)}
						</Notice>
					) : (
						<Select value={key} onValueChange={setKey} disabled={busy}>
							<SelectTrigger className="w-full">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{candidates.map((column) => (
									<SelectItem key={column} value={column}>
										{column}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					)}
					<p className="text-xs text-muted-foreground">
						{t(
							"settings:offlineAccess.keyColumnHint",
							"Each row needs a unique value in this column. Offline changes are matched by it.",
						)}
					</p>
				</div>

				<div className="flex items-start gap-2">
					<Checkbox
						id={prefetchId}
						checked={prefetch}
						disabled={busy}
						onCheckedChange={(value) => setPrefetch(value === true)}
					/>
					<div className="space-y-1">
						<Label htmlFor={prefetchId} className="flex items-center gap-1.5">
							<CloudDownloadIcon className="size-4" />
							{t(
								"settings:offlineAccess.downloadEverything",
								"Download everything",
							)}
						</Label>
						<p className="text-xs text-muted-foreground">
							{t(
								"settings:offlineAccess.downloadEverythingHint",
								"Keep the whole table on this device so every flow can use it offline. Updates download only what changed.",
							)}
						</p>
					</div>
				</div>

				<div className="space-y-2">
					{keyUnindexed && (
						<Notice tone="warning">
							{t(
								"settings:offlineAccess.enableKeyIndexWarning",
								"The key column has no index. The first change made on this device downloads the whole table, and changes made offline need all of it on this device. Turn on Download everything if you plan to change this table offline.",
							)}
						</Notice>
					)}
					{recentChanges > 0 && (
						<Notice tone="warning">
							{t("settings:offlineAccess.enableHistoryWarning", {
								count: recentChanges,
								defaultValue_one:
									"This table changed {{count}} time in the last 7 days. If it changes elsewhere while this device has queued changes, those changes will need your decision.",
								defaultValue_other:
									"This table changed {{count}} times in the last 7 days. If it changes elsewhere while this device has queued changes, those changes will need your decision.",
							})}
						</Notice>
					)}
					{readOnly && (
						<Notice tone="warning">
							{t(
								"settings:offlineAccess.enableReadOnlyWarning",
								"Your role cannot change this table. You can read it offline, but changes you make will be rejected when they sync.",
							)}
						</Notice>
					)}
					<Notice tone="info">
						{t(
							"settings:offlineAccess.enableThroughputNote",
							"Changes are sent in order, a batch at a time. Large imports reach the cloud more slowly than direct writes.",
						)}
					</Notice>
				</div>

				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</div>
			<DialogFooter>
				<Button
					type="button"
					variant="outline"
					disabled={busy}
					onClick={onCancel}
				>
					{t("common:cancel", "Cancel")}
				</Button>
				<Button
					type="button"
					disabled={busy || !key}
					onClick={() => void confirm()}
				>
					{t("settings:offlineAccess.enableConfirmAction", "Turn on")}
				</Button>
			</DialogFooter>
		</>
	);
}

export function EnableOfflineTableDialog({
	appId,
	target,
	state,
	onOpenChange,
	onEnabled,
}: Readonly<{
	appId: string;
	target: OfflineTableTarget | null;
	state: IOfflineWritesState;
	onOpenChange: (open: boolean) => void;
	onEnabled: () => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<Dialog open={target !== null} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{t(
							"settings:offlineAccess.enableConfirmTitle",
							"Make “{{table}}” available offline?",
							{ table: target?.table ?? "" },
						)}
					</DialogTitle>
					<DialogDescription>
						{t(
							"settings:offlineAccess.enableConfirmBody",
							"Flows on this device will read this table from a local copy and queue their changes, even while connected. Data is downloaded as flows use it; while offline, only data that is already on this device can be used. Structure changes, branches and graph operations stay unavailable for this table on this device. Changes made elsewhere arrive every few minutes. If someone changes the table while your changes are queued, you will be asked to resolve it.",
						)}
					</DialogDescription>
				</DialogHeader>
				{target && (
					<EnableForm
						key={`${target.purpose}:${target.table}`}
						appId={appId}
						target={target}
						state={state}
						onCancel={() => onOpenChange(false)}
						onDone={() => {
							onOpenChange(false);
							onEnabled();
						}}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
