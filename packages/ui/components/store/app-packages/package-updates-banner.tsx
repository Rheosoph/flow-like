"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUpCircle } from "lucide-react";
import type { PackageUpdate } from "../../../lib/schema/wasm";
import { Button } from "../../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";

export function PackageUpdatesBanner({
	updates,
	packageNames,
	pending,
	onApply,
	onApplyAll,
}: Readonly<{
	updates: readonly PackageUpdate[];
	packageNames: ReadonlyMap<string, string>;
	pending: boolean;
	onApply: (update: PackageUpdate) => void;
	onApplyAll: () => void;
}>) {
	const { t } = useTranslation("store");
	if (updates.length === 0) return null;

	const single = updates.length === 1 ? updates[0] : undefined;
	const nameOf = (update: PackageUpdate) =>
		packageNames.get(update.packageId) ??
		update.packageName ??
		update.packageId;

	return (
		<section
			aria-live="polite"
			className="flex flex-wrap items-center gap-3 rounded-xl border border-primary/30 bg-primary/5 px-4 py-3"
		>
			<span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
				<ArrowUpCircle className="size-5" aria-hidden="true" />
			</span>
			<div className="min-w-0 flex-1">
				<p className="text-sm font-semibold">
					{single
						? t("appPackagesUpdateTitle", "{{name}} {{version}} is available", {
								name: nameOf(single),
								version: single.latestVersion,
							})
						: t("appPackagesUpdatesTitle", {
								defaultValue_one: "{{count}} update is available",
								defaultValue_other: "{{count}} updates are available",
								count: updates.length,
							})}
				</p>
				<p className="truncate text-sm text-muted-foreground">
					{single
						? t("appPackagesPinnedTo", "This app is pinned to {{version}}.", {
								version: single.currentVersion,
							})
						: updates
								.map(
									(update) =>
										`${nameOf(update)} ${update.currentVersion} → ${update.latestVersion}`,
								)
								.join(" · ")}
				</p>
			</div>
			<div className="flex items-center gap-2">
				{single?.releaseNotes && (
					<Popover>
						<PopoverTrigger asChild>
							<Button size="sm" variant="ghost" className="text-primary">
								{t("releaseNotes", "Release Notes")}
							</Button>
						</PopoverTrigger>
						<PopoverContent align="end" className="max-h-80 w-96 overflow-auto">
							<p className="mb-2 text-sm font-semibold">
								{`${nameOf(single)} ${single.latestVersion}`}
							</p>
							<p className="whitespace-pre-wrap text-sm text-muted-foreground">
								{single.releaseNotes}
							</p>
						</PopoverContent>
					</Popover>
				)}
				<Button
					size="sm"
					onClick={single ? () => onApply(single) : onApplyAll}
					disabled={pending}
				>
					{single
						? t("appPackagesUpdate", "Update")
						: t("updateAllLength", "Update all ({{length}})", {
								length: updates.length,
							})}
				</Button>
			</div>
		</section>
	);
}
