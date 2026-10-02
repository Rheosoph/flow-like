"use client";

import { useTranslation } from "@flow-like/locales";
import { Globe, TriangleAlert } from "lucide-react";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import type { LocalSummary } from "../../../../lib/device-management/workspace/types";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";

const KEYS: DevicesRoute = { screen: "keys" };

/** The browser may delete the keys: it refused to keep them, or it can't say. Not yet read is not a risk. */
export function keysAtRisk(persistence: LocalSummary["persistence"]): boolean {
	return persistence === "denied" || persistence === "unavailable";
}

/**
 * SPEC §3.2 item 10: web only. The desktop app says "Desktop app" in the status bar instead.
 * Below 480 px the bar keeps only its buttons (SPEC §3.8); the attention count and the status bar carry the risk.
 */
export function PlatformBadge({
	platform,
	persistence,
	className,
}: Readonly<{
	platform: LocalSummary["platform"];
	persistence: LocalSummary["persistence"];
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (platform !== "web") return null;
	if (!keysAtRisk(persistence))
		return (
			<StatusChip
				tone="outline"
				icon={Globe}
				data-platform="web"
				title={t("shell.platform.webSafe", "Web app · keys kept safely")}
				className={cx("shrink-0 @max-[720px]/devices:hidden", className)}
			>
				{t("shell.platform.web", "Web")}
			</StatusChip>
		);
	const label = t(
		"shell.platform.webRisk",
		"Web · keys can be deleted by the browser",
	);
	return (
		<a
			{...link(KEYS)}
			data-platform="web-risk"
			aria-label={label}
			title={t(
				"shell.platform.webRiskHint",
				"Web · keys can be deleted by the browser. Back them up or use the desktop app.",
			)}
			className={cx(
				"inline-flex shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring @max-[480px]/devices:hidden",
				className,
			)}
		>
			<StatusChip tone="warning" icon={TriangleAlert}>
				<span className="@max-[1280px]/devices:hidden">{label}</span>
				<span className="hidden @max-[1280px]/devices:inline @max-[720px]/devices:hidden">
					{t("shell.platform.web", "Web")}
				</span>
			</StatusChip>
		</a>
	);
}
