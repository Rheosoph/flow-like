"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Anchor,
	Archive,
	ExternalLink,
	Fingerprint,
	History,
	Hourglass,
	ShieldAlert,
	ShieldCheck,
	ShieldEllipsis,
} from "lucide-react";
import Link from "next/link";
import { type ReactNode, useCallback } from "react";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { cn } from "../../../../lib/utils";
import {
	AuditHash,
	AuditIntegrityBadge,
	chainIntegrity,
	epochIntegrity,
	isIntegrityFailure,
} from "../../../audit";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Progress,
	RelativeTime,
	Skeleton,
} from "../../../ui";
import type { IChainStatusResponse } from "./types";
import {
	anchorAlertSeconds,
	epochIntervalSeconds,
	pendingAlertSeconds,
	useChainStatus,
} from "./use-chain-status";

interface DashboardChainWidgetProps {
	profile: IProfile | undefined;
}

type WorkerHealth =
	| "healthy"
	| "behind"
	| "notAnchoring"
	| "attention"
	| "held"
	| "broken";

function workerHealth(status: IChainStatusResponse, now: number): WorkerHealth {
	if (
		isIntegrityFailure(epochIntegrity(status.epochs)) ||
		isIntegrityFailure(chainIntegrity(status.platform))
	) {
		return "broken";
	}
	if (status.held_chains > 0) return "held";
	if (status.quarantined_records > 0) return "attention";
	if (anchoringStalled(status, now)) return "notAnchoring";
	const pendingAge = pendingAgeSeconds(status, now);
	if (pendingAge != null && pendingAge > pendingAlertSeconds(status)) {
		return "behind";
	}
	return "healthy";
}

function ageSeconds(since: number | null | undefined, now: number) {
	return since ? Math.max(0, (now - since) / 1000) : null;
}

function pendingAgeSeconds(
	status: IChainStatusResponse,
	now: number,
): number | null {
	return status.pending_records === 0
		? null
		: ageSeconds(status.oldest_pending_ms, now);
}

function unanchoredAgeSeconds(
	status: IChainStatusResponse,
	now: number,
): number | null {
	return status.unanchored_seals === 0
		? null
		: ageSeconds(status.oldest_unanchored_ms, now);
}

function anchoringStalled(status: IChainStatusResponse, now: number): boolean {
	const age = unanchoredAgeSeconds(status, now);
	return age != null && age > anchorAlertSeconds(status);
}

function ageUnit(seconds: number): { value: number; unit: string } {
	if (seconds < 60) return { value: seconds, unit: "second" };
	if (seconds < 3600) return { value: seconds / 60, unit: "minute" };
	if (seconds < 86_400) return { value: seconds / 3600, unit: "hour" };
	return { value: seconds / 86_400, unit: "day" };
}

function useFormatAge() {
	const { i18n } = useTranslation("audit");
	const language = i18n.resolvedLanguage;
	return useCallback(
		(seconds: number) => {
			const { value, unit } = ageUnit(seconds);
			return new Intl.NumberFormat(language, {
				style: "unit",
				unit,
				unitDisplay: "short",
				maximumFractionDigits: 0,
			}).format(value);
		},
		[language],
	);
}

function HealthBadge({ health }: Readonly<{ health: WorkerHealth }>) {
	const { t } = useTranslation("audit");
	const variants: Record<
		WorkerHealth,
		{
			label: string;
			icon: ReactNode;
			variant: "default" | "destructive" | "outline";
			className?: string;
		}
	> = {
		broken: {
			label: t("healthBroken", "Integrity failure"),
			icon: <ShieldAlert />,
			variant: "destructive",
		},
		held: {
			label: t("healthHeld", "Chains held"),
			icon: <ShieldAlert />,
			variant: "destructive",
		},
		attention: {
			label: t("healthQuarantine", "Quarantined records"),
			icon: <ShieldAlert />,
			variant: "destructive",
		},
		notAnchoring: {
			label: t("healthNotAnchoring", "Not anchoring"),
			icon: <Anchor />,
			variant: "outline",
			className: "border-destructive/40 text-destructive",
		},
		behind: {
			label: t("healthBehind", "Worker behind"),
			icon: <ShieldEllipsis />,
			variant: "outline",
		},
		healthy: {
			label: t("healthHealthy", "Healthy"),
			icon: <ShieldCheck />,
			variant: "default",
		},
	};
	const { label, icon, variant, className } = variants[health];
	return (
		<Badge variant={variant} className={cn("gap-1 text-[10px]", className)}>
			{icon}
			{label}
		</Badge>
	);
}

function MiniStat({
	label,
	value,
	tone = "default",
	hint,
	className,
}: Readonly<{
	label: string;
	value: ReactNode;
	tone?: "default" | "bad";
	hint?: ReactNode;
	className?: string;
}>) {
	return (
		<div
			className={cn(
				"min-w-0 rounded-lg border bg-muted/40 px-3 py-2",
				tone === "bad" && "border-destructive/30",
				className,
			)}
		>
			<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
				{label}
			</div>
			<div
				className={cn(
					"truncate text-sm font-semibold tabular-nums",
					tone === "bad" && "text-destructive",
				)}
			>
				{value}
			</div>
			{hint && <p className="mt-1 text-[11px] text-muted-foreground">{hint}</p>}
		</div>
	);
}

function AgeMeter({
	label,
	age,
	threshold,
	idleLabel,
}: Readonly<{
	label: string;
	age: number | null;
	threshold: number;
	idleLabel: string;
}>) {
	const { t } = useTranslation("audit");
	const formatAge = useFormatAge();
	const overdue = age != null && age > threshold;
	return (
		<div className="space-y-1">
			<div className="flex items-center justify-between text-[11px] text-muted-foreground">
				<span>{label}</span>
				<span className={cn("tabular-nums", overdue && "text-destructive")}>
					{age == null
						? idleLabel
						: t("ageOfThreshold", "{{age}} of {{threshold}}", {
								age: formatAge(age),
								threshold: formatAge(threshold),
							})}
				</span>
			</div>
			<Progress
				value={Math.min(100, ((age ?? 0) / threshold) * 100)}
				className={cn(
					overdue &&
						"bg-destructive/20 **:data-[slot=progress-indicator]:bg-destructive",
				)}
			/>
		</div>
	);
}

function Section({
	icon,
	title,
	aside,
	children,
}: Readonly<{
	icon: ReactNode;
	title: string;
	aside?: ReactNode;
	children: ReactNode;
}>) {
	return (
		<div className="rounded-lg border bg-card/50 p-3">
			<div className="mb-2 flex items-center justify-between gap-2">
				<div className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
					{icon}
					{title}
				</div>
				{aside}
			</div>
			{children}
		</div>
	);
}

function WorkerSection({
	status,
	now,
}: Readonly<{ status: IChainStatusResponse; now: number }>) {
	const { t } = useTranslation("audit");
	const formatAge = useFormatAge();
	const stalled = anchoringStalled(status, now);
	const epochAge = ageSeconds(status.latest_epoch_at_ms, now);
	const anchorTone = stalled ? "bad" : "default";

	return (
		<Section
			icon={<Hourglass className="h-3.5 w-3.5" />}
			title={t("worker", "Audit worker")}
		>
			<div className="grid gap-2 sm:grid-cols-2">
				<MiniStat
					label={t("latestEpoch", "Latest epoch")}
					value={
						epochAge == null
							? t("none", "None yet")
							: t("ago", "{{age}} ago", { age: formatAge(epochAge) })
					}
					tone={anchorTone}
				/>
				<MiniStat
					label={t("quarantined", "Quarantined")}
					value={status.quarantined_records.toLocaleString()}
					tone={status.quarantined_records > 0 ? "bad" : "default"}
				/>
				<MiniStat
					label={t("unanchoredSeals", "Unanchored seals")}
					value={status.unanchored_seals.toLocaleString()}
					tone={anchorTone}
				/>
				<MiniStat
					label={t("pendingRecords", "Pending records")}
					value={status.pending_records.toLocaleString()}
				/>
				{status.held_chains > 0 && (
					<MiniStat
						className="sm:col-span-2"
						label={t("heldChains", "Held chains")}
						value={status.held_chains.toLocaleString()}
						tone="bad"
						hint={t(
							"heldChainsHint",
							"A seal of these chains failed its hash or MAC check. They are not signed into epochs until an operator investigates; verify each chain to find the failing seal.",
						)}
					/>
				)}
			</div>
			<div className="mt-3 space-y-3">
				<AgeMeter
					label={t("oldestPending", "Oldest pending record")}
					age={pendingAgeSeconds(status, now)}
					threshold={pendingAlertSeconds(status)}
					idleLabel={t("nothingPending", "Nothing pending")}
				/>
				<AgeMeter
					label={t("oldestUnanchored", "Oldest unanchored seal")}
					age={unanchoredAgeSeconds(status, now)}
					threshold={anchorAlertSeconds(status)}
					idleLabel={
						status.unanchored_seals > 0
							? t("unanchoredHeldOnly", "Only held chains are waiting")
							: t("nothingUnanchored", "Everything anchored")
					}
				/>
				{stalled && (
					<p className="text-[11px] text-destructive">
						{t(
							"anchoringStalledHint",
							"Seals should be signed into an epoch every {{interval}}. Check that the audit worker is running and can use its signing key.",
							{ interval: formatAge(epochIntervalSeconds(status)) },
						)}
					</p>
				)}
			</div>
		</Section>
	);
}

function TimelineSection({
	status,
}: Readonly<{ status: IChainStatusResponse }>) {
	const { t } = useTranslation("audit");
	const { epochs } = status;
	return (
		<Section
			icon={<Anchor className="h-3.5 w-3.5" />}
			title={t("epochTimeline", "Epoch timeline")}
			aside={<AuditIntegrityBadge integrity={epochIntegrity(epochs)} />}
		>
			<div className="space-y-1.5 text-xs">
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("latestEpoch", "Latest epoch")}
					</span>
					<span className="inline-flex items-center gap-2">
						<span className="font-mono">
							{epochs.latest_epoch_seq == null
								? "—"
								: `#${epochs.latest_epoch_seq}`}
						</span>
						<AuditHash value={epochs.latest_epoch_hash} />
					</span>
				</div>
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("epochsChecked", "Epochs checked")}
					</span>
					<span className="font-mono">
						{epochs.epochs_checked.toLocaleString()}
					</span>
				</div>
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("platformChain", "Platform chain")}
					</span>
					<AuditIntegrityBadge integrity={chainIntegrity(status.platform)} />
				</div>
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("signingKey", "Signing key")}
					</span>
					<span className="truncate font-mono">
						{status.signing_kid ??
							t("signingKeyElsewhere", "Not in this process")}
					</span>
				</div>
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("verifyingKeys", "Verifying keys")}
					</span>
					<span className="truncate font-mono">
						{status.verifying_kids.length > 0
							? status.verifying_kids.join(", ")
							: "—"}
					</span>
				</div>
				{epochs.problem && (
					<p className="font-mono text-destructive">{epochs.problem}</p>
				)}
			</div>
		</Section>
	);
}

function RetentionSection({
	status,
}: Readonly<{ status: IChainStatusResponse }>) {
	const { t } = useTranslation("audit");
	const archive = status.latest_archive;
	return (
		<Section
			icon={<Archive className="h-3.5 w-3.5" />}
			title={t("archiveSection", "Archive")}
		>
			<div className="space-y-1.5 text-xs">
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("latestArchive", "Latest archived month")}
					</span>
					{archive ? (
						<span className="inline-flex items-center gap-2">
							<span className="font-mono">{archive.period}</span>
							<span className="text-muted-foreground">
								{t("archiveRecords", "{{records}} records", {
									records: archive.record_count.toLocaleString(),
								})}
							</span>
							<RelativeTime
								value={archive.created_at_ms}
								className="text-muted-foreground"
							/>
						</span>
					) : (
						<span className="text-muted-foreground">
							{t("noArchive", "No archive yet")}
						</span>
					)}
				</div>
				<div className="flex items-center justify-between gap-2">
					<span className="text-muted-foreground">
						{t("legacyExport", "Legacy export")}
					</span>
					{status.legacy_entries > 0 ? (
						<span>
							{t("legacyWaiting", "{{entries}} old entries waiting", {
								entries: status.legacy_entries.toLocaleString(),
							})}
						</span>
					) : (
						<span className="text-muted-foreground">
							{t("legacyDone", "Complete")}
						</span>
					)}
				</div>
			</div>
		</Section>
	);
}

function RecentChains({ status }: Readonly<{ status: IChainStatusResponse }>) {
	const { t } = useTranslation("audit");
	const chains = status.recent_chains.slice(0, 5);
	return (
		<Section
			icon={<History className="h-3.5 w-3.5" />}
			title={t("recentChains", "Recently sealed")}
		>
			{chains.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{t("noSealedChains", "No chain has been sealed yet.")}
				</p>
			) : (
				<ul className="space-y-1">
					{chains.map((chain) => (
						<li key={chain.chain_id}>
							<Link
								href={`/admin/logs?tab=audit&chain=${encodeURIComponent(chain.chain_id)}`}
								className="flex items-center gap-2 rounded-md px-1.5 py-1 text-xs hover:bg-muted/60"
							>
								<span className="min-w-0 flex-1 truncate font-mono">
									{chain.chain_id}
								</span>
								{chain.pending > 0 && (
									<Badge variant="outline" className="px-1 text-[10px]">
										{t("pendingCount", "{{pending}} pending", {
											pending: chain.pending,
										})}
									</Badge>
								)}
								<span className="font-mono text-muted-foreground">
									{chain.latest_seal_seq == null
										? "—"
										: `#${chain.latest_seal_seq}`}
								</span>
								{chain.latest_sealed_at_ms != null && (
									<RelativeTime
										value={chain.latest_sealed_at_ms}
										className="text-muted-foreground"
									/>
								)}
							</Link>
						</li>
					))}
				</ul>
			)}
		</Section>
	);
}

export function DashboardChainWidget({
	profile,
}: Readonly<DashboardChainWidgetProps>) {
	const { t } = useTranslation("audit");
	const status = useChainStatus(profile);
	const data = status.data;
	const now = status.dataUpdatedAt || Date.now();

	if (status.isError) {
		return (
			<Card className="border-destructive/20">
				<CardHeader>
					<CardTitle className="text-base">
						{t("auditTrail", "Audit trail")}
					</CardTitle>
				</CardHeader>
				<CardContent className="flex flex-wrap items-center justify-between gap-3">
					<output className="text-sm text-muted-foreground">
						{t(
							"statusUnavailable",
							"Audit status is unavailable. Retry to check the audit worker.",
						)}
					</output>
					<Button
						size="sm"
						variant="outline"
						disabled={status.isFetching}
						onClick={() => void status.refetch()}
					>
						{t("retry", "Retry")}
					</Button>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card className="overflow-hidden">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<Fingerprint className="h-4 w-4 text-primary" />
						{t("auditTrail", "Audit trail")}
						{data && <HealthBadge health={workerHealth(data, now)} />}
					</CardTitle>
					<CardDescription>
						{t(
							"widgetDescription",
							"Records are sealed per chain, anchored in signed epochs and archived monthly.",
						)}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/logs?tab=audit">
						{t("inspect", "Inspect")}
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-3">
				{status.isLoading || !data ? (
					<div className="space-y-2">
						<Skeleton className="h-14 w-full" />
						<Skeleton className="h-28 w-full" />
						<Skeleton className="h-20 w-full" />
					</div>
				) : (
					<>
						<div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
							<MiniStat
								label={t("totalRecords", "Records")}
								value={data.total_records.toLocaleString()}
							/>
							<MiniStat
								label={t("totalSeals", "Seals")}
								value={data.total_seals.toLocaleString()}
							/>
							<MiniStat
								label={t("chains", "Chains")}
								value={data.chains.toLocaleString()}
							/>
						</div>
						<WorkerSection status={data} now={now} />
						<TimelineSection status={data} />
						<RetentionSection status={data} />
						<RecentChains status={data} />
					</>
				)}
			</CardContent>
		</Card>
	);
}
