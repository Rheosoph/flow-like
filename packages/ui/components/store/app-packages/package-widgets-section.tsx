"use client";

import { useTranslation } from "@flow-like/locales";
import type { AppPackageWidget } from "../../../lib/package-widgets";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { WidgetCard } from "../widget-card";
import { EmptyHint, Section } from "./parts";

const SKELETON_KEYS = ["a", "b", "c"] as const;

export function PackageWidgetsSection({
	widgets,
	loading,
	limit,
	onShowAll,
}: Readonly<{
	widgets: readonly AppPackageWidget[];
	loading: boolean;
	limit?: number;
	onShowAll?: () => void;
}>) {
	const { t } = useTranslation("store");
	const visible = limit ? widgets.slice(0, limit) : widgets;
	const hidden = widgets.length - visible.length;

	return (
		<Section
			title={t("appPackagesWidgets", "Widgets")}
			meta={
				widgets.length > 0
					? t(
							"appPackagesWidgetsMeta",
							"Add them to a page from the widget library",
						)
					: undefined
			}
			action={
				hidden > 0 && onShowAll ? (
					<Button
						variant="link"
						size="sm"
						className="h-auto p-0"
						onClick={onShowAll}
					>
						{t("appPackagesShowAll", "Show all ({{total}})", {
							total: widgets.length,
						})}
					</Button>
				) : undefined
			}
		>
			{loading && widgets.length === 0 ? (
				<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
					{SKELETON_KEYS.map((key) => (
						<Skeleton key={key} className="h-64 rounded-xl" />
					))}
				</div>
			) : widgets.length === 0 ? (
				<EmptyHint>
					{t("appPackagesNoWidgets", "None of these packages ship widgets.")}
				</EmptyHint>
			) : (
				<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
					{visible.map((entry) => (
						<WidgetCard
							key={`${entry.packageId}/${entry.widget.id}`}
							widget={entry.widget}
							packageId={entry.packageId}
							packageVersion={entry.packageVersion}
							bundleHash={entry.bundleHash}
							packageName={entry.packageName}
						/>
					))}
				</div>
			)}
		</Section>
	);
}
