"use client";

import { useTranslation } from "@flow-like/locales";
import { DownloadIcon, GavelIcon } from "lucide-react";
import { useInvoke } from "../..";
import { humanFileSize } from "../../lib/utils";
import type {
	ILockedItem,
	ILockedItemDetail,
	ILockedItemVersion,
} from "../../state/backend-state/locked-storage-state";
import { Badge, Button, Skeleton, UserInlineTag } from "../ui";
import { formatLockedDate, retentionLabel } from "./locked-storage-entries";

export interface ILockedVersionQueries {
	lockedItemDetail(appId: string, itemId: string): Promise<ILockedItemDetail>;
}

export interface ILockedVersionListProps {
	readonly appId: string;
	readonly item: ILockedItem;
	readonly queries: ILockedVersionQueries;
	readonly onDownload: (item: ILockedItem, version?: number) => void;
	readonly onOpenDetails: (itemId: string) => void;
}

function VersionState({ version }: Readonly<{ version: ILockedItemVersion }>) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	if (version.status === "purged") {
		return (
			<span className="text-muted-foreground">
				{t("lockedStorage.purgedOn", "Purged on {{date}}", {
					date: formatLockedDate(version.purged_at, language),
				})}
			</span>
		);
	}
	if (version.status === "pending") {
		return (
			<span className="text-muted-foreground">
				{t("lockedStorage.statusPending", "Pending")}
			</span>
		);
	}
	return (
		<span className="text-muted-foreground">
			{retentionLabel(version, language, t)}
		</span>
	);
}

function VersionLine({
	item,
	version,
	onDownload,
}: Readonly<{
	item: ILockedItem;
	version: ILockedItemVersion;
	onDownload: ILockedVersionListProps["onDownload"];
}>) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const stored = version.status === "locked";
	return (
		<li
			className={`flex flex-wrap items-center gap-x-2 gap-y-1 rounded-md px-2 py-1 text-xs ${
				stored ? "hover:bg-muted/50" : "opacity-60"
			}`}
			data-locked-version={version.id}
		>
			<Badge variant="secondary" className="h-5 px-1.5 text-xs">
				{version.version === null
					? t("lockedStorage.versionPending", "New version")
					: t("lockedStorage.versionBadge", "v{{version}}", {
							version: version.version,
						})}
			</Badge>
			<span className="min-w-0 max-w-[40%] truncate font-medium text-foreground">
				{version.file_name}
			</span>
			<span className="text-muted-foreground">
				{humanFileSize(version.size, true)}
			</span>
			<span className="text-muted-foreground">
				{formatLockedDate(
					version.locked_at ?? version.created_at,
					language,
					true,
				)}
			</span>
			<UserInlineTag userId={version.uploaded_by} />
			<VersionState version={version} />
			{version.legal_hold && (
				<Badge variant="destructive" className="h-5 px-1.5 text-xs">
					<GavelIcon className="mr-1 h-3 w-3" />
					{t("lockedStorage.legalHold", "Legal hold")}
				</Badge>
			)}
			{stored && version.version !== null && (
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="ml-auto h-6 w-6"
					aria-label={t(
						"lockedStorage.downloadVersion",
						"Download v{{version}}",
						{
							version: version.version,
						},
					)}
					onClick={() => onDownload(item, version.version ?? undefined)}
				>
					<DownloadIcon className="h-3.5 w-3.5" />
				</Button>
			)}
		</li>
	);
}

/** Every version of a locked file, inline under its row in the file browser. */
export function LockedVersionList({
	appId,
	item,
	queries,
	onDownload,
	onOpenDetails,
}: ILockedVersionListProps) {
	const { t } = useTranslation("common");
	const detail = useInvoke(queries.lockedItemDetail, queries, [appId, item.id]);

	if (detail.isError && !detail.data) {
		return (
			<p className="px-2 pt-2 text-xs text-destructive">
				{t("lockedStorage.detailsFailed", "Could not load the versions")}
			</p>
		);
	}
	if (!detail.data) {
		return (
			<div className="flex flex-col gap-1 px-2 pt-2">
				<Skeleton className="h-6 w-full" />
				<Skeleton className="h-6 w-full" />
			</div>
		);
	}
	return (
		<div className="mt-2 border-t border-border/50 pt-2">
			<ul className="flex flex-col gap-0.5">
				{detail.data.versions.map((version) => (
					<VersionLine
						key={version.id}
						item={detail.data.item}
						version={version}
						onDownload={onDownload}
					/>
				))}
			</ul>
			<Button
				type="button"
				variant="link"
				size="sm"
				className="h-7 px-2 text-xs"
				onClick={() => onOpenDetails(item.id)}
			>
				{t(
					"lockedStorage.allVersionDetails",
					"Checksums and retention details",
				)}
			</Button>
		</div>
	);
}
