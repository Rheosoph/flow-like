"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2, RefreshCw } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { reloadMicroWidgetInstance } from "../a2ui/micro-widget-reload";
import type { MicroWidgetInstanceComponent } from "../a2ui/types";
import { Button } from "../ui/button";
import {
	describeMicroWidgetReload,
	useMicroWidgetReload,
} from "./use-micro-widget-reload";

function buildLabel(version: string, hash: string | null | undefined): string {
	return hash ? `${version} · ${hash.slice(0, 8)}` : version;
}

/** Offers the installed build of a placed package widget and previews what carries over. */
export function MicroWidgetReloadNotice({
	componentId,
	micro,
}: {
	componentId: string;
	micro: MicroWidgetInstanceComponent;
}) {
	const { t } = useTranslation("flow");
	const { updateFor, reload } = useMicroWidgetReload();
	const [reloading, setReloading] = useState(false);
	const update = updateFor(micro);
	const changes = useMemo(
		() =>
			update
				? describeMicroWidgetReload(
						reloadMicroWidgetInstance(micro, update).report,
						t,
					)
				: [],
		[update, micro, t],
	);
	const onReload = useCallback(async () => {
		setReloading(true);
		try {
			await reload(componentId);
		} finally {
			setReloading(false);
		}
	}, [reload, componentId]);

	if (!update) return null;

	return (
		<div className="space-y-2 rounded-md border border-primary/30 bg-primary/5 p-3">
			<div className="flex items-start gap-2">
				<RefreshCw className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
				<div className="min-w-0 space-y-0.5">
					<p className="text-xs font-medium">
						{t(
							"aDifferentBuildOfThisWidgetIsInstalled",
							"A different build of this widget is installed",
						)}
					</p>
					<p className="truncate font-mono text-[10px] text-muted-foreground">
						{buildLabel(micro.packageVersion, micro.bundleHash)} →{" "}
						{buildLabel(update.packageVersion, update.bundleHash)}
					</p>
				</div>
			</div>
			{changes.length > 0 ? (
				<ul className="list-disc space-y-0.5 pl-4 text-[10px] text-muted-foreground">
					{changes.map((change) => (
						<li key={change} className="wrap-break-word">
							{change}
						</li>
					))}
				</ul>
			) : (
				<p className="text-[10px] text-muted-foreground">
					{t(
						"reloadingKeepsAllSettingsAndActions",
						"Reloading keeps all settings and actions.",
					)}
				</p>
			)}
			<Button
				type="button"
				size="sm"
				variant="outline"
				className="h-7 w-full text-xs"
				disabled={reloading}
				onClick={onReload}
			>
				{reloading ? (
					<Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
				) : (
					<RefreshCw className="mr-1.5 h-3.5 w-3.5" />
				)}
				{t("reloadWidget", "Reload widget")}
			</Button>
		</div>
	);
}
