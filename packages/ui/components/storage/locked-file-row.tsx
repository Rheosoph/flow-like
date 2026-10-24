"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarClockIcon,
	ChevronDownIcon,
	ChevronRightIcon,
	DownloadIcon,
	EllipsisVerticalIcon,
	GavelIcon,
	HistoryIcon,
	LockIcon,
	UploadIcon,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { humanFileSize } from "../../lib/utils";
import type { ILockedItem } from "../../state/backend-state/locked-storage-state";
import {
	Badge,
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui";
import { retentionLabel } from "./locked-storage-entries";

const BADGE_CLASS = "text-xs px-1 py-0 h-4 sm:h-5 sm:px-2 sm:py-1";

export interface ILockedFileRowActions {
	onOpen: (item: ILockedItem) => void;
	onDownload: (item: ILockedItem) => void;
	onShowVersions: (item: ILockedItem) => void;
	onUploadVersion?: (item: ILockedItem) => void;
	onExtendRetention?: (item: ILockedItem) => void;
	onToggleLegalHold?: (item: ILockedItem) => void;
	/** Renders the version list shown when the row is expanded. */
	renderVersions?: (item: ILockedItem) => ReactNode;
}

export function LockedFileRow({
	item,
	highlight,
	actions,
}: Readonly<{
	item: ILockedItem;
	highlight: boolean;
	actions: ILockedFileRowActions;
}>) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const [expanded, setExpanded] = useState(false);
	const latest = item.latest;
	const legalHold = !!latest?.legal_hold;
	const lockedVersion = latest?.status === "locked" ? latest : null;
	const ToggleIcon = expanded ? ChevronDownIcon : ChevronRightIcon;

	return (
		<div
			className={`group relative rounded-lg border border-border/50 p-3 w-full transition-all duration-200 bg-linear-to-r from-background to-muted/10 hover:border-primary/50 hover:shadow-md cursor-pointer ${
				highlight ? "border-primary bg-primary/5 shadow-sm" : ""
			}`}
			data-locked-item={item.id}
		>
			<div className="w-full flex flex-row justify-between items-center">
				<button
					type="button"
					className="flex flex-row items-center gap-3 flex-1 min-w-0"
					onClick={() => actions.onOpen(item)}
				>
					<div className="p-2 rounded-md bg-primary/10 group-hover:bg-primary/20 transition-colors">
						<LockIcon className="w-5 h-5 text-primary" />
					</div>
					<div className="flex flex-col items-start flex-1 min-w-0">
						<p className="line-clamp-1 text-start font-medium text-foreground truncate w-full text-sm sm:text-base">
							{item.name}
						</p>
						<div className="flex flex-wrap items-center gap-1 sm:gap-2 mt-1">
							<Badge variant="secondary" className={BADGE_CLASS}>
								{t("lockedStorage.versionBadge", "v{{version}}", {
									version: latest?.version ?? item.latest_version,
								})}
							</Badge>
							<Badge variant="outline" className={BADGE_CLASS}>
								{humanFileSize(item.live_size, true)}
							</Badge>
							{latest && (
								<span className="text-xs text-muted-foreground">
									{retentionLabel(latest, language, t)}
								</span>
							)}
							{legalHold && (
								<Badge variant="destructive" className={BADGE_CLASS}>
									<GavelIcon className="h-3 w-3 mr-1" />
									{t("lockedStorage.legalHold", "Legal hold")}
								</Badge>
							)}
						</div>
					</div>
				</button>
				{actions.renderVersions && (
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="shrink-0 gap-1 px-2 text-xs text-muted-foreground"
						aria-expanded={expanded}
						aria-label={
							expanded
								? t("lockedStorage.hideVersions", "Hide versions")
								: t("lockedStorage.showVersions", "Show versions")
						}
						onClick={() => setExpanded((open) => !open)}
					>
						<HistoryIcon className="h-3.5 w-3.5" />
						{item.live_versions}
						<ToggleIcon className="h-3.5 w-3.5" />
					</Button>
				)}
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button
							type="button"
							aria-label={t(
								"lockedStorage.actionsFor",
								"Locked file actions for {{name}}",
								{ name: item.name },
							)}
							className="shrink-0 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
							variant="ghost"
							size="sm"
							onClick={(event) => {
								event.stopPropagation();
								event.preventDefault();
							}}
						>
							<EllipsisVerticalIcon className="h-4 w-4" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end">
						<DropdownMenuLabel>
							{t("lockedStorage.lockedFile", "Locked file")}
						</DropdownMenuLabel>
						<DropdownMenuSeparator />
						<DropdownMenuItem onSelect={() => actions.onDownload(item)}>
							<DownloadIcon className="h-4 w-4 mr-2" />
							{t("lockedStorage.downloadLatest", "Download latest")}
						</DropdownMenuItem>
						<DropdownMenuItem onSelect={() => actions.onShowVersions(item)}>
							<HistoryIcon className="h-4 w-4 mr-2" />
							{t("lockedStorage.viewVersions", "View versions")}
						</DropdownMenuItem>
						{actions.onUploadVersion && (
							<DropdownMenuItem
								onSelect={() => actions.onUploadVersion?.(item)}
							>
								<UploadIcon className="h-4 w-4 mr-2" />
								{t("lockedStorage.uploadNewVersion", "Upload new version")}
							</DropdownMenuItem>
						)}
						{lockedVersion &&
							(actions.onExtendRetention || actions.onToggleLegalHold) && (
								<DropdownMenuSeparator />
							)}
						{lockedVersion &&
							actions.onExtendRetention &&
							lockedVersion.enforcement === "object" && (
								<DropdownMenuItem
									onSelect={() => actions.onExtendRetention?.(item)}
								>
									<CalendarClockIcon className="h-4 w-4 mr-2" />
									{t("lockedStorage.extendRetention", "Extend retention")}
								</DropdownMenuItem>
							)}
						{lockedVersion && actions.onToggleLegalHold && (
							<DropdownMenuItem
								onSelect={() => actions.onToggleLegalHold?.(item)}
							>
								<GavelIcon className="h-4 w-4 mr-2" />
								{legalHold
									? t("lockedStorage.releaseLegalHold", "Release legal hold")
									: t("lockedStorage.placeLegalHold", "Place legal hold")}
							</DropdownMenuItem>
						)}
					</DropdownMenuContent>
				</DropdownMenu>
			</div>
			{expanded && actions.renderVersions?.(item)}
		</div>
	);
}
