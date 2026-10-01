"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	CircleArrowUp,
	CircleDashed,
	Cloud,
	Eye,
	Globe,
	HardDrive,
	Lock,
	type LucideIcon,
	Rocket,
	UserPlus,
	WifiOff,
} from "lucide-react";
import type { ReactNode } from "react";
import { type AppCopy, appCopy } from "../copy/app-copy";
import { IdRef } from "./id-ref";
import { StatusChip } from "./status-chip";
import { type ChipTone, cx } from "./tone";

export type AppVisibilityValue = Parameters<AppCopy["visibility"]>[0];
export type AppModeValue = Parameters<AppCopy["mode"]>[0];

const VISIBILITY_ICON: Record<AppVisibilityValue, LucideIcon> = {
	Offline: WifiOff,
	Private: Lock,
	Prototype: Eye,
	PublicRequestAccess: UserPlus,
	Public: Globe,
};

export const MODE_ICON: Record<AppModeValue, LucideIcon> = {
	online: Cloud,
	offline: HardDrive,
};

const SMALL = "h-5 px-1.5 text-[11.5px]";

/** APP §7.1: outline chip with the visibility's icon; the hover explains it. */
export function VisibilityChip({
	visibility,
	className,
}: Readonly<{ visibility: AppVisibilityValue; className?: string }>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t).visibility(visibility);
	return (
		<StatusChip
			tone="outline"
			icon={VISIBILITY_ICON[visibility]}
			title={copy.tooltip}
			data-visibility={visibility}
			className={className}
		>
			{copy.label}
		</StatusChip>
	);
}

/** APP §7.2: "Runs online" / "Offline copy"; the hover carries the mode sentence. */
export function ModeChip({
	mode,
	app = "",
	className,
}: Readonly<{ mode: AppModeValue; app?: string; className?: string }>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t).mode(mode, app);
	return (
		<StatusChip
			tone="outline"
			icon={MODE_ICON[mode]}
			title={copy.sentence}
			data-mode={mode}
			className={className}
		>
			{copy.chip}
		</StatusChip>
	);
}

/**
 * APP §7.5: Newest (good) · n behind (info, never warning: an older version is
 * a choice) · Unknown · "v2.4.0 staged" (info).
 */
export function DriftChip({
	behind,
	staged,
	title,
	className,
}: Readonly<{
	/** 0 = newest, n = behind, null = unknown. */
	behind: number | null;
	/** Label of a staged version; wins over `behind`. */
	staged?: string;
	/** "Runs v1.4.0 from 29 Sept. Newest is v1.5.0 from today 13:00." */
	title?: string;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t);
	const look: {
		tone: ChipTone;
		icon: LucideIcon;
		text: string;
		hover: string;
	} = staged
		? {
				tone: "info",
				icon: Rocket,
				text: copy.staged(staged),
				hover: t(
					"view.drift.stagedTitle",
					"Staged on the device, not active yet.",
				),
			}
		: behind === null
			? {
					tone: "unknown",
					icon: CircleDashed,
					text: copy.drift(null),
					hover: t(
						"view.drift.unknownTitle",
						"The device's version isn't known: it's locked, or its status has no version.",
					),
				}
			: behind === 0
				? {
						tone: "good",
						icon: Check,
						text: copy.drift(0),
						hover: t("view.drift.newestTitle", "Runs the newest version."),
					}
				: {
						tone: "info",
						icon: CircleArrowUp,
						text: copy.drift(behind),
						hover: "",
					};
	return (
		<StatusChip
			tone={look.tone}
			icon={look.icon}
			title={title ?? (look.hover || undefined)}
			data-drift={
				staged
					? "staged"
					: behind === null
						? "unknown"
						: behind === 0
							? "newest"
							: "behind"
			}
			className={cx(SMALL, className)}
		>
			{look.text}
		</StatusChip>
	);
}

/** APP §7.5 version cell: label (mono) + short hash + drift; never the hash alone. */
export function VersionCell({
	label,
	hash,
	behind,
	staged,
	title,
	sub,
	className,
}: Readonly<{
	/** "v1.4.0"; omitted = version unknown. */
	label?: string;
	hash?: string;
	behind: number | null;
	staged?: string;
	title?: string;
	/** Second line ("v1.5.0 staged", "Uploading v1.5.0 · 14 of 26 files"). */
	sub?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	return (
		<span className={cx("inline-flex min-w-0 flex-col", className)}>
			<span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
				{label ? (
					<span className="inline-flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
						<span className="font-mono">{label}</span>
						{hash ? (
							<IdRef
								id={hash}
								copyLabel={t("view.drift.copyHash", "Copy app version hash")}
							/>
						) : null}
					</span>
				) : (
					<span className="text-muted-foreground">
						{t("view.drift.versionUnknown", "Version unknown")}
					</span>
				)}
				<DriftChip
					behind={label ? behind : null}
					staged={label ? staged : undefined}
					title={title}
				/>
			</span>
			{sub ? (
				<span className="mt-0.5 block text-xs text-muted-foreground">
					{sub}
				</span>
			) : null}
		</span>
	);
}
