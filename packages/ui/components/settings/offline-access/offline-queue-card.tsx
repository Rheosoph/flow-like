"use client";

import { useTranslation } from "@flow-like/locales";
import { useInfiniteQuery } from "@tanstack/react-query";
import { CopyIcon, RefreshCwIcon, SearchIcon } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { humanFileSize } from "../../../lib/utils";
import type {
	IOfflineOperation,
	IOfflineOperationLookup,
	IOfflineQueueStatus,
	IOfflineWritesState,
} from "../../../state/backend-state/offline-writes-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Input } from "../../ui/input";
import {
	LIVE_ONLY,
	OPERATIONS_PAGE_SIZE,
	formatTimestamp,
	headActions,
	headMessage,
	lookupLabel,
	offlineQueryKeys,
	operationKindLabel,
	operationLookupState,
	resourceName,
} from "./offline-access-logic";
import { SkipOfflineChangeDialog } from "./skip-offline-change-dialog";

function errorText(cause: unknown): string {
	return cause instanceof Error ? cause.message : String(cause);
}

function CopyIdButton({ id }: Readonly<{ id: string }>) {
	const { t } = useTranslation("settings");
	return (
		<Button
			type="button"
			variant="ghost"
			size="icon"
			className="size-6 shrink-0"
			aria-label={t("common:copyId", "Copy ID")}
			onClick={() => {
				void navigator.clipboard
					.writeText(id)
					.then(() => toast.success(t("common:copied", "Copied!")))
					.catch((cause) => toast.error(errorText(cause)));
			}}
		>
			<CopyIcon className="size-3" />
		</Button>
	);
}

function OperationSummary({
	operation,
}: Readonly<{ operation: IOfflineOperation }>) {
	const { t } = useTranslation("settings");
	return (
		<div className="min-w-0 space-y-1">
			<div className="flex flex-wrap items-center gap-2 text-sm">
				<span className="font-medium">
					{operationKindLabel(t, operation.kind)}
				</span>
				<span className="min-w-0 break-all text-muted-foreground">
					{resourceName(operation.resource)}
				</span>
				<Badge variant="outline">
					{lookupLabel(t, { state: operationLookupState(operation) })}
				</Badge>
			</div>
			<div className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
				<span>{formatTimestamp(operation.createdAt)}</span>
				<span aria-hidden>·</span>
				<span>{humanFileSize(operation.bytes)}</span>
				<span aria-hidden>·</span>
				<code className="min-w-0 truncate">{operation.operationId}</code>
				<CopyIdButton id={operation.operationId} />
			</div>
		</div>
	);
}

export function OfflineBlockedHead({
	operation,
	busy,
	onRetry,
	onSkip,
	onKeepBoth,
}: Readonly<{
	operation: IOfflineOperation;
	busy: boolean;
	onRetry: () => void;
	onSkip: () => void;
	onKeepBoth: () => void;
}>) {
	const { t } = useTranslation("settings");
	const actions = headActions(operation);
	const message = headMessage(t, operation);
	return (
		<div className="space-y-2 rounded-md border border-destructive/40 p-3">
			<OperationSummary operation={operation} />
			{message && <p className="text-sm">{message}</p>}
			<div className="flex flex-wrap gap-2">
				{actions.retry && (
					<Button
						type="button"
						size="sm"
						variant="outline"
						disabled={busy}
						onClick={onRetry}
					>
						<RefreshCwIcon className="mr-1.5 size-3" />
						{t("settings:offlineAccess.retry", "Retry")}
					</Button>
				)}
				{actions.keepBoth && (
					<Button
						type="button"
						size="sm"
						variant="outline"
						disabled={busy}
						onClick={onKeepBoth}
					>
						<CopyIcon className="mr-1.5 size-3" />
						{t("settings:offlineAccess.keepBoth", "Keep both")}
					</Button>
				)}
				{actions.skip && (
					<Button
						type="button"
						size="sm"
						variant="ghost"
						className="text-destructive hover:text-destructive"
						disabled={busy}
						onClick={onSkip}
					>
						{t("settings:offlineAccess.skip", "Skip change")}
					</Button>
				)}
			</div>
		</div>
	);
}

function OperationLookup({
	appId,
	state,
}: Readonly<{ appId: string; state: IOfflineWritesState }>) {
	const { t } = useTranslation("settings");
	const [id, setId] = useState("");
	const [busy, setBusy] = useState(false);
	const [result, setResult] = useState<{
		lookup: IOfflineOperationLookup;
		merged?: IOfflineOperationLookup;
	}>();
	const [error, setError] = useState<string>();

	const find = async () => {
		const operationId = id.trim();
		if (!operationId) return;
		setBusy(true);
		setError(undefined);
		setResult(undefined);
		try {
			const lookup = await state.getOperationState(appId, operationId);
			const merged =
				lookup.state === "superseded" && lookup.supersededBy
					? await state.getOperationState(appId, lookup.supersededBy)
					: undefined;
			setResult({ lookup, merged });
		} catch (cause) {
			setError(errorText(cause));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div className="space-y-2">
			<h4 className="text-sm font-medium">
				{t("settings:offlineAccess.findOperation", "Find a change by ID")}
			</h4>
			<form
				className="flex gap-2"
				onSubmit={(event) => {
					event.preventDefault();
					void find();
				}}
			>
				<Input
					value={id}
					disabled={busy}
					aria-label={t(
						"settings:offlineAccess.findOperation",
						"Find a change by ID",
					)}
					placeholder={t(
						"settings:offlineAccess.operationIdPlaceholder",
						"Change ID",
					)}
					onChange={(event) => setId(event.target.value)}
				/>
				<Button type="submit" variant="outline" disabled={busy || !id.trim()}>
					<SearchIcon className="mr-1.5 size-3" />
					{t("common:find", "Find")}
				</Button>
			</form>
			{result && (
				<output className="block text-sm">
					{lookupLabel(t, result.lookup)}
					{result.merged && ` · ${lookupLabel(t, result.merged)}`}
				</output>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}

export function OfflineQueueCard({
	appId,
	state,
	queue,
	onChanged,
}: Readonly<{
	appId: string;
	state: IOfflineWritesState;
	queue: IOfflineQueueStatus;
	onChanged: () => void;
}>) {
	const { t } = useTranslation("settings");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [notice, setNotice] = useState<string>();
	const [skipping, setSkipping] = useState<IOfflineOperation | null>(null);

	const operations = useInfiniteQuery({
		queryKey: offlineQueryKeys.operations(appId),
		initialPageParam: undefined as number | undefined,
		queryFn: ({ pageParam }) =>
			state.listOperations(appId, pageParam, OPERATIONS_PAGE_SIZE),
		getNextPageParam: (last) =>
			last.length === OPERATIONS_PAGE_SIZE ? last.at(-1)?.sequence : undefined,
		meta: LIVE_ONLY,
	});

	const act = useCallback(
		async (action: () => Promise<string | undefined>) => {
			setBusy(true);
			setError(undefined);
			setNotice(undefined);
			try {
				setNotice(await action());
				onChanged();
			} catch (cause) {
				setError(errorText(cause));
			} finally {
				setBusy(false);
			}
		},
		[onChanged],
	);

	const listed = operations.data?.pages.flat() ?? [];

	return (
		<Card>
			<CardHeader>
				<CardTitle>
					{t("settings:offlineAccess.queueTitle", "Waiting to sync")}
				</CardTitle>
				<CardDescription>
					{queue.pendingCount === 0
						? t("settings:offlineAccess.queueEmpty", "Everything is synced.")
						: [
								t("settings:offlineAccess.pendingCount", {
									count: queue.pendingCount,
									defaultValue_one: "{{count}} change waiting",
									defaultValue_other: "{{count}} changes waiting",
								}),
								humanFileSize(queue.pendingBytes),
								queue.oldestAt != null &&
									t(
										"settings:offlineAccess.oldestPending",
										"Oldest from {{time}}",
										{ time: formatTimestamp(queue.oldestAt) },
									),
							]
								.filter(Boolean)
								.join(" · ")}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{queue.blockedHeads.map((operation) => (
					<OfflineBlockedHead
						key={operation.operationId}
						operation={operation}
						busy={busy}
						onRetry={() =>
							void act(async () => {
								await state.retryOperation(appId, operation.operationId);
								return undefined;
							})
						}
						onKeepBoth={() =>
							void act(async () => {
								const { newPath } = await state.keepBoth(
									appId,
									operation.operationId,
								);
								return t(
									"settings:offlineAccess.keepBothDone",
									"Saved this device's version as {{path}}.",
									{ path: newPath },
								);
							})
						}
						onSkip={() => setSkipping(operation)}
					/>
				))}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				{notice && <output className="block text-sm">{notice}</output>}

				{listed.length > 0 && (
					<div className="space-y-2">
						<h4 className="text-sm font-medium">
							{t("settings:offlineAccess.operationsTitle", "Queued changes")}
						</h4>
						<ul className="divide-y rounded-md border">
							{listed.map((operation) => (
								<li key={operation.operationId} className="p-2">
									<OperationSummary operation={operation} />
								</li>
							))}
						</ul>
						{operations.hasNextPage && (
							<Button
								type="button"
								variant="outline"
								size="sm"
								disabled={operations.isFetchingNextPage}
								onClick={() => void operations.fetchNextPage()}
							>
								{t("settings:offlineAccess.loadMore", "Load more")}
							</Button>
						)}
					</div>
				)}
				{operations.isError && (
					<p role="alert" className="text-sm text-destructive">
						{errorText(operations.error)}
					</p>
				)}

				<OperationLookup appId={appId} state={state} />
			</CardContent>
			<SkipOfflineChangeDialog
				operation={skipping}
				onOpenChange={(open) => {
					if (!open) setSkipping(null);
				}}
				onSkip={async (reason, acknowledgeUncertain) => {
					if (!skipping) return;
					await state.skipOperation(
						appId,
						skipping.operationId,
						reason,
						acknowledgeUncertain,
					);
					onChanged();
				}}
			/>
		</Card>
	);
}
