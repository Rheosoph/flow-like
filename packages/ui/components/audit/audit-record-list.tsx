"use client";

import { useTranslation } from "@flow-like/locales";
import { useDebounce } from "@uidotdev/usehooks";
import {
	Box,
	Clock,
	Cpu,
	EyeOff,
	Globe,
	KeyRound,
	Loader2,
	RefreshCw,
	Search,
	ShieldAlert,
	ShieldCheck,
} from "lucide-react";
import { useMemo, useState } from "react";
import { apiErrorMessage } from "../../lib/api-error";
import type { IProfile } from "../../lib/schema/profile/profile";
import { cn } from "../../lib/utils";
import { Alert, AlertDescription } from "../ui/alert";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { RelativeTime } from "../ui/relative-time";
import { Skeleton } from "../ui/skeleton";
import { UserInlineTag } from "../ui/user-identity";
import {
	auditDetailEntries,
	parseAuditActor,
	useAuditSentence,
} from "./audit-sentence";
import type {
	AuditRecordStatus,
	IAuditRecordFilters,
	IAuditRecordView,
} from "./types";
import { useAuditRecords } from "./use-audit";

function RecordStatusBadge({
	status,
}: Readonly<{ status: AuditRecordStatus }>) {
	const { t } = useTranslation("audit");
	if (status === "sealed") {
		return (
			<Badge
				variant="secondary"
				className="gap-1"
				title={t(
					"statusSealedHint",
					"Part of a seal: any later change breaks verification.",
				)}
			>
				<ShieldCheck />
				{t("statusSealed", "Sealed")}
			</Badge>
		);
	}
	if (status === "invalid") {
		return (
			<Badge
				variant="destructive"
				className="gap-1"
				title={t(
					"statusInvalidHint",
					"Failed its integrity check before sealing and was quarantined.",
				)}
			>
				<ShieldAlert />
				{t("statusInvalid", "Invalid")}
			</Badge>
		);
	}
	return (
		<Badge
			variant="outline"
			className="gap-1"
			title={t(
				"statusPendingHint",
				"Recorded and protected by a MAC; it joins a seal within minutes.",
			)}
		>
			<Clock />
			{t("statusPending", "Pending")}
		</Badge>
	);
}

function RedactedMarker({ kind }: Readonly<{ kind: "ip" | "details" }>) {
	const { t } = useTranslation("audit");
	return (
		<Badge
			variant="outline"
			className="gap-1 border-dashed text-muted-foreground"
			title={t(
				"redactedHint",
				"The value expired under the retention policy. Its commitment still verifies.",
			)}
		>
			<EyeOff />
			{kind === "ip"
				? t("ipRedacted", "IP expired")
				: t("detailsRedacted", "Details expired")}
		</Badge>
	);
}

function AuditActorLabel({
	record,
}: Readonly<{ record: Pick<IAuditRecordView, "actor_id" | "actor_type"> }>) {
	const { t } = useTranslation("audit");
	const actor = useMemo(
		() => parseAuditActor(record.actor_id, record.actor_type),
		[record.actor_id, record.actor_type],
	);

	if (actor.userId) {
		const via =
			actor.method === "pat"
				? t("viaToken", "with an access token")
				: actor.method === "api_key"
					? t("viaApiKey", "with an API key")
					: actor.method === "executor"
						? t("viaRun", "from a run")
						: null;
		return (
			<span className="inline-flex items-center gap-1.5">
				<UserInlineTag userId={actor.userId} />
				{via && <span className="text-xs text-muted-foreground">{via}</span>}
			</span>
		);
	}

	const Icon = actor.kind === "apiKey" ? KeyRound : Cpu;
	const label =
		actor.kind === "apiKey"
			? t("actorApiKey", "API key")
			: actor.kind === "executor"
				? t("actorExecutor", "Executor")
				: actor.kind === "technicalUser"
					? t("actorTechnicalUser", "Technical user")
					: t("actorSystem", "System");
	return (
		<span
			className="inline-flex items-center gap-1 text-sm font-medium"
			title={actor.raw}
		>
			<Icon className="h-3.5 w-3.5 text-muted-foreground" />
			{label}
			{actor.kind !== "apiKey" &&
				actor.raw.toLowerCase() !== label.toLowerCase() && (
					<code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px] font-normal text-muted-foreground">
						{actor.raw}
					</code>
				)}
		</span>
	);
}

function DetailChips({ details }: Readonly<{ details: unknown }>) {
	const entries = useMemo(() => auditDetailEntries(details), [details]);
	if (entries.length === 0) return null;
	return (
		<div className="mt-2 flex flex-wrap gap-1">
			{entries.map(([key, value]) => (
				<code
					key={key}
					title={`${key}: ${value}`}
					className="max-w-64 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
				>
					{key}: <span className="text-foreground">{value}</span>
				</code>
			))}
		</div>
	);
}

function AuditRecordRow({
	record,
	sentence,
}: Readonly<{ record: IAuditRecordView; sentence: string }>) {
	return (
		<li className="rounded-lg border bg-card/50 p-3">
			<div className="flex flex-wrap items-center gap-x-2 gap-y-1">
				<AuditActorLabel record={record} />
				<span className="text-sm">{sentence}</span>
				<span className="ml-auto inline-flex items-center gap-2">
					<RecordStatusBadge status={record.status} />
					<RelativeTime
						value={record.timestamp_ms}
						className="text-xs text-muted-foreground"
					/>
				</span>
			</div>
			<div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
				<Badge variant="outline" className="font-mono text-[10px]">
					{record.action}
				</Badge>
				<span className="inline-flex min-w-0 items-center gap-1">
					<Box className="h-3 w-3 shrink-0" />
					{record.resource_type}
					<code className="max-w-56 truncate rounded bg-muted px-1 py-0.5 font-mono">
						{record.resource_id}
					</code>
				</span>
				{record.actor_ip && (
					<span className="inline-flex items-center gap-1 font-mono">
						<Globe className="h-3 w-3" />
						{record.actor_ip}
					</span>
				)}
				{record.ip_redacted && <RedactedMarker kind="ip" />}
				{record.details_redacted && <RedactedMarker kind="details" />}
			</div>
			<DetailChips details={record.details} />
		</li>
	);
}

function RecordFilters({
	filters,
	onChange,
}: Readonly<{
	filters: IAuditRecordFilters;
	onChange: (filters: IAuditRecordFilters) => void;
}>) {
	const { t } = useTranslation("audit");
	return (
		<div className="flex flex-wrap gap-2">
			<div className="relative min-w-48 flex-1">
				<Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
				<Input
					value={filters.action ?? ""}
					onChange={(event) =>
						onChange({ ...filters, action: event.target.value })
					}
					placeholder={t("filterAction", "Action, e.g. membership.*")}
					aria-label={t("filterActionLabel", "Filter by action")}
					className="h-8 pl-8 font-mono text-xs"
				/>
			</div>
			<Input
				value={filters.resource_id ?? ""}
				onChange={(event) =>
					onChange({ ...filters, resource_id: event.target.value })
				}
				placeholder={t("filterResource", "Resource id")}
				aria-label={t("filterResourceLabel", "Filter by resource id")}
				className="h-8 min-w-40 flex-1 font-mono text-xs"
			/>
		</div>
	);
}

function LoadOlder({
	emptyPeriod,
	loading,
	error,
	onLoad,
}: Readonly<{
	emptyPeriod: boolean;
	loading: boolean;
	error: unknown;
	onLoad: () => void;
}>) {
	const { t } = useTranslation("audit");
	return (
		<div
			className={cn(
				"flex flex-col items-center gap-2",
				emptyPeriod &&
					"rounded-lg border border-dashed py-6 text-center text-sm text-muted-foreground",
			)}
		>
			{emptyPeriod && (
				<p>{t("noRecordsInPeriod", "No matching records in this period.")}</p>
			)}
			<div className="flex flex-wrap items-center justify-center gap-2">
				<Button variant="outline" size="sm" onClick={onLoad} disabled={loading}>
					{loading && <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />}
					{t("loadOlder", "Load older records")}
				</Button>
				{error != null && !loading && (
					<span role="alert" className="text-xs text-destructive">
						{apiErrorMessage(
							error,
							t("olderFailed", "Could not load older records."),
						)}
					</span>
				)}
			</div>
		</div>
	);
}

export interface AuditRecordListProps {
	profile: IProfile | undefined;
	chainId: string;
	emptyLabel?: string;
}

export function AuditRecordList({
	profile,
	chainId,
	emptyLabel,
}: Readonly<AuditRecordListProps>) {
	const { t } = useTranslation("audit");
	const describe = useAuditSentence();
	const [filters, setFilters] = useState<IAuditRecordFilters>({});
	const debouncedFilters = useDebounce(filters, 300);
	const records = useAuditRecords(profile, chainId, debouncedFilters);

	const pages = records.data?.pages;
	const rows = useMemo(
		() =>
			(pages ?? []).flatMap((page) =>
				page.records.map((record) => ({
					record,
					sentence: describe(record),
				})),
			),
		[pages, describe],
	);
	const lastPageEmpty =
		records.hasNextPage && pages?.[pages.length - 1]?.records.length === 0;
	const filtered = Object.values(debouncedFilters).some((value) =>
		value?.trim(),
	);

	return (
		<div className="space-y-3">
			<div className="flex items-start gap-2">
				<div className="flex-1">
					<RecordFilters filters={filters} onChange={setFilters} />
				</div>
				<Button
					variant="outline"
					size="sm"
					className="h-8"
					aria-label={t("refresh", "Refresh")}
					onClick={() => records.refetch()}
					disabled={records.isFetching}
				>
					<RefreshCw
						className={`h-3.5 w-3.5 ${records.isFetching ? "animate-spin" : ""}`}
					/>
				</Button>
			</div>

			{records.isRefetchError && !records.isFetching && (
				<p
					role="alert"
					className="flex items-center gap-1.5 text-xs text-destructive"
					title={apiErrorMessage(records.error, "") || undefined}
				>
					<ShieldAlert className="h-3.5 w-3.5 shrink-0" />
					{t(
						"recordsRefreshFailed",
						"Could not refresh. Showing the records loaded earlier.",
					)}
				</p>
			)}

			{records.isLoading ? (
				<div className="space-y-2">
					<Skeleton className="h-16 w-full" />
					<Skeleton className="h-16 w-full" />
					<Skeleton className="h-16 w-full" />
				</div>
			) : !pages && records.isError ? (
				<Alert variant="destructive">
					<ShieldAlert className="h-4 w-4" />
					<AlertDescription>
						{apiErrorMessage(
							records.error,
							t("recordsFailed", "Could not load the audit records."),
						)}
					</AlertDescription>
				</Alert>
			) : rows.length > 0 ? (
				<ol className="space-y-1.5">
					{rows.map(({ record, sentence }) => (
						<AuditRecordRow
							key={record.id}
							record={record}
							sentence={sentence}
						/>
					))}
				</ol>
			) : pages && !records.hasNextPage ? (
				<div className="rounded-lg border border-dashed py-10 text-center text-sm text-muted-foreground">
					{filtered
						? t("noMatchingRecords", "No records match these filters.")
						: (emptyLabel ?? t("noRecords", "No records in this chain yet."))}
				</div>
			) : null}

			{records.hasNextPage && (
				<LoadOlder
					emptyPeriod={lastPageEmpty}
					loading={records.isFetchingNextPage}
					error={records.isFetchNextPageError ? records.error : null}
					onLoad={() => records.fetchNextPage()}
				/>
			)}
		</div>
	);
}
