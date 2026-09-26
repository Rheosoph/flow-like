"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowUp } from "lucide-react";
import { type ReactNode, useId, useMemo } from "react";
import type { PackagePinState } from "../../../lib/package-license";
import { cn } from "../../../lib/utils";
import { Badge } from "../../ui/badge";

export const ELEVATED_CHIP_CLASS =
	"border-primary/35 bg-primary/10 font-normal text-primary";
export const LICENSE_WARNING_BADGE_CLASS =
	"border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400";

/** Display labels for manifest tiers and capability tags that have no store label. */
export function useAccessLabels() {
	const { t } = useTranslation("store");
	return useMemo(() => {
		const memory: Record<string, string> = {
			minimal: t("appPackagesMemoryMinimal", "Minimal · 16 MB"),
			light: t("appPackagesMemoryLight", "Light · 32 MB"),
			standard: t("appPackagesMemoryStandard", "Standard · 64 MB"),
			heavy: t("appPackagesMemoryHeavy", "Heavy · 128 MB"),
			intensive: t("appPackagesMemoryIntensive", "Intensive · 256 MB"),
			large: t("appPackagesMemoryLarge", "Large · 512 MB"),
			huge: t("appPackagesMemoryHuge", "Huge · 1 GB"),
			extreme: t("appPackagesMemoryExtreme", "Extreme · 2 GB"),
			maximum: t("appPackagesMemoryMaximum", "Maximum · 4 GB"),
		};
		const timeout: Record<string, string> = {
			quick: t("appPackagesTimeoutQuick", "Quick · 5 s"),
			standard: t("appPackagesTimeoutStandard", "Standard · 30 s"),
			extended: t("appPackagesTimeoutExtended", "Extended · 60 s"),
			long_running: t("appPackagesTimeoutLongRunning", "Long running · 5 min"),
			very_long: t("appPackagesTimeoutVeryLong", "Very long · 10 min"),
			maximum: t("appPackagesTimeoutMaximum", "Maximum · 30 min"),
		};
		const tags: Record<string, string> = {
			"net.http": t("appPackagesAccessHttp", "HTTP"),
			"net.ws": t("appPackagesAccessWebsocket", "WebSocket"),
			"net.tcp": t("appPackagesAccessTcp", "TCP"),
			"net.udp": t("appPackagesAccessUdp", "UDP"),
			"net.dns": t("appPackagesAccessDns", "DNS"),
			"storage.user": t("appPackagesAccessUserFiles", "Your files"),
			"storage.uploads": t("appPackagesAccessUploads", "Uploads"),
			"storage.cache": t("appPackagesAccessCacheDir", "Cache folder"),
			"storage.node": t("appPackagesAccessNodeStorage", "Node storage"),
		};
		return {
			memory: (tier?: string) => (tier ? (memory[tier] ?? tier) : undefined),
			timeout: (tier?: string) => (tier ? (timeout[tier] ?? tier) : undefined),
			tag: (tag: string) => tags[tag] ?? tag,
		};
	}, [t]);
}

export function SectionHeader({
	title,
	meta,
	action,
	id,
}: Readonly<{
	title: string;
	meta?: ReactNode;
	action?: ReactNode;
	id?: string;
}>) {
	return (
		<div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
			<div className="flex min-w-0 flex-wrap items-baseline gap-x-2.5 gap-y-1">
				<h2 id={id} className="text-base font-semibold">
					{title}
				</h2>
				{meta && <span className="text-sm text-muted-foreground">{meta}</span>}
			</div>
			{action}
		</div>
	);
}

export function Section({
	title,
	meta,
	action,
	children,
	className,
}: Readonly<{
	title: string;
	meta?: ReactNode;
	action?: ReactNode;
	children: ReactNode;
	className?: string;
}>) {
	const headingId = useId();
	return (
		<section
			aria-labelledby={headingId}
			className={cn("flex flex-col gap-3", className)}
		>
			<SectionHeader id={headingId} title={title} meta={meta} action={action} />
			{children}
		</section>
	);
}

export function EmptyHint({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="rounded-xl border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">
			{children}
		</p>
	);
}

/** The one state worth a chip on a package: licence trouble first, then an update. */
export function PackageStatusBadge({
	pinState,
	timeLeftLabel,
	latestVersion,
	className,
}: Readonly<{
	pinState: PackagePinState;
	timeLeftLabel?: string;
	latestVersion?: string;
	className?: string;
}>) {
	const { t } = useTranslation("store");
	switch (pinState) {
		case "expired":
			return (
				<Badge variant="destructive" className={className}>
					{t("expired", "Expired")}
				</Badge>
			);
		case "stale":
			return (
				<Badge variant="destructive" className={className}>
					{t("stale", "Stale")}
				</Badge>
			);
		case "lapsed":
			return (
				<Badge
					variant="outline"
					className={cn(LICENSE_WARNING_BADGE_CLASS, className)}
				>
					{timeLeftLabel
						? t("licenseLapsedTimeLeft", "Licence lapsed · {{timeLeft}}", {
								timeLeft: timeLeftLabel,
							})
						: t("licenseLapsed", "Licence lapsed")}
				</Badge>
			);
		default:
			if (!latestVersion) return null;
			return (
				<Badge
					variant="outline"
					className={cn(
						"gap-1 font-mono",
						ELEVATED_CHIP_CLASS,
						"bg-background/90 backdrop-blur-sm",
						className,
					)}
					title={t("appPackagesUpdateAvailableTo", "Update to {{version}}", {
						version: latestVersion,
					})}
				>
					<ArrowUp className="size-3" aria-hidden="true" />
					{latestVersion}
				</Badge>
			);
	}
}
