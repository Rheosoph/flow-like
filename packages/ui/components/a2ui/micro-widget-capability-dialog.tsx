"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ServerCrashIcon,
	ShieldEllipsisIcon,
	ShieldOffIcon,
} from "lucide-react";
import type { Ref } from "react";
import { Button } from "../ui/button";
import { Card, CardContent } from "../ui/card";
import { Skeleton } from "../ui/skeleton";
import type { ComponentProps } from "./ComponentRegistry";
import type { WidgetConsentSummary } from "./micro-widget-consent-view";
import { WidgetSourceLevelBadge } from "./micro-widget-purpose-card";

/** The viewer refused the declared part (§14.5.5). */
export function MicroWidgetBlockedCard({
	elementRef,
	reviewRef,
	widgetId,
	summary,
	onReview,
	onRunBaseline,
}: {
	widgetId: string;
	/** What the refused prompt asked for; hosts count sites, runtime addresses included when they were. */
	summary: WidgetConsentSummary;
	onReview: () => void;
	/** Omitted when the server cannot run the widget at baseline (legacy frames). */
	onRunBaseline?: () => void;
	elementRef?: ComponentProps["elementRef"];
	reviewRef?: Ref<HTMLButtonElement>;
}) {
	const { t } = useTranslation("common");
	const { hostCount, includesRuntime, level } = summary;
	return (
		<Card
			ref={elementRef}
			className="border-dashed"
			data-widget-blocked={widgetId}
		>
			<CardContent className="flex items-start gap-3 p-4 text-sm">
				<ShieldOffIcon
					aria-hidden="true"
					className="mt-0.5 size-4 shrink-0 text-muted-foreground"
				/>
				<div className="flex min-w-0 flex-1 flex-col gap-2">
					<div className="flex flex-col gap-1">
						{hostCount > 0 && level && <WidgetSourceLevelBadge level={level} />}
						<p className="font-medium">
							{hostCount > 0
								? includesRuntime
									? t(
											"widgetBlockedNetworkRuntimeTitle",
											"Blocked network access to {{count}} sites, including addresses provided while the app runs",
											{ count: hostCount },
										)
									: t(
											"widgetBlockedNetworkTitle",
											"Blocked network access to {{count}} sites",
											{ count: hostCount },
										)
								: t(
										"widgetCapabilityBlockedTitle",
										'Widget "{{widgetId}}" is blocked',
										{ widgetId },
									)}
						</p>
						<p className="text-muted-foreground">
							{hostCount > 0
								? t(
										"widgetBlockedNetworkDescription",
										'Widget "{{widgetId}}" was not loaded because you did not approve its permissions.',
										{ widgetId },
									)
								: t(
										"widgetCapabilityBlockedDescription",
										"You did not approve the browser capabilities this widget requires, so it was not loaded.",
									)}
						</p>
					</div>
					<div className="flex flex-wrap gap-2">
						<Button
							ref={reviewRef}
							type="button"
							variant="outline"
							size="sm"
							onClick={onReview}
						>
							{t("widgetPermissionsReview", "Review permissions")}
						</Button>
						{onRunBaseline && (
							<Button
								type="button"
								variant="ghost"
								size="sm"
								onClick={onRunBaseline}
							>
								{t(
									"widgetRunWithoutPermissions",
									"Run without these permissions",
								)}
							</Button>
						)}
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

/** Another widget's consent dialog is open; this one waits its turn (§14.5.7). */
export function MicroWidgetQueuedCard({
	elementRef,
	widgetId,
	height,
}: {
	widgetId: string;
	height: number;
	elementRef?: ComponentProps["elementRef"];
}) {
	const { t } = useTranslation("common");
	return (
		<Card
			ref={elementRef}
			className="relative overflow-hidden border-dashed"
			style={{ height }}
			data-widget-queued={widgetId}
		>
			<Skeleton className="absolute inset-0 rounded-none" />
			<CardContent className="relative flex items-center gap-2 p-4 text-sm text-muted-foreground">
				<ShieldEllipsisIcon aria-hidden="true" className="size-4 shrink-0" />
				<output>
					{t("widgetConsentQueued", "Waiting for your permission")}
				</output>
			</CardContent>
		</Card>
	);
}

/** The backend cannot describe the widget, and its page contract declares more than a v1 contract can. */
export function MicroWidgetUnsupportedCard({
	elementRef,
	widgetId,
	detail,
}: {
	widgetId: string;
	detail: string | null;
	elementRef?: ComponentProps["elementRef"];
}) {
	const { t } = useTranslation("common");
	return (
		<Card
			ref={elementRef}
			className="border-dashed"
			data-widget-unsupported={widgetId}
		>
			<CardContent className="flex items-start gap-3 p-4 text-sm">
				<ServerCrashIcon
					aria-hidden="true"
					className="mt-0.5 size-4 shrink-0 text-muted-foreground"
				/>
				<div className="flex min-w-0 flex-1 flex-col gap-1">
					<p className="font-medium">
						{t(
							"widgetNeedsNewerServerTitle",
							'Widget "{{widgetId}}" needs a newer server',
							{ widgetId },
						)}
					</p>
					<p className="text-muted-foreground">
						{t(
							"widgetNeedsNewerServerDescription",
							"This widget declares network access that this server cannot enforce yet. Update Flow-Like, or ask your administrator to update the server.",
						)}
					</p>
					{detail && (
						<p className="text-xs text-muted-foreground wrap-break-word">
							{detail}
						</p>
					)}
				</div>
			</CardContent>
		</Card>
	);
}
