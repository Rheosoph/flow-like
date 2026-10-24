"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarClockIcon,
	CopyIcon,
	DownloadIcon,
	GavelIcon,
	LockIcon,
	ShieldAlertIcon,
	ShieldCheckIcon,
	TimerIcon,
	TimerOffIcon,
	Trash2Icon,
	UploadIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { humanFileSize } from "../../lib/utils";
import type {
	ILockedItem,
	ILockedItemVersion,
} from "../../state/backend-state/locked-storage-state";
import {
	Badge,
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	Skeleton,
	UserInlineTag,
} from "../ui";
import {
	autoDeletionLabel,
	formatLockedDate,
	retentionEnded,
	shortSha256,
} from "./locked-storage-entries";
import type { LockedStorageController } from "./use-locked-storage";

function Fact({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="flex min-w-0 flex-col gap-0.5">
			<span className="text-xs text-muted-foreground">{label}</span>
			<div className="min-w-0 text-sm">{children}</div>
		</div>
	);
}

function Sha256Fact({ version }: Readonly<{ version: ILockedItemVersion }>) {
	const { t } = useTranslation("common");
	const copy = async () => {
		try {
			await navigator.clipboard.writeText(version.sha256);
			toast.success(t("lockedStorage.checksumCopied", "SHA-256 copied"));
		} catch (error) {
			console.error("Copying the SHA-256 failed:", error);
		}
	};
	return (
		<Fact label={t("lockedStorage.sha256", "SHA-256")}>
			<div className="flex items-center gap-1">
				<code className="truncate font-mono text-xs" title={version.sha256}>
					{shortSha256(version.sha256)}
				</code>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6"
					aria-label={t("lockedStorage.copyChecksum", "Copy SHA-256")}
					onClick={copy}
				>
					<CopyIcon className="h-3 w-3" />
				</Button>
				{version.checksum_verified ? (
					<span className="flex items-center gap-1 text-xs text-primary">
						<ShieldCheckIcon className="h-3.5 w-3.5" />
						{t("lockedStorage.checksumVerified", "Verified by the store")}
					</span>
				) : (
					<span className="flex items-center gap-1 text-xs text-muted-foreground">
						<ShieldAlertIcon className="h-3.5 w-3.5" />
						{t("lockedStorage.checksumUnverified", "Not verified by the store")}
					</span>
				)}
			</div>
		</Fact>
	);
}

/** Switching on after retention ended would delete within the hour; that is the purge button's job. */
function AutoDeleteToggle({
	controller,
	item,
	version,
}: Readonly<{
	controller: LockedStorageController;
	item: ILockedItem;
	version: ILockedItemVersion;
}>) {
	const { t } = useTranslation("common");
	const enabled = version.delete_after_retention;
	if (!controller.canPurge) return null;
	if (!enabled && retentionEnded(version.retain_until)) return null;
	return (
		<Button
			variant="outline"
			size="sm"
			disabled={controller.mutating}
			onClick={() => void controller.toggleAutoDelete({ item, version })}
		>
			{enabled ? (
				<TimerOffIcon className="h-4 w-4" />
			) : (
				<TimerIcon className="h-4 w-4" />
			)}
			{enabled
				? t("lockedStorage.keepAfterRetention", "Keep after retention")
				: t(
						"lockedStorage.deleteAfterRetentionAction",
						"Delete after retention",
					)}
		</Button>
	);
}

function VersionActions({
	controller,
	item,
	version,
}: Readonly<{
	controller: LockedStorageController;
	item: ILockedItem;
	version: ILockedItemVersion;
}>) {
	const { t } = useTranslation("common");
	if (version.status !== "locked") return null;
	const target = { item, version };
	return (
		<div className="flex flex-wrap gap-2">
			<Button
				variant="outline"
				size="sm"
				onClick={() =>
					void controller.download(item, version.version ?? undefined)
				}
			>
				<DownloadIcon className="h-4 w-4" />
				{t("lockedStorage.download", "Download")}
			</Button>
			{controller.canExtendRetention && version.enforcement === "object" && (
				<Button
					variant="outline"
					size="sm"
					disabled={controller.mutating}
					onClick={() => controller.openRetention(target)}
				>
					<CalendarClockIcon className="h-4 w-4" />
					{t("lockedStorage.extendRetention", "Extend retention")}
				</Button>
			)}
			{controller.canSetLegalHold && (
				<Button
					variant="outline"
					size="sm"
					disabled={controller.mutating}
					onClick={() => void controller.toggleLegalHold(target)}
				>
					<GavelIcon className="h-4 w-4" />
					{version.legal_hold
						? t("lockedStorage.releaseLegalHold", "Release legal hold")
						: t("lockedStorage.placeLegalHold", "Place legal hold")}
				</Button>
			)}
			<AutoDeleteToggle controller={controller} item={item} version={version} />
			{controller.canPurge && version.purgeable && (
				<Button
					variant="outline"
					size="sm"
					className="text-destructive"
					disabled={controller.mutating}
					onClick={() => controller.openPurge({ item, versions: [version] })}
				>
					<Trash2Icon className="h-4 w-4" />
					{t("lockedStorage.purgeVersion", "Delete (retention ended)")}
				</Button>
			)}
		</div>
	);
}

function VersionCard({
	controller,
	item,
	version,
}: Readonly<{
	controller: LockedStorageController;
	item: ILockedItem;
	version: ILockedItemVersion;
}>) {
	const { t, i18n } = useTranslation("common");
	const language = i18n.resolvedLanguage ?? i18n.language;
	const purged = version.status === "purged";
	const statusLabel = {
		pending: t("lockedStorage.statusPending", "Pending"),
		locked: t("lockedStorage.statusLocked", "Locked"),
		purged: t("lockedStorage.statusPurged", "Purged"),
	}[version.status];

	return (
		<li
			className={`flex flex-col gap-3 rounded-lg border p-3 ${purged ? "bg-muted/40 opacity-60" : "bg-background"}`}
			data-locked-version={version.id}
		>
			<div className="flex flex-wrap items-center gap-2">
				<Badge variant="secondary">
					{version.version === null
						? t("lockedStorage.versionPending", "New version")
						: t("lockedStorage.versionBadge", "v{{version}}", {
								version: version.version,
							})}
				</Badge>
				<span className="min-w-0 flex-1 truncate font-medium">
					{version.file_name}
				</span>
				<Badge variant={purged ? "outline" : "secondary"}>{statusLabel}</Badge>
				{version.legal_hold && (
					<Badge variant="destructive">
						<GavelIcon className="h-3 w-3" />
						{t("lockedStorage.legalHold", "Legal hold")}
					</Badge>
				)}
			</div>
			{purged && (
				<div className="flex flex-wrap items-center gap-1 text-sm text-muted-foreground">
					<span>
						{t("lockedStorage.purgedOn", "Purged on {{date}}", {
							date: formatLockedDate(version.purged_at, language, true),
						})}
					</span>
					{version.purged_by && (
						<>
							<span>{t("lockedStorage.purgedBy", "by")}</span>
							<UserInlineTag userId={version.purged_by} />
						</>
					)}
				</div>
			)}
			<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
				<Fact label={t("lockedStorage.size", "Size")}>
					{humanFileSize(version.size, true)}
				</Fact>
				<Fact label={t("lockedStorage.uploadedBy", "Uploaded by")}>
					<UserInlineTag userId={version.uploaded_by} />
				</Fact>
				<Fact label={t("lockedStorage.uploadedAt", "Uploaded")}>
					{formatLockedDate(version.created_at, language, true)}
				</Fact>
				<Fact label={t("lockedStorage.lockedAt", "Locked")}>
					{formatLockedDate(version.locked_at, language, true) || "—"}
				</Fact>
				<Sha256Fact version={version} />
				<Fact label={t("lockedStorage.retention", "Retention")}>
					<span className="flex flex-wrap items-center gap-1">
						<Badge variant="outline">
							{version.retention_mode === "compliance"
								? t("lockedStorage.modeCompliance", "Compliance")
								: t("lockedStorage.modeGovernance", "Governance")}
						</Badge>
						<Badge variant="outline">
							{version.enforcement === "object"
								? t("lockedStorage.enforcementObject", "Object lock")
								: t("lockedStorage.enforcementBucket", "Bucket policy")}
						</Badge>
						{!purged && (
							<span className="text-muted-foreground">
								{autoDeletionLabel(version, language, t) ??
									(retentionEnded(version.retain_until)
										? t(
												"lockedStorage.retentionEndedOn",
												"Retention ended {{date}}",
												{
													date: formatLockedDate(
														version.retain_until,
														language,
													),
												},
											)
										: t("lockedStorage.lockedUntil", "Locked until {{date}}", {
												date: formatLockedDate(version.retain_until, language),
											}))}
							</span>
						)}
					</span>
				</Fact>
				{version.comment && (
					<Fact label={t("lockedStorage.comment", "Comment")}>
						<p className="whitespace-pre-wrap break-words">{version.comment}</p>
					</Fact>
				)}
			</div>
			<VersionActions controller={controller} item={item} version={version} />
		</li>
	);
}

function DetailsHeader({
	controller,
	item,
	versions,
}: Readonly<{
	controller: LockedStorageController;
	item: ILockedItem;
	versions: ILockedItemVersion[];
}>) {
	const { t } = useTranslation("common");
	const purgeable = versions.filter((version) => version.purgeable);
	return (
		<div className="flex flex-wrap items-center gap-2">
			<Badge variant="outline">
				{t("lockedStorage.liveVersions", "Stored versions: {{versions}}", {
					versions: item.live_versions,
				})}
			</Badge>
			<Badge variant="outline">
				{t("lockedStorage.liveSize", "Counts against storage: {{size}}", {
					size: humanFileSize(item.live_size, true),
				})}
			</Badge>
			<div className="ml-auto flex flex-wrap gap-2">
				{controller.canWrite && (
					<Button
						size="sm"
						variant="outline"
						onClick={() => controller.openUpload({ item })}
					>
						<UploadIcon className="h-4 w-4" />
						{t("lockedStorage.uploadNewVersion", "Upload new version")}
					</Button>
				)}
				{controller.canPurge && purgeable.length > 0 && (
					<Button
						size="sm"
						variant="outline"
						className="text-destructive"
						disabled={controller.mutating}
						onClick={() => controller.openPurge({ item, versions: purgeable })}
					>
						<Trash2Icon className="h-4 w-4" />
						{t("lockedStorage.purgeExpired", "Delete expired versions")}
					</Button>
				)}
			</div>
		</div>
	);
}

export function LockedItemDetails({
	controller,
}: Readonly<{ controller: LockedStorageController }>) {
	const { t } = useTranslation("common");
	const detail = controller.details.data;
	const open = !!controller.detailsItemId;
	const loaded = detail?.item?.id === controller.detailsItemId ? detail : null;

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) controller.closeDetails();
			}}
		>
			<DialogContent className="flex max-h-[90dvh] flex-col gap-4 sm:max-w-2xl">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<LockIcon className="h-4 w-4 shrink-0 text-primary" />
						<span className="truncate">
							{loaded?.item.name ??
								t("lockedStorage.lockedFile", "Locked file")}
						</span>
					</DialogTitle>
					<DialogDescription>
						{t(
							"lockedStorage.detailsDescription",
							"Every version is stored write-once. Versions can only be deleted after their retention ends.",
						)}
					</DialogDescription>
				</DialogHeader>
				{controller.details.isError && !loaded ? (
					<p className="text-sm text-destructive">
						{t("lockedStorage.detailsFailed", "Could not load the versions")}
					</p>
				) : loaded ? (
					<>
						<DetailsHeader
							controller={controller}
							item={loaded.item}
							versions={loaded.versions}
						/>
						<ul className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
							{loaded.versions.map((version) => (
								<VersionCard
									key={version.id}
									controller={controller}
									item={loaded.item}
									version={version}
								/>
							))}
						</ul>
					</>
				) : (
					<div className="flex flex-col gap-3">
						<Skeleton className="h-24 w-full" />
						<Skeleton className="h-24 w-full" />
					</div>
				)}
			</DialogContent>
		</Dialog>
	);
}
