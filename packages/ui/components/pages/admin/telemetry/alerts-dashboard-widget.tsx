"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { BellRing, ExternalLink, Inbox, Siren } from "lucide-react";
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
import { AlertStatusBadge } from "./alerts-inbox";
import {
	ALERTS_QUERY_KEY,
	ALERT_EVENTS_PATH,
	ALERT_RULES_PATH,
	type ITelemetryAlertEventsResponse,
	type ITelemetryAlertRulesResponse,
	formatAlertValue,
} from "./alerts-types";
import { EmptyState, StatTile } from "./telemetry-shared";

const WINDOW_HOURS = 168;
const PREVIEW_LIMIT = 3;

interface DashboardTelemetryAlertsWidgetProps {
	profile: IProfile | undefined;
}

export function DashboardTelemetryAlertsWidget({
	profile,
}: Readonly<DashboardTelemetryAlertsWidgetProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const events = useQuery<ITelemetryAlertEventsResponse>({
		queryKey: [
			...ALERTS_QUERY_KEY,
			"events",
			"dashboard",
			profile?.hub,
			profile?.id,
		],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryAlertEventsResponse>(
				profile,
				`${ALERT_EVENTS_PATH}?hours=${WINDOW_HOURS}&status=triggered&page=0&page_size=${PREVIEW_LIMIT}`,
			);
		},
		enabled: !!profile,
		staleTime: 60_000,
		refetchInterval: 60_000,
		meta: { adminDashboard: true, persist: false },
	});

	const rules = useQuery<ITelemetryAlertRulesResponse>({
		queryKey: [
			...ALERTS_QUERY_KEY,
			"rules",
			"dashboard",
			profile?.hub,
			profile?.id,
		],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryAlertRulesResponse>(
				profile,
				ALERT_RULES_PATH,
			);
		},
		enabled: !!profile,
		staleTime: 60_000,
		refetchInterval: 60_000,
		meta: { adminDashboard: true, persist: false },
	});

	const metricByRule = useMemo(() => {
		const map = new Map<string, string>();
		for (const rule of rules.data?.rules ?? []) map.set(rule.id, rule.metric);
		return map;
	}, [rules.data?.rules]);

	const enabledRules = useMemo(
		() => (rules.data?.rules ?? []).filter((rule) => rule.enabled).length,
		[rules.data?.rules],
	);

	const triggered = events.data?.events ?? [];
	const unacknowledged = events.data?.unacknowledged ?? 0;

	if (events.isError || rules.isError) {
		return (
			<Card className="border-destructive/20">
				<CardHeader>
					<CardTitle className="text-base">{t("alerts", "Alerts")}</CardTitle>
				</CardHeader>
				<CardContent className="flex flex-wrap items-center justify-between gap-3">
					<output className="text-sm text-muted-foreground">
						{t(
							"alertStatusUnavailable",
							"Alert status is unavailable. Retry to check triggered alerts.",
						)}
					</output>
					<Button
						size="sm"
						variant="outline"
						disabled={events.isFetching || rules.isFetching}
						onClick={() => {
							if (events.isError) void events.refetch();
							if (rules.isError) void rules.refetch();
						}}
					>
						{t("retry", "Retry")}
					</Button>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card className="overflow-hidden border-primary/20">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<BellRing className="h-4 w-4 text-primary" />
						{t("alerts", "Alerts")}
						<Badge variant="outline" className="text-[10px]">
							7d
						</Badge>
					</CardTitle>
					<CardDescription>
						{t(
							"thresholdAndAnomalyRulesOverAnonymousTelemetry",
							"Threshold and anomaly rules over anonymous telemetry",
						)}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/telemetry/alerts">
						{t("openAlerts", "Open Alerts")}
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-2 sm:grid-cols-3">
					<StatTile
						label={t("openAlerts2", "Open alerts")}
						value={events.isLoading ? "…" : unacknowledged.toLocaleString()}
						icon={<Siren className="h-4 w-4" />}
						hint="Triggered and unacknowledged"
					/>
					<StatTile
						label="Triggered"
						value={
							events.isLoading
								? "…"
								: (events.data?.total ?? 0).toLocaleString()
						}
						icon={<Inbox className="h-4 w-4" />}
						hint="In the last 7 days"
					/>
					<StatTile
						label={t("activeRules", "Active rules")}
						value={rules.isLoading ? "…" : enabledRules.toLocaleString()}
						icon={<BellRing className="h-4 w-4" />}
						hint={t("valConfigured", "{{val}} configured", {
							val: (rules.data?.rules.length ?? 0).toLocaleString(),
						})}
					/>
				</div>

				{events.isLoading ? (
					<div className="space-y-2">
						<Skeleton className="h-10 w-full" />
						<Skeleton className="h-10 w-full" />
						<Skeleton className="h-10 w-full" />
					</div>
				) : triggered.length === 0 ? (
					<EmptyState
						message="No alerts triggered — rules that breach their threshold or baseline appear here."
						className="py-8 text-sm"
					/>
				) : (
					<ul className="space-y-1">
						{triggered.map((event) => (
							<li key={event.id}>
								<Link
									href="/admin/telemetry/alerts"
									className="group flex items-center gap-2 rounded border bg-card/50 px-2 py-1.5 hover:bg-muted/50"
								>
									<AlertStatusBadge status={event.status} />
									<div className="min-w-0 flex-1">
										<div className="truncate text-xs font-medium group-hover:text-primary">
											{event.ruleName}
										</div>
										<div className="truncate text-[11px] text-muted-foreground">
											{event.message}
										</div>
									</div>
									<RelativeTime
										value={event.createdAt}
										className="hidden text-[11px] text-muted-foreground sm:block"
									/>
									<span className="w-16 text-right text-xs tabular-nums text-muted-foreground">
										{formatAlertValue(
											metricByRule.get(event.ruleId) ?? "",
											event.value,
										)}
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
