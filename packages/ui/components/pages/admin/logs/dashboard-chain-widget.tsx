"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	CheckCircle2,
	ExternalLink,
	Fingerprint,
	GitBranch,
	KeyRound,
	Link2,
	ShieldAlert,
	ShieldCheck,
	ShieldEllipsis,
} from "lucide-react";
import Link from "next/link";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	RelativeTime,
	Skeleton,
} from "../../../ui";
import type { IChainStatusResponse } from "./types";

interface DashboardChainWidgetProps {
	profile: IProfile | undefined;
}

function HashChip({ hash }: { hash?: string | null }) {
	const { t } = useTranslation("admin");
	if (!hash) return null;
	const head = hash.slice(0, 8);
	const tail = hash.slice(-6);
	return (
		<code
			title={hash}
			className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
		>{`${head}…${tail}`}</code>
	);
}

function ChainPulse({
	signed,
	valid,
	hasEntries,
}: {
	signed: boolean;
	valid?: boolean | null;
	hasEntries: boolean;
}) {
	const { t } = useTranslation("admin");
	if (!hasEntries) {
		return (
			<span
				className="inline-flex h-2 w-2 rounded-full bg-muted"
				title="empty"
			/>
		);
	}
	if (valid === false) {
		return (
			<span
				className="inline-flex h-2 w-2 animate-pulse rounded-full bg-destructive shadow-[0_0_8px] shadow-destructive"
				title={t('chainBroken', 'chain broken')}
			/>
		);
	}
	if (valid === true && signed) {
		return (
			<span
				className="inline-flex h-2 w-2 rounded-full bg-emerald-500 shadow-[0_0_8px] shadow-emerald-500/60"
				title={t('signedVerified', 'signed & verified')}
			/>
		);
	}
	if (signed) {
		return (
			<span
				className="inline-flex h-2 w-2 rounded-full bg-sky-500 shadow-[0_0_8px] shadow-sky-500/60"
				title="signed"
			/>
		);
	}
	return (
		<span
			className="inline-flex h-2 w-2 rounded-full bg-amber-500 shadow-[0_0_8px] shadow-amber-500/60"
			title="unsigned"
		/>
	);
}

export function DashboardChainWidget({ profile }: DashboardChainWidgetProps) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const status = useQuery<IChainStatusResponse>({
		queryKey: ["admin", "logs", "chain-status"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IChainStatusResponse>(
				profile,
				"admin/logs/chain-status",
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const data = status.data;
	const root = data?.root_chain;
	const signedRatio =
		data && data.total_entries > 0
			? Math.round((data.signed_entries / data.total_entries) * 100)
			: null;

	let healthBadge: { tone: string; label: string; icon: React.ReactNode };
	if (!data) {
		healthBadge = { tone: "muted", label: "—", icon: <ShieldEllipsis /> };
	} else if (!data.signing_configured) {
		healthBadge = {
			tone: "warn",
			label: t('unsigned', 'Unsigned'),
			icon: <ShieldAlert className="h-4 w-4" />,
		};
	} else if (root?.valid === false) {
		healthBadge = {
			tone: "bad",
			label: t('broken', 'Broken'),
			icon: <ShieldAlert className="h-4 w-4" />,
		};
	} else if (root?.valid === true) {
		healthBadge = {
			tone: "good",
			label: t('verified', 'Verified'),
			icon: <ShieldCheck className="h-4 w-4" />,
		};
	} else {
		healthBadge = {
			tone: "muted",
			label: t('idle', 'Idle'),
			icon: <ShieldEllipsis className="h-4 w-4" />,
		};
	}

	return (
		<Card className="overflow-hidden">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<Fingerprint className="h-4 w-4 text-emerald-500" />
						{t('cryptographicLogs', 'Cryptographic Logs')}
						<Badge
							variant={
								healthBadge.tone === "good"
									? "default"
									: healthBadge.tone === "bad"
										? "destructive"
										: "outline"
							}
							className="gap-1 text-[10px]"
						>
							{healthBadge.icon}
							{healthBadge.label}
						</Badge>
					</CardTitle>
					<CardDescription>
						{t('hashchainAuditTrailWithEs256ServerSignatures', 'Hash-chain audit trail with ES256 server signatures')}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/logs?tab=audit">
						{t('inspectChain', 'Inspect chain')}
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-2 sm:grid-cols-4">
					<MiniStat
						label="Entries"
						value={
							status.isLoading
								? "…"
								: (data?.total_entries ?? 0).toLocaleString()
						}
						icon={<Link2 className="h-4 w-4" />}
					/>
					<MiniStat
						label={t('last24h', 'Last 24h')}
						value={
							status.isLoading
								? "…"
								: (data?.last_24h_entries ?? 0).toLocaleString()
						}
						icon={<CheckCircle2 className="h-4 w-4" />}
					/>
					<MiniStat
						label="Branches"
						value={
							status.isLoading
								? "…"
								: (data?.branch_chain_count ?? 0).toLocaleString()
						}
						icon={<GitBranch className="h-4 w-4" />}
					/>
					<MiniStat
						label="KID"
						value={status.isLoading ? "…" : (data?.current_kid ?? "—")}
						icon={<KeyRound className="h-4 w-4" />}
						mono
					/>
				</div>

				<div className="rounded-lg border bg-card/50 p-3">
					<div className="mb-2 flex items-center justify-between">
						<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
							{t('signatureCoverage', 'Signature coverage')}
						</div>
						<div className="text-xs tabular-nums text-muted-foreground">
							{signedRatio == null ? "—" : `${signedRatio}%`}
						</div>
					</div>
					<div className="h-2 overflow-hidden rounded-full bg-muted">
						<div
							className="h-full rounded-full bg-linear-to-r from-emerald-400 to-emerald-600"
							style={{ width: `${signedRatio ?? 0}%` }}
						/>
					</div>
					<div className="mt-2 grid grid-cols-2 text-[11px] text-muted-foreground">
						<div className="flex items-center gap-1"><Trans i18nKey="spanClassnameinlineblockH2W2RoundedfullBgemerald500Signed"><span className="inline-block h-2 w-2 rounded-full bg-emerald-500" />
							Signed:</Trans>{data?.signed_entries.toLocaleString() ?? "0"}
						</div>
						<div className="flex items-center gap-1 justify-end"><Trans i18nKey="spanClassnameinlineblockH2W2RoundedfullBgamber500Unsigned"><span className="inline-block h-2 w-2 rounded-full bg-amber-500" />
							Unsigned:</Trans>{data?.unsigned_entries.toLocaleString() ?? "0"}
						</div>
					</div>
				</div>

				<div className="rounded-lg border bg-card/50 p-3">
					<div className="mb-2 flex items-center justify-between">
						<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
							{t('rootChain', 'Root chain')}
						</div>
						<div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
							<ChainPulse
								signed={Boolean(root?.signed)}
								valid={root?.valid ?? null}
								hasEntries={(root?.entries ?? 0) > 0}
							/>
							{root?.last_entry_at ? (
								<RelativeTime value={root.last_entry_at} />
							) : (
								<span>{t('noEntries', 'no entries')}</span>
							)}
						</div>
					</div>
					{status.isLoading ? (
						<Skeleton className="h-12 w-full" />
					) : (
						<div className="flex items-center justify-between gap-3 text-xs">
							<div className="space-y-1">
								<div className="text-muted-foreground">
									{t('sequence', 'Sequence')}{" "}
									<span className="font-mono text-foreground">
										#{root?.last_sequence ?? 0}
									</span>
								</div>
								<div className="flex items-center gap-2">
									<span className="text-muted-foreground">{t('tail', 'Tail')}</span>
									<HashChip hash={root?.last_entry_hash} />
								</div>
							</div>
							<Badge
								variant={
									root?.valid === false
										? "destructive"
										: root?.valid === true
											? "default"
											: "outline"
								}
							>
								{root?.valid === false
									? "Broken"
									: root?.valid === true
										? "Chain valid"
										: "Idle"}
							</Badge>
						</div>
					)}
				</div>

				<div>
					<div className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
						{t('recentBranchChains', 'Recent branch chains')}
					</div>
					{status.isLoading ? (
						<div className="space-y-1.5">
							<Skeleton className="h-7 w-full" />
							<Skeleton className="h-7 w-full" />
							<Skeleton className="h-7 w-full" />
						</div>
					) : (data?.recent_branches.length ?? 0) === 0 ? (
						<div className="rounded-md border border-dashed px-3 py-3 text-xs text-muted-foreground">
							{t('onlyTheRootChainIsActive', 'Only the root chain is active.')}
						</div>
					) : (
						<ul className="space-y-1">
							{data?.recent_branches.map((b) => (
								<li
									key={b.chain_id ?? b.label}
									className="flex items-center gap-2 rounded-md border border-border/50 bg-background/50 px-2 py-1.5"
								>
									<ChainPulse
										signed={b.signed}
										valid={b.valid ?? null}
										hasEntries={b.entries > 0}
									/>
									<span className="truncate text-xs font-medium">
										{b.label}
									</span>
									<span className="ml-auto inline-flex items-center gap-2 text-[11px] text-muted-foreground">
										<span className="font-mono">#{b.last_sequence ?? 0}</span>
										{b.last_entry_at && (
											<RelativeTime value={b.last_entry_at} />
										)}
									</span>
								</li>
							))}
						</ul>
					)}
				</div>
			</CardContent>
		</Card>
	);
}

function MiniStat({
	label,
	value,
	icon,
	mono = false,
}: {
	label: string;
	value: string;
	icon: React.ReactNode;
	mono?: boolean;
}) {
	return (
		<div className="flex items-center gap-2 rounded-lg border bg-muted/40 px-3 py-2">
			<div className="text-muted-foreground">{icon}</div>
			<div className="min-w-0 flex-1">
				<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
					{label}
				</div>
				<div
					className={`truncate text-sm font-semibold ${mono ? "font-mono" : ""}`}
					title={value}
				>
					{value}
				</div>
			</div>
		</div>
	);
}
