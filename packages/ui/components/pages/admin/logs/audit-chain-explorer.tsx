"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Box,
	CheckCircle2,
	Cpu,
	Fingerprint,
	GitBranch,
	KeyRound,
	Link2,
	RefreshCw,
	Shield,
	ShieldAlert,
	ShieldCheck,
	ShieldEllipsis,
} from "lucide-react";
import { useMemo, useState } from "react";
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
	Separator,
	Skeleton,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../../ui";
import type { IChainStatusResponse } from "./types";
import { UserPill } from "./user-pill";

interface AuditChainExplorerProps {
	profile: IProfile | undefined;
}

interface IAuditEntry {
	id: string;
	sequence: number;
	timestamp: string;
	actor_id: string;
	actor_type: string;
	action: string;
	resource_type: string;
	resource_id: string;
	chain_id?: string | null;
	summary: string;
	details?: unknown;
	entry_hash: string;
	prev_hash: string;
	signature?: string | null;
	kid?: string | null;
}

function HashChip({ hash }: { hash?: string | null }) {
	if (!hash) return null;
	return (
		<TooltipProvider>
			<Tooltip>
				<TooltipTrigger asChild>
					<code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[10px]">
						{hash.slice(0, 8)}…{hash.slice(-6)}
					</code>
				</TooltipTrigger>
				<TooltipContent>
					<code className="font-mono text-[11px] break-all">{hash}</code>
				</TooltipContent>
			</Tooltip>
		</TooltipProvider>
	);
}

function ChainHealthBadge({
	signed,
	valid,
}: {
	signed: boolean;
	valid?: boolean | null;
}) {
	const { t } = useTranslation("admin");
	if (valid === false) {
		return (
			<Badge variant="destructive" className="gap-1">
				<ShieldAlert className="h-3 w-3" /> {t('broken', 'Broken')}
			</Badge>
		);
	}
	if (valid === true && signed) {
		return (
			<Badge className="gap-1 bg-emerald-500 text-white hover:bg-emerald-500/90">
				<ShieldCheck className="h-3 w-3" /> {t('verified', 'Verified')}
			</Badge>
		);
	}
	if (signed) {
		return (
			<Badge variant="secondary" className="gap-1">
				<Shield className="h-3 w-3" /> {t('signed', 'Signed')}
			</Badge>
		);
	}
	return (
		<Badge variant="outline" className="gap-1">
			<ShieldEllipsis className="h-3 w-3" /> {t('unsigned', 'Unsigned')}
		</Badge>
	);
}

export function AuditChainExplorer({
	profile,
}: Readonly<AuditChainExplorerProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const [selected, setSelected] = useState<string | null>(null);

	const status = useQuery<IChainStatusResponse>({
		queryKey: ["admin", "logs", "chain-status", "explorer"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IChainStatusResponse>(
				profile,
				"admin/logs/chain-status",
			);
		},
		enabled: !!profile,
	});

	const activeChainId = selected ?? null;

	const entries = useQuery<IAuditEntry[]>({
		queryKey: ["admin", "logs", "audit", "entries", activeChainId],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const qs = new URLSearchParams();
			if (activeChainId) qs.set("chain_id", activeChainId);
			qs.set("limit", "50");
			return backend.apiState.get<IAuditEntry[]>(
				profile,
				`audit/entries?${qs.toString()}`,
			);
		},
		enabled: !!profile,
	});

	const chainOptions = useMemo(() => {
		if (!status.data) return [];
		const root = {
			chain_id: null as string | null,
			label: status.data.root_chain.label,
			signed: status.data.root_chain.signed,
			valid: status.data.root_chain.valid,
			entries: status.data.root_chain.entries,
		};
		return [
			root,
			...status.data.recent_branches.map((b) => ({
				chain_id: b.chain_id ?? null,
				label: b.label,
				signed: b.signed,
				valid: b.valid,
				entries: b.entries,
			})),
		];
	}, [status.data]);

	const signedRatio =
		status.data && status.data.total_entries > 0
			? Math.round(
					(status.data.signed_entries / status.data.total_entries) * 100,
				)
			: 0;

	return (
		<div className="space-y-4">
			<div className="grid gap-4 md:grid-cols-4">
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-1">
						<CardTitle className="text-sm">{t('totalEntries', 'Total entries')}</CardTitle>
						<Link2 className="h-4 w-4 text-muted-foreground" />
					</CardHeader>
					<CardContent>
						{status.isLoading ? (
							<Skeleton className="h-8 w-20" />
						) : (
							<div className="text-2xl font-bold tabular-nums">
								{status.data?.total_entries.toLocaleString() ?? "0"}
							</div>
						)}
						<p className="mt-1 text-xs text-muted-foreground">
							{status.data?.last_24h_entries.toLocaleString() ?? "0"} {t('inLast24h', "in last 24h")}
						</p>
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-1">
						<CardTitle className="text-sm">{t('signedCoverage', 'Signed coverage')}</CardTitle>
						<CheckCircle2 className="h-4 w-4 text-emerald-500" />
					</CardHeader>
					<CardContent>
						{status.isLoading ? (
							<Skeleton className="h-8 w-20" />
						) : (
							<div className="text-2xl font-bold tabular-nums">{`${signedRatio}%`}</div>
						)}
						<div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
							<div
								className="h-full bg-linear-to-r from-emerald-400 to-emerald-600"
								style={{ width: `${signedRatio}%` }}
							/>
						</div>
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-1">
						<CardTitle className="text-sm">{t('branches', 'Branches')}</CardTitle>
						<GitBranch className="h-4 w-4 text-muted-foreground" />
					</CardHeader>
					<CardContent>
						{status.isLoading ? (
							<Skeleton className="h-8 w-20" />
						) : (
							<div className="text-2xl font-bold tabular-nums">
								{status.data?.branch_chain_count.toLocaleString() ?? "0"}
							</div>
						)}
						<p className="mt-1 text-xs text-muted-foreground">
							{t('appPackageSubchains', 'App + package sub-chains')}
						</p>
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-1">
						<CardTitle className="text-sm">{t('activeKey', 'Active key')}</CardTitle>
						<KeyRound className="h-4 w-4 text-muted-foreground" />
					</CardHeader>
					<CardContent>
						{status.isLoading ? (
							<Skeleton className="h-8 w-20" />
						) : (
							<div
								className="truncate font-mono text-base font-semibold"
								title={status.data?.current_kid}
							>
								{status.data?.current_kid ?? "—"}
							</div>
						)}
						<p className="mt-1 text-xs text-muted-foreground">
							{status.data?.signing_configured
								? t('signingEnabled', 'Signing enabled')
								: t('signingNotConfigured', 'Signing not configured')}
						</p>
					</CardContent>
				</Card>
			</div>

			<div className="grid gap-4 lg:grid-cols-[260px_1fr]">
				<Card className="overflow-hidden">
					<CardHeader className="pb-3">
						<CardTitle className="flex items-center gap-2 text-base">
							<Fingerprint className="h-4 w-4 text-emerald-500" />
							{t('chains', 'Chains')}
						</CardTitle>
						<CardDescription>{t('pickAChainToInspectEntries', 'Pick a chain to inspect entries')}</CardDescription>
					</CardHeader>
					<CardContent className="space-y-1.5">
						{status.isLoading ? (
							<>
								<Skeleton className="h-9 w-full" />
								<Skeleton className="h-9 w-full" />
								<Skeleton className="h-9 w-full" />
							</>
						) : (
							chainOptions.map((c) => {
								const isActive = (selected ?? null) === (c.chain_id ?? null);
								return (
									<button
										key={c.chain_id ?? "__root__"}
										type="button"
										onClick={() => setSelected(c.chain_id ?? null)}
										className={`w-full rounded-md border px-2 py-1.5 text-left text-xs transition-colors ${
											isActive
												? "border-primary/40 bg-primary/5"
												: "hover:bg-muted/60"
										}`}
									>
										<div className="flex items-center justify-between gap-1">
											<span className="truncate font-medium">{c.label}</span>
											<Badge
												variant="outline"
												className="ml-2 shrink-0 text-[10px]"
											>
												{c.entries}
											</Badge>
										</div>
										<div className="mt-1 flex items-center gap-1.5 text-[11px] text-muted-foreground">
											<ChainHealthBadge
												signed={c.signed}
												valid={c.valid ?? null}
											/>
										</div>
									</button>
								);
							})
						)}
					</CardContent>
				</Card>

				<Card className="overflow-hidden">
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-3">
						<div className="space-y-1">
							<CardTitle className="text-base">{t('chainEntries', 'Chain entries')}</CardTitle>
							<CardDescription>
								{activeChainId
									? t('branchChainActivechainid', 'Branch chain {{activeChainId}}', { activeChainId })
									: t('platformRootChain', 'Platform root chain')}
							</CardDescription>
						</div>
						<Button
							variant="outline"
							size="sm"
							onClick={() => entries.refetch()}
							disabled={entries.isFetching}
						>
							<RefreshCw
								className={`h-3.5 w-3.5 ${entries.isFetching ? "animate-spin" : ""}`}
							/>
						</Button>
					</CardHeader>
					<CardContent>
						{entries.isLoading ? (
							<div className="space-y-2">
								<Skeleton className="h-12 w-full" />
								<Skeleton className="h-12 w-full" />
								<Skeleton className="h-12 w-full" />
							</div>
						) : entries.data && entries.data.length > 0 ? (
							<ol className="relative space-y-1.5 border-l border-border/60 pl-4">
								{entries.data.map((e) => (
									<li
										key={e.id}
										className="relative rounded-lg border bg-background/50 p-3"
									>
										<span className="absolute -left-1.75 top-4 h-3 w-3 rounded-full border-2 border-background bg-emerald-500 shadow" />
										<div className="flex flex-wrap items-center gap-2">
											<Badge
												variant="outline"
												className="font-mono text-[10px]"
											>{`#${e.sequence}`}</Badge>
											<Badge
												variant="secondary"
												className="font-mono text-[10px]"
											>
												{e.action}
											</Badge>
											{e.signature ? (
												<Badge className="gap-1 bg-emerald-500 text-white hover:bg-emerald-500/90">
													<Shield className="h-3 w-3" /> {t('signed', 'Signed')}
												</Badge>
											) : (
												<Badge variant="outline" className="gap-1">
													<ShieldEllipsis className="h-3 w-3" /> {t('unsigned', 'Unsigned')}
												</Badge>
											)}
											<RelativeTime
												value={e.timestamp}
												className="ml-auto text-[11px] text-muted-foreground"
											/>
										</div>
										<div className="mt-2 text-sm">{e.summary}</div>
										<div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
											<span className="inline-flex items-center gap-1">
												<Cpu className="h-3 w-3" />
												{e.actor_type}
											</span>
											{e.actor_type === "USER" ? (
												<UserPill userId={e.actor_id} compact muted />
											) : (
												<code className="rounded bg-muted px-1.5 py-0.5 font-mono">
													{e.actor_id}
												</code>
											)}
											<span className="inline-flex items-center gap-1">
												<Box className="h-3 w-3" />{`${e.resource_type}:${e.resource_id}`}</span>
										</div>
										<Separator className="my-2" />
										<div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
											<span className="inline-flex items-center gap-1">
												prev <HashChip hash={e.prev_hash} />
											</span>
											<span className="inline-flex items-center gap-1">
												hash <HashChip hash={e.entry_hash} />
											</span>
											{e.kid && (
												<span className="inline-flex items-center gap-1">
													<KeyRound className="h-3 w-3" /> {e.kid}
												</span>
											)}
										</div>
									</li>
								))}
							</ol>
						) : (
							<div className="rounded-lg border border-dashed py-10 text-center text-sm text-muted-foreground">
								{t('noEntriesInThisChain', 'No entries in this chain.')}
							</div>
						)}
					</CardContent>
				</Card>
			</div>
		</div>
	);
}
