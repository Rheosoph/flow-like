"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpCircle, Loader2, RefreshCw } from "lucide-react";
import { Button } from "../../ui/button";

export interface PackageWidgetUpdatesProps {
	packageName: string;
	outdated?: { widgets: number; pages: number };
	isChecking: boolean;
	checkFailed: boolean;
	isUpdating: boolean;
	disabled: boolean;
	nextVersion?: string;
	onUpdate: () => void;
	onCheck: () => void;
}

export function PackageWidgetUpdates({
	packageName,
	outdated,
	isChecking,
	checkFailed,
	isUpdating,
	disabled,
	nextVersion,
	onUpdate,
	onCheck,
}: Readonly<PackageWidgetUpdatesProps>) {
	const { t } = useTranslation("store");
	const canUpdate =
		!disabled &&
		!isUpdating &&
		((outdated?.widgets ?? 0) > 0 || checkFailed || !!nextVersion);
	const checkLabel = checkFailed
		? t("appPackageWidgetRetryCheck", "Retry check")
		: t("appPackageWidgetCheckAgain", "Check again");
	const status = isUpdating
		? t("appPackageWidgetUpdating", "Updating widgets…")
		: isChecking
			? t("appPackageWidgetChecking", "Checking widgets…")
			: checkFailed
				? t("appPackageWidgetCheckFailed", "Could not check all pages.")
				: outdated === undefined
					? t("appPackageWidgetNotChecked", "Widgets have not been checked.")
					: outdated.widgets === 0
						? t("appPackageWidgetUpToDate", "Widgets are up to date")
						: t("appPackageWidgetOutdated", "{{widgets}} on {{pages}}", {
								widgets: t("appPackageWidgetOutdatedCount", {
									defaultValue_one: "{{count}} outdated widget",
									defaultValue_other: "{{count}} outdated widgets",
									count: outdated.widgets,
								}),
								pages: t("appPackageWidgetPageCount", {
									defaultValue_one: "{{count}} page",
									defaultValue_other: "{{count}} pages",
									count: outdated.pages,
								}),
							});

	return (
		<div className="flex min-w-0 flex-col gap-2 border-t pt-3">
			<div className="flex items-start justify-between gap-2">
				<p className="text-xs text-muted-foreground" aria-live="polite">
					{status}
				</p>
				<Button
					type="button"
					size="sm"
					variant="ghost"
					className="-mt-1 -mr-1 h-auto shrink-0 gap-1 px-1 py-1 text-xs"
					onClick={onCheck}
					disabled={isChecking || isUpdating}
					aria-label={t("appPackageWidgetCheckFor", "{{action}} for {{name}}", {
						action: checkLabel,
						name: packageName,
					})}
				>
					{isChecking ? (
						<Loader2 className="size-3 animate-spin" aria-hidden="true" />
					) : (
						<RefreshCw className="size-3" aria-hidden="true" />
					)}
					{checkLabel}
				</Button>
			</div>
			{nextVersion && (
				<p className="text-xs text-muted-foreground">
					{t(
						"appPackageWidgetNextVersion",
						"Updates the package to v{{version}} first.",
						{ version: nextVersion },
					)}
				</p>
			)}
			<Button
				type="button"
				size="sm"
				variant="outline"
				className="h-auto min-h-8 w-full gap-1.5 whitespace-normal px-2 py-1.5 text-xs"
				onClick={onUpdate}
				disabled={!canUpdate}
				aria-label={t(
					"appPackageWidgetUpdateFor",
					"Update all widgets of {{name}}",
					{ name: packageName },
				)}
			>
				{isUpdating ? (
					<Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
				) : (
					<ArrowUpCircle className="size-3.5" aria-hidden="true" />
				)}
				{t("appPackageWidgetUpdateAll", "Update all widgets")}
			</Button>
			<p className="text-xs text-muted-foreground">
				{t("appPackageWidgetScope", "Across all pages in this app.")}
			</p>
		</div>
	);
}
