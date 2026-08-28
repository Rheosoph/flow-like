"use client";

import { useTranslation } from "@flow-like/locales";
import { DownloadIcon, RefreshCwIcon, TriangleAlertIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { isExpiredAssetUrl } from "../../../../lib/stable-asset-url";
import { cn } from "../../../../lib/utils";
import { useBackend } from "../../../../state/backend-state";
import type { IStorageItemActionResult } from "../../../../state/backend-state/types";
import { Badge } from "../../../ui/badge";
import { Button } from "../../../ui/button";
import { FilePreviewer, isCode, isText } from "../../../ui/file-previewer";
import { Skeleton } from "../../../ui/skeleton";
import type { IEditorScope } from "../editor-documents";

/**
 * A storage file, read-only unless it is text.
 *
 * `location` is always the path *relative to the storage root*, which is the only shape both
 * halves of the API accept: list/upload/delete fold the segments onto the configured base,
 * and download strips a legacy absolute key before doing the same.
 */
export function StorageDocument({
	appId,
	scope,
	location,
	className,
}: Readonly<{
	appId: string;
	scope: IEditorScope;
	location: string;
	className?: string;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const [url, setUrl] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(true);

	const filename = useMemo(
		() => location.split("/").filter(Boolean).pop() ?? location,
		[location],
	);
	const folder = useMemo(
		() => location.split("/").slice(0, -1).join("/"),
		[location],
	);

	const load = useCallback(async () => {
		setLoading(true);
		setError(null);
		try {
			const results: IStorageItemActionResult[] =
				scope === "user"
					? await backend.storageState.downloadStorageItemsUser(appId, [
							location,
						])
					: await backend.storageState.downloadStorageItems(appId, [location]);
			const signed = results[0]?.url;
			if (!signed) {
				throw new Error(results[0]?.error ?? "no url returned");
			}
			setUrl(signed);
		} catch (cause) {
			console.error("Failed to sign storage file", location, cause);
			setUrl(null);
			setError(
				cause instanceof Error ? cause.message : String(cause ?? "unknown"),
			);
		} finally {
			setLoading(false);
		}
	}, [appId, backend.storageState, location, scope]);

	useEffect(() => {
		void load();
	}, [load]);

	// A signed URL can die with the credentials that minted it — as little as a minute —
	// so a tab left open re-signs rather than rendering a dead link.
	useEffect(() => {
		if (!url || !isExpiredAssetUrl(url)) return;
		void load();
	}, [load, url]);

	const editable = Boolean(
		url && (isCode(url, filename) || isText(url, filename)),
	);

	const save = useCallback(
		async (content: string) => {
			const file = new File([new Blob([content])], filename, {
				type: "text/plain",
			});
			if (scope === "user") {
				await backend.storageState.uploadStorageItemsUser(appId, folder, [
					file,
				]);
			} else {
				await backend.storageState.uploadStorageItems(appId, folder, [file]);
			}
			toast.success(t("fileSaved", "File saved"));
			await load();
		},
		[appId, backend.storageState, filename, folder, load, scope, t],
	);

	const download = useCallback(async () => {
		if (!url) return;
		if (backend.storageState.writeStorageItems) {
			await backend.storageState.writeStorageItems([{ prefix: location, url }]);
			return;
		}
		const anchor = document.createElement("a");
		anchor.href = url;
		anchor.download = filename;
		anchor.rel = "noopener";
		document.body.appendChild(anchor);
		anchor.click();
		document.body.removeChild(anchor);
	}, [backend.storageState, filename, location, url]);

	return (
		<div
			className={cn("flex h-full min-h-0 flex-col bg-background", className)}
		>
			<div className="flex shrink-0 items-center gap-2 border-b px-3 py-1.5">
				<span className="truncate font-mono text-xs" title={location}>
					{location}
				</span>
				<Badge variant="outline" className="shrink-0 text-[10px]">
					{scope === "user"
						? t("userStorage", "User Storage")
						: t("appStorage", "App Storage")}
				</Badge>
				<span className="flex-1" />
				<Button
					size="icon"
					variant="ghost"
					className="size-6"
					title={t("refresh", "Refresh")}
					aria-label={t("refresh", "Refresh")}
					onClick={() => void load()}
				>
					<RefreshCwIcon className="size-3.5" />
				</Button>
				<Button
					size="icon"
					variant="ghost"
					className="size-6"
					disabled={!url}
					title={t("download", "Download")}
					aria-label={t("download", "Download")}
					onClick={() => void download()}
				>
					<DownloadIcon className="size-3.5" />
				</Button>
			</div>

			<div className="min-h-0 flex-1 overflow-auto p-3">
				{loading && <Skeleton className="h-full w-full" />}
				{!loading && error && (
					<div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground text-sm">
						<TriangleAlertIcon className="size-5 text-destructive" />
						<p>{t("couldNotOpenThisFile", "Could not open this file")}</p>
						<p className="max-w-md text-center text-xs">{error}</p>
						<Button size="sm" variant="outline" onClick={() => void load()}>
							{t("tryAgain", "Try again")}
						</Button>
					</div>
				)}
				{!loading && !error && url && (
					<FilePreviewer
						url={url}
						filename={filename}
						editable={editable}
						onSave={editable ? save : undefined}
					/>
				)}
			</div>
		</div>
	);
}
