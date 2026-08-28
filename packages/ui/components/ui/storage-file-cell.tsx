"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy, ExternalLink, Loader2 } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { cn } from "../../lib";
import { getErrorMessage } from "../../lib/error-message";
import type { StorageFileRef } from "../../lib/storage-file";
import { useBackend } from "../../state/backend-state";
import { Button } from "./button";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "./dialog";
import { FilePreviewer, canPreview, isCode, isText } from "./file-previewer";
import { FileTypeBadge, FileTypeIcon } from "./file-type-visuals";

/** A signed URL outlives its credentials by about an hour; re-sign well inside that. */
const SIGNED_URL_STALE_MS = 5 * 60 * 1000;

/**
 * The in-table stand-in for a stored file.
 *
 * Nothing is fetched here — a page of rows would otherwise sign and download a
 * URL per cell — so the placeholder shows only what the path already says, and
 * the bytes are asked for when a preview is actually opened.
 */
export function StorageFileChip({
	file,
	onClick,
	className,
}: Readonly<{
	file: StorageFileRef;
	onClick: () => void;
	className?: string;
}>) {
	return (
		<Button
			variant="ghost"
			size="sm"
			onClick={onClick}
			title={file.path}
			className={cn("h-6 max-w-[220px] justify-start gap-1.5 px-2", className)}
		>
			<FileTypeIcon
				name={file.fileName}
				className="h-3.5 w-3.5 shrink-0 text-primary"
			/>
			<span className="truncate font-normal">{file.fileName}</span>
		</Button>
	);
}

function useSignedUrl(appId: string, file: StorageFileRef) {
	const backend = useBackend();
	const download =
		file.scope === "user"
			? backend.storageState.downloadStorageItemsUser
			: backend.storageState.downloadStorageItems;

	return useInvoke(
		download,
		backend.storageState,
		[appId, [file.path]],
		Boolean(appId) && Boolean(file.path),
		[],
		SIGNED_URL_STALE_MS,
	);
}

function PreviewNotice({ children }: Readonly<{ children: React.ReactNode }>) {
	return (
		<div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center text-sm text-muted-foreground">
			{children}
		</div>
	);
}

/**
 * The path a cell holds, plus the file it points at.
 *
 * Mounted only from an opened preview, so the signing round-trip is paid once per
 * file the user actually asks to see.
 */
export function StorageFilePreview({
	appId,
	file,
	className,
}: Readonly<{ appId: string; file: StorageFileRef; className?: string }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const signed = useSignedUrl(appId, file);
	const refetch = signed.refetch;
	const url = signed.data?.[0]?.url ?? "";

	const editable = useMemo(
		() => isCode(file.fileName) || isText(file.fileName),
		[file.fileName],
	);

	const saveContent = useCallback(
		async (content: string) => {
			const upload =
				file.scope === "user"
					? backend.storageState.uploadStorageItemsUser
					: backend.storageState.uploadStorageItems;
			const blob = new File([content], file.fileName, { type: "text/plain" });
			await upload.call(
				backend.storageState,
				appId,
				file.directory,
				[blob],
				undefined,
			);
			await refetch();
			toast.success(t("savedName", "Saved {{name}}", { name: file.fileName }));
		},
		[appId, backend.storageState, file, refetch, t],
	);

	return (
		<div className={cn("flex min-h-0 flex-1 flex-col gap-2", className)}>
			<div className="flex shrink-0 items-center gap-2 rounded-md border bg-muted/40 px-2 py-1.5">
				<FileTypeIcon
					name={file.fileName}
					className="h-4 w-4 shrink-0 text-primary"
				/>
				<code className="min-w-0 flex-1 truncate text-xs" title={file.path}>
					{file.path}
				</code>
				<FileTypeBadge
					filename={file.fileName}
					className="h-5 shrink-0 px-1.5 py-0 text-[10px]"
				/>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6 shrink-0"
					aria-label={t("copyPath", "Copy path")}
					onClick={() => {
						void navigator.clipboard.writeText(file.path);
						toast.success(
							t("pathCopiedToClipboard", "Path copied to clipboard"),
						);
					}}
				>
					<Copy className="h-3 w-3" />
				</Button>
				{url && (
					<Button
						variant="ghost"
						size="icon"
						className="h-6 w-6 shrink-0"
						aria-label={t("openInNewTab", "Open in new tab")}
						onClick={() => window.open(url, "_blank", "noopener,noreferrer")}
					>
						<ExternalLink className="h-3 w-3" />
					</Button>
				)}
			</div>

			<div className="min-h-[240px] flex-1 overflow-auto rounded-md border bg-background">
				{!url && signed.isFetching && (
					<PreviewNotice>
						<span className="flex items-center gap-2">
							<Loader2 className="h-4 w-4 animate-spin" />
							{t("loadingPreview", "Loading preview...")}
						</span>
					</PreviewNotice>
				)}
				{!url && !signed.isFetching && (
					<PreviewNotice>
						{signed.error
							? getErrorMessage(
									signed.error,
									t("unknownError", "Unknown error"),
								)
							: t("fileCouldNotBeLoaded", "File could not be loaded")}
					</PreviewNotice>
				)}
				{url && !canPreview(file.fileName) && (
					<PreviewNotice>
						{t(
							"fileTypeNotSupportedForPreview",
							"File type not supported for preview",
						)}
						<Button variant="outline" size="sm" asChild>
							<a href={url} download={file.fileName}>
								{t("download", "Download")}
							</a>
						</Button>
					</PreviewNotice>
				)}
				{url && canPreview(file.fileName) && (
					<FilePreviewer
						key={url}
						url={url}
						filename={file.fileName}
						editable={editable}
						onSave={editable ? saveContent : undefined}
					/>
				)}
			</div>
		</div>
	);
}

/**
 * A file cell that owns its own preview, for tables without a per-cell dialog.
 * Hosts that already have one render the chip and the preview separately.
 */
export function StorageFileCell({
	appId,
	file,
	className,
}: Readonly<{ appId: string; file: StorageFileRef; className?: string }>) {
	const { t } = useTranslation("common");
	const [open, setOpen] = useState(false);

	return (
		<>
			<StorageFileChip
				file={file}
				className={className}
				onClick={() => setOpen(true)}
			/>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent className="flex h-[80vh] max-w-5xl flex-col overflow-hidden">
					<DialogHeader className="shrink-0">
						<DialogTitle className="truncate">
							{t("previewName", "Preview {{name}}", { name: file.fileName })}
						</DialogTitle>
					</DialogHeader>
					{open && <StorageFilePreview appId={appId} file={file} />}
				</DialogContent>
			</Dialog>
		</>
	);
}
