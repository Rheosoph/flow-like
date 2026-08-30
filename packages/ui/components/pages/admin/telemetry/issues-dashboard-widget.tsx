"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Bug, ExternalLink, ServerCrash, ShieldCheck } from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
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
import { IssueLevelBadge } from "./issues-shared";
import { formatRatePercent } from "./release-health-section";
import { EmptyState, StatTile } from "./telemetry-shared";
import type {
	ITelemetryIssuesResponse,
	ITelemetryReleaseHealthResponse,
} from "./types";

const WINDOW_HOURS = 168;

interface DashboardTelemetryIssuesWidgetProps {
	profile: IProfile | undefined;
}

export function DashboardTelemetryIssuesWidget({
	profile,
}: Readonly<DashboardTelemetryIssuesWidgetProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const issues = useQuery<ITelemetryIssuesResponse>({
		queryKey: ["admin", "telemetry", "issues", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryIssuesResponse>(
				profile,
				`admin/telemetry/issues?hours=${WINDOW_HOURS}&status=unresolved&page=0&page_size=25`,
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const health = useQuery<ITelemetryReleaseHealthResponse>({
		queryKey: ["admin", "telemetry", "release-health", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryReleaseHealthResponse>(
				profile,
				`admin/telemetry/release-health?hours=${WINDOW_HOURS}`,
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const topIssues = useMemo(
		() =>
			[...(issues.data?.issues ?? [])]
				.sort((a, b) => b.eventCount - a.eventCount)
				.slice(0, 3),
		[issues.data?.issues],
	);

	const unresolved = issues.data?.total ?? 0;

	return (
		<Card className="overflow-hidden border-primary/20">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<Bug className="h-4 w-4 text-primary" />
						{t("issuesReleaseHealth", "Issues & release health")}
						<Badge variant="outline" className="text-[10px]">
							7d
						</Badge>
					</CardTitle>
					<CardDescription>
						{t(
							"groupedCrashesAndErrorsReportedByInstalls",
							"Grouped crashes and errors reported by installs",
						)}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/telemetry/issues">
						{t("openIssues", "Open Issues")}
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-2 sm:grid-cols-3">
					<StatTile
						label={t("unresolvedIssues", "Unresolved issues")}
						value={issues.isLoading ? "…" : unresolved.toLocaleString()}
						icon={<Bug className="h-4 w-4" />}
					/>
					<StatTile
						label={t("crashfreeSessions", "Crash-free sessions")}
						value={
							health.isLoading
								? "…"
								: formatRatePercent(health.data?.crashFreeSessionRate)
						}
						icon={<ShieldCheck className="h-4 w-4" />}
						hint={t("valSessions", "{{val}} sessions", {
							val: (health.data?.totalSessions ?? 0).toLocaleString(),
						})}
					/>
					<StatTile
						label={t("crashedSessions", "Crashed sessions")}
						value={
							health.isLoading
								? "…"
								: (health.data?.crashedSessions ?? 0).toLocaleString()
						}
						icon={<ServerCrash className="h-4 w-4" />}
						hint={t("valInstalls", "{{val}} installs", {
							val: (health.data?.totalInstalls ?? 0).toLocaleString(),
						})}
					/>
				</div>

				{issues.isLoading ? (
					<div className="space-y-2">
						<Skeleton className="h-10 w-full" />
						<Skeleton className="h-10 w-full" />
						<Skeleton className="h-10 w-full" />
					</div>
				) : topIssues.length === 0 ? (
					<EmptyState
						message="No unresolved issues — crash reports appear here as installs report them."
						className="py-8 text-sm"
					/>
				) : (
					<ul className="space-y-1">
						{topIssues.map((issue) => (
							<li key={issue.id}>
								<Link
									href={`/admin/telemetry/issues?issue=${encodeURIComponent(issue.id)}`}
									className="group flex items-center gap-2 rounded border bg-card/50 px-2 py-1.5 hover:bg-muted/50"
								>
									<IssueLevelBadge level={issue.level} />
									<div className="min-w-0 flex-1">
										<div className="truncate text-xs font-medium group-hover:text-primary">
											{issue.kind}
										</div>
										<div className="truncate text-[11px] text-muted-foreground">
											{issue.title}
										</div>
									</div>
									<RelativeTime
										value={issue.lastSeen}
										className="hidden text-[11px] text-muted-foreground sm:block"
									/>
									<span className="w-12 text-right text-xs tabular-nums text-muted-foreground">
										{issue.eventCount.toLocaleString()}
									</span>
								</Link>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}
