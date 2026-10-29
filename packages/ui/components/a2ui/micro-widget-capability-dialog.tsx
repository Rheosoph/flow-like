"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpCircleIcon,
	Loader2Icon,
	RefreshCwIcon,
	ServerCrashIcon,
	ShieldEllipsisIcon,
	ShieldOffIcon,
} from "lucide-react";
import type { Ref } from "react";
import { Button } from "../ui/button";
import { Card, CardContent } from "../ui/card";
import { Skeleton } from "../ui/skeleton";
import type { ComponentProps } from "./ComponentRegistry";
import { widgetDeclinedPermissions } from "./micro-widget-network-copy";
import type { WidgetPolicy } from "./micro-widget-policy";

/**
 * Placeholder of a declined widget the server cannot run without its
 * permissions (legacy frames, §14.5.5). Allow opens the consent dialog again.
 */
export function MicroWidgetBlockedCard({
	elementRef,
	reviewRef,
	widgetId,
	policy,
	onReview,
}: {
	widgetId: string;
	/** What the viewer declined. */
	policy: WidgetPolicy;
	onReview: () => void;
	elementRef?: ComponentProps["elementRef"];
	reviewRef?: Ref<HTMLButtonElement>;
}) {
	const { t, i18n } = useTranslation("common");
	return (
		<Card
			ref={elementRef}
			className="border-dashed"
			data-widget-blocked={widgetId}
		>
			<CardContent className="flex flex-wrap items-center gap-x-3 gap-y-2 p-3 text-sm">
				<ShieldOffIcon
					aria-hidden="true"
					className="size-4 shrink-0 text-muted-foreground"
				/>
				<p className="min-w-0 flex-1">
					{widgetDeclinedPermissions(policy, t, i18n.language)}
				</p>
				<Button
					ref={reviewRef}
					type="button"
					variant="outline"
					size="sm"
					onClick={onReview}
				>
					{t("widgetPermissionAllow", "Allow…")}
				</Button>
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

/** Editors offer this on a failed widget when a different build of it is installed. */
export function MicroWidgetReloadAction({
	onReload,
	onReloadAll,
	isReloadingAll,
}: {
	onReload: () => void;
	onReloadAll?: () => void;
	isReloadingAll?: boolean;
}) {
	const { t } = useTranslation(["common", "store"]);
	return (
		<div className="mt-1 flex flex-col items-start gap-2">
			<p className="text-muted-foreground">
				{t(
					"aDifferentBuildOfThisWidgetIsInstalledReloadingKeepsItsActionsAndCarriesOverItsSettings",
					"A different build of this widget is installed. Reloading keeps its actions and carries over its settings.",
				)}
			</p>
			<div className="flex flex-wrap gap-2">
				<Button
					type="button"
					size="sm"
					variant="outline"
					data-builder-interactive=""
					disabled={isReloadingAll}
					onClick={onReload}
				>
					<RefreshCwIcon aria-hidden="true" />
					{t("reloadWidget", "Reload widget")}
				</Button>
				{onReloadAll && (
					<Button
						type="button"
						size="sm"
						variant="outline"
						data-builder-interactive=""
						disabled={isReloadingAll}
						title={t(
							"updateAllWidgetsDescription",
							"Update outdated package widgets on every page in this project.",
						)}
						onClick={onReloadAll}
					>
						{isReloadingAll ? (
							<Loader2Icon aria-hidden="true" className="animate-spin" />
						) : (
							<ArrowUpCircleIcon aria-hidden="true" />
						)}
						{t("store:appPackageWidgetUpdateAll", "Update all widgets")}
					</Button>
				)}
			</div>
		</div>
	);
}

/** The backend cannot describe the widget, and its page contract declares more than a v1 contract can. */
export function MicroWidgetUnsupportedCard({
	elementRef,
	widgetId,
	detail,
	onReload,
	onReloadAll,
	isReloadingAll,
}: {
	widgetId: string;
	detail: string | null;
	elementRef?: ComponentProps["elementRef"];
	onReload?: () => void;
	onReloadAll?: () => void;
	isReloadingAll?: boolean;
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
					{onReload && (
						<MicroWidgetReloadAction
							onReload={onReload}
							onReloadAll={onReloadAll}
							isReloadingAll={isReloadingAll}
						/>
					)}
				</div>
			</CardContent>
		</Card>
	);
}
