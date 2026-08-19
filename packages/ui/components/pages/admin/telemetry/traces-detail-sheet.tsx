"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Copy, Layers, Timer } from "lucide-react";
import { toast } from "sonner";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	RelativeTime,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
} from "../../../ui";
import { TraceFlamegraph } from "./flamegraph";
import { EmptyState } from "./telemetry-shared";
import { SpanStatusBadge, formatDurationMs } from "./traces-shared";
import type { ITelemetryTraceDetail } from "./types";

interface TraceDetailSheetProps {
	traceId: string | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	profile: IProfile | undefined;
}

export function TelemetryTraceDetailSheet({
	traceId,
	open,
	onOpenChange,
	profile,
}: Readonly<TraceDetailSheetProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const detail = useQuery<ITelemetryTraceDetail>({
		queryKey: ["admin", "telemetry", "traces", "detail", traceId],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			if (!traceId) throw new Error("No trace selected");
			return backend.apiState.get<ITelemetryTraceDetail>(
				profile,
				`admin/telemetry/traces/${encodeURIComponent(traceId)}`,
			);
		},
		enabled: !!profile && !!traceId && open,
	});

	const spans = detail.data?.spans ?? [];
	const errorSpans = spans.filter((span) => span.status === "error").length;
	const rootSpan = spans.find((span) => !span.parentSpanId) ?? spans[0];

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-3xl lg:max-w-5xl">
				<SheetHeader>
					<SheetTitle className="flex flex-wrap items-center gap-2 pr-8">
						<span className="truncate font-mono text-sm">
							{detail.data?.rootName ?? "Trace"}
						</span>
						{rootSpan ? <SpanStatusBadge status={rootSpan.status} /> : null}
					</SheetTitle>
					<SheetDescription className="flex items-center gap-2">
						<code className="truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]">
							{traceId ?? ""}
						</code>
						<Button
							variant="ghost"
							size="sm"
							className="h-6 px-1.5"
							onClick={() => {
								if (!traceId) return;
								navigator.clipboard.writeText(traceId).catch(() => null);
								toast.success("Trace id copied");
							}}
						>
							<Copy className="h-3 w-3" />
							<span className="sr-only">{t('copyTraceId', 'Copy trace id')}</span>
						</Button>
					</SheetDescription>
				</SheetHeader>

				<div className="space-y-4 px-4 pb-8">
					{detail.isLoading ? (
						<div className="space-y-3">
							<Skeleton className="h-16 w-full" />
							<Skeleton className="h-64 w-full" />
						</div>
					) : detail.isError ? (
						<EmptyState
							message="This trace could not be loaded — it may have been swept by retention."
							className="py-10 text-sm"
						/>
					) : (
						<>
							<div className="grid gap-2 sm:grid-cols-3">
								<div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
									<div className="flex items-center justify-between text-muted-foreground">
										<span className="text-[10px] uppercase tracking-wide">
											{t('totalDuration', 'Total duration')}
										</span>
										<Timer className="h-3.5 w-3.5" />
									</div>
									<div className="mt-0.5 text-lg font-semibold tabular-nums">
										{formatDurationMs(detail.data?.totalDurationMs ?? 0)}
									</div>
								</div>
								<div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
									<div className="flex items-center justify-between text-muted-foreground">
										<span className="text-[10px] uppercase tracking-wide">
											{t('spans', 'Spans')}
										</span>
										<Layers className="h-3.5 w-3.5" />
									</div>
									<div className="mt-0.5 text-lg font-semibold tabular-nums">
										{(detail.data?.spanCount ?? spans.length).toLocaleString()}
									</div>
								</div>
								<div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
									<div className="flex items-center justify-between text-muted-foreground">
										<span className="text-[10px] uppercase tracking-wide">
											{t('started', 'Started')}
										</span>
										{errorSpans > 0 ? (
											<Badge variant="destructive" className="text-[10px]">{t('errorspansFailing', '{{errorSpans}} failing', { errorSpans })}</Badge>
										) : null}
									</div>
									<div className="mt-0.5 text-sm">
										{rootSpan ? (
											<RelativeTime
												value={rootSpan.startedAt}
												className="text-muted-foreground"
											/>
										) : (
											<span className="text-muted-foreground">—</span>
										)}
									</div>
								</div>
							</div>

							<TraceFlamegraph spans={spans} />
						</>
					)}
				</div>
			</SheetContent>
		</Sheet>
	);
}
