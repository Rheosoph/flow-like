"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BellRing, Check, Inbox } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
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
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
} from "../../../ui";
import {
	ALERTS_QUERY_KEY,
	ALERT_EVENTS_PATH,
	ALERT_HOUR_OPTIONS,
	ALERT_STATUS_OPTIONS,
	type ITelemetryAlertEvent,
	type ITelemetryAlertEventsResponse,
	alertEventAckPath,
	alertStatusLabel,
	alertStatusTone,
	formatAlertValue,
} from "./alerts-types";
import { EmptyState } from "./telemetry-shared";

const ALL = "all";

const PAGE_SIZE = 50;

export function AlertStatusBadge({ status }: { readonly status: string }) {
	const tone = alertStatusTone(status);
	return (
		<span
			className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium ${tone.chip} ${tone.text}`}
		>
			<span className={`h-1.5 w-1.5 rounded-full ${tone.dot}`} />
			{alertStatusLabel(status)}
		</span>
	);
}

function AlertEventRow({
	event,
	metric,
	onAcknowledge,
	acknowledging,
}: {
	readonly event: ITelemetryAlertEvent;
	readonly metric: string;
	readonly onAcknowledge: (id: string) => void;
	readonly acknowledging: boolean;
}) {
	return (
		<div className="flex flex-wrap items-start gap-3 border-b px-4 py-3 last:border-b-0">
			<div className="min-w-0 flex-1 space-y-1">
				<div className="flex flex-wrap items-center gap-2">
					<AlertStatusBadge status={event.status} />
					<span className="truncate text-sm font-semibold">
						{event.ruleName}
					</span>
					{event.acknowledgedAt ? (
						<Badge variant="outline" className="gap-1 text-[10px]">
							<Check className="h-3 w-3" />
							Acknowledged
						</Badge>
					) : null}
				</div>
				<div className="text-sm text-muted-foreground">{event.message}</div>
				<div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
					<span className="font-mono tabular-nums">
						{formatAlertValue(metric, event.value)}
					</span>
					{event.threshold !== null && event.threshold !== undefined ? (
						<>
							<span>vs threshold</span>
							<span className="font-mono tabular-nums">
								{formatAlertValue(metric, event.threshold)}
							</span>
						</>
					) : (
						<span>no fixed threshold</span>
					)}
					<span>·</span>
					<RelativeTime value={event.createdAt} />
					{event.acknowledgedAt ? (
						<>
							<span>·</span>
							<span>acknowledged</span>
							<RelativeTime value={event.acknowledgedAt} />
						</>
					) : null}
				</div>
			</div>
			{event.acknowledgedAt ? null : (
				<Button
					variant="outline"
					size="sm"
					onClick={() => onAcknowledge(event.id)}
					disabled={acknowledging}
				>
					<Check className="mr-1 h-3.5 w-3.5" />
					Acknowledge
				</Button>
			)}
		</div>
	);
}

interface TelemetryAlertsInboxProps {
	profile?: IProfile;
	metricByRuleId?: Record<string, string>;
}

export function TelemetryAlertsInbox({
	profile,
	metricByRuleId,
}: Readonly<TelemetryAlertsInboxProps>) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [status, setStatus] = useState<string>(ALL);
	const [hours, setHours] = useState<number>(168);

	const queryParams = useMemo(() => {
		const params = new URLSearchParams({
			hours: String(hours),
			page: "0",
			page_size: String(PAGE_SIZE),
		});
		if (status !== ALL) params.set("status", status);
		return params.toString();
	}, [hours, status]);

	const events = useQuery<ITelemetryAlertEventsResponse>({
		queryKey: [...ALERTS_QUERY_KEY, "events", queryParams],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryAlertEventsResponse>(
				profile,
				`${ALERT_EVENTS_PATH}?${queryParams}`,
			);
		},
		enabled: !!profile,
	});

	const acknowledge = useMutation({
		mutationFn: async (id: string) => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.post(profile, alertEventAckPath(id));
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ALERTS_QUERY_KEY });
			toast.success("Alert acknowledged");
		},
		onError: (error: Error) =>
			toast.error(error.message ?? "Failed to acknowledge the alert"),
	});

	const onAcknowledge = useCallback(
		(id: string) => acknowledge.mutate(id),
		[acknowledge],
	);

	const rows = events.data?.events ?? [];
	const total = events.data?.total ?? 0;
	const openCount = rows.filter(
		(event) => event.status === "triggered" && !event.acknowledgedAt,
	).length;

	return (
		<Card>
			<CardHeader className="pb-3">
				<div className="flex flex-wrap items-center justify-between gap-2">
					<CardTitle className="flex items-center gap-2 text-base">
						<Inbox className="h-4 w-4" />
						Alert inbox
					</CardTitle>
					<div className="flex flex-wrap items-center gap-2">
						<Select value={status} onValueChange={setStatus}>
							<SelectTrigger className="w-40">
								<SelectValue placeholder="Status" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={ALL}>All statuses</SelectItem>
								{ALERT_STATUS_OPTIONS.map((option) => (
									<SelectItem key={option} value={option}>
										{alertStatusLabel(option)}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						<Select
							value={String(hours)}
							onValueChange={(value) => setHours(Number.parseInt(value, 10))}
						>
							<SelectTrigger className="w-40">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								{ALERT_HOUR_OPTIONS.map((option) => (
									<SelectItem key={option.value} value={String(option.value)}>
										{option.label}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>
				</div>
				<CardDescription className="flex flex-wrap items-center gap-3">
					<span>{total.toLocaleString()} alerts in the selected window</span>
					<span className="inline-flex items-center gap-1">
						<BellRing className="h-3 w-3" />
						{openCount.toLocaleString()} unacknowledged on this page
					</span>
					{total > rows.length ? (
						<span>showing the {rows.length.toLocaleString()} most recent</span>
					) : null}
				</CardDescription>
			</CardHeader>
			<CardContent className="p-0">
				{events.isLoading ? (
					<div className="space-y-2 p-4">
						<Skeleton className="h-14 w-full" />
						<Skeleton className="h-14 w-full" />
						<Skeleton className="h-14 w-full" />
					</div>
				) : rows.length === 0 ? (
					<EmptyState
						message="No alerts fired in the selected window."
						className="m-4 py-10 text-sm"
					/>
				) : (
					<div>
						{rows.map((event) => (
							<AlertEventRow
								key={event.id}
								event={event}
								metric={metricByRuleId?.[event.ruleId] ?? ""}
								onAcknowledge={onAcknowledge}
								acknowledging={acknowledge.isPending}
							/>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}
