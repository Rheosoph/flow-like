"use client";

import { useDraggable } from "@dnd-kit/core";
import { useTranslation } from "@flow-like/locales";
import {
	ChevronDownIcon,
	ChevronRightIcon,
	FileIcon,
	FolderIcon,
	Loader2Icon,
	RefreshCwIcon,
	Trash2Icon,
	TriangleAlertIcon,
	UploadIcon,
} from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";
import {
	type IStorageDirectory,
	useStorageTree,
} from "../../../../hooks/use-storage-tree";
import type {
	IStorageScope,
	IStorageTreeEntry,
} from "../../../../lib/storage-tree";
import { useBackend } from "../../../../state/backend-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../../ui/alert-dialog";
import { Button } from "../../../ui/button";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "../../../ui/context-menu";
import { EmptyRow, SectionHeader, TreeRow } from "./explorer-primitives";

/**
 * One storage scope as a lazy tree.
 *
 * Only expanded folders are listed, and a row's `path` is always root-relative — the two
 * backends report `location` differently (absolute object keys on cloud, relative on
 * desktop) and feeding a cloud key back as a prefix folds the base on twice.
 *
 * There is deliberately no rename and no "new folder": `storage_rename` is a registered
 * no-op that returns Ok while doing nothing, and folders are implied by keys rather than
 * created. Offering either would report a success that never happened.
 */
export function StorageRoot({
	appId,
	scope,
	label,
	onOpenFile,
	enabled = true,
}: Readonly<{
	appId: string;
	scope: IStorageScope;
	label: string;
	onOpenFile: (scope: IStorageScope, path: string) => void;
	enabled?: boolean;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const tree = useStorageTree({ appId, scope, enabled });
	const [uploadTo, setUploadTo] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [pendingDelete, setPendingDelete] = useState<IStorageTreeEntry | null>(
		null,
	);
	const fileInput = useRef<HTMLInputElement | null>(null);

	const upload = useCallback(
		async (prefix: string, files: File[]) => {
			if (files.length === 0) return;
			setBusy(true);
			try {
				if (scope === "user") {
					await backend.storageState.uploadStorageItemsUser(
						appId,
						prefix,
						files,
					);
				} else {
					await backend.storageState.uploadStorageItems(appId, prefix, files);
				}
				await tree.refetch(prefix);
				toast.success(
					t("uploadedNFiles", "Uploaded {{count}} file(s)", {
						count: files.length,
					}),
				);
			} catch (cause) {
				console.error("Storage upload failed", cause);
				toast.error(t("uploadFailed", "Upload failed"));
			} finally {
				setBusy(false);
			}
		},
		[appId, backend.storageState, scope, t, tree],
	);

	const remove = useCallback(
		async (entry: IStorageTreeEntry) => {
			// Delete is recursive and there is no trash, and an empty prefix resolves to the
			// scope root — one bad row would wipe the whole store.
			if (!entry.path.trim()) return;
			setBusy(true);
			try {
				if (scope === "user") {
					await backend.storageState.deleteStorageItemsUser(appId, [
						entry.path,
					]);
				} else {
					await backend.storageState.deleteStorageItems(appId, [entry.path]);
				}
				await tree.refetch();
				toast.success(t("deleted", "Deleted"));
			} catch (cause) {
				console.error("Storage delete failed", cause);
				toast.error(t("deleteFailed", "Delete failed"));
			} finally {
				setBusy(false);
			}
		},
		[appId, backend.storageState, scope, t, tree],
	);

	const pickFiles = useCallback((prefix: string) => {
		setUploadTo(prefix);
		fileInput.current?.click();
	}, []);

	const renderDirectory = (
		directory: IStorageDirectory | undefined,
		depth: number,
	): React.ReactNode => {
		if (!directory) return null;
		if (directory.error) {
			return (
				<div
					className="flex items-center gap-1.5 py-1 text-[11px] text-destructive"
					style={{ paddingLeft: `${depth * 12 + 24}px` }}
				>
					<TriangleAlertIcon className="size-3" />
					{t("couldNotListThisFolder", "Could not list this folder")}
				</div>
			);
		}
		if (directory.isLoading) {
			return (
				<div
					className="flex items-center gap-1.5 py-1 text-[11px] text-muted-foreground"
					style={{ paddingLeft: `${depth * 12 + 24}px` }}
				>
					<Loader2Icon className="size-3 animate-spin" />
					{t("loading", "Loading…")}
				</div>
			);
		}
		if (directory.entries.length === 0) {
			return <EmptyRow label={t("emptyFolder", "Empty")} depth={depth} />;
		}
		return directory.entries.map((entry) => (
			<StorageEntryRow
				key={entry.nodeId}
				entry={entry}
				scope={scope}
				depth={depth}
				expanded={entry.isFolder && tree.isExpanded(entry.path)}
				onToggle={() => tree.toggle(entry.path)}
				onOpen={() => onOpenFile(scope, entry.path)}
				onUpload={() => pickFiles(entry.path)}
				onDelete={() => setPendingDelete(entry)}
				renderChildren={() =>
					renderDirectory(tree.directories.get(entry.path), depth + 1)
				}
			/>
		));
	};

	return (
		<>
			<SectionHeader
				label={label}
				action={
					<span className="flex items-center gap-0.5">
						<Button
							size="icon"
							variant="ghost"
							className="size-5 text-muted-foreground"
							disabled={busy}
							title={t("uploadFiles", "Upload files")}
							aria-label={t("uploadFiles", "Upload files")}
							onClick={() => pickFiles("")}
						>
							<UploadIcon className="size-3" />
						</Button>
						<Button
							size="icon"
							variant="ghost"
							className="size-5 text-muted-foreground"
							title={t("refresh", "Refresh")}
							aria-label={t("refresh", "Refresh")}
							onClick={() => void tree.refetch()}
						>
							<RefreshCwIcon
								className={tree.isFetching ? "size-3 animate-spin" : "size-3"}
							/>
						</Button>
					</span>
				}
			/>
			{renderDirectory(tree.root, 0)}

			<input
				ref={fileInput}
				type="file"
				multiple
				className="hidden"
				onChange={(event) => {
					const files = Array.from(event.target.files ?? []);
					event.target.value = "";
					if (uploadTo === null) return;
					void upload(uploadTo, files);
				}}
			/>

			<AlertDialog
				open={pendingDelete !== null}
				onOpenChange={(open) => {
					if (!open) setPendingDelete(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{pendingDelete?.isFolder
								? t("deleteFolderAndItsContents", "Delete folder and contents?")
								: t("deleteFile", "Delete file?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"pathIsDeletedPermanentlyThereIsNoTrash",
								"{{path}} is deleted permanently. There is no trash.",
								{ path: pendingDelete?.path ?? "" },
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							onClick={() => {
								const target = pendingDelete;
								setPendingDelete(null);
								if (target) void remove(target);
							}}
						>
							{t("delete", "Delete")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

/**
 * A file or folder row. Dragging either onto the canvas mints the two nodes that address
 * it, so the payload carries the root-relative path and nothing about how it was listed.
 */
function StorageEntryRow({
	entry,
	scope,
	depth,
	expanded,
	onToggle,
	onOpen,
	onUpload,
	onDelete,
	renderChildren,
}: Readonly<{
	entry: IStorageTreeEntry;
	scope: IStorageScope;
	depth: number;
	expanded: boolean;
	onToggle: () => void;
	onOpen: () => void;
	onUpload: () => void;
	onDelete: () => void;
	renderChildren: () => React.ReactNode;
}>) {
	const { t } = useTranslation("flow");
	const { attributes, listeners, setNodeRef, transform, isDragging } =
		useDraggable({
			id: entry.nodeId,
			data: { type: "storage-path", scope, path: entry.path },
		});

	const row = (
		<TreeRow
			ref={setNodeRef}
			{...attributes}
			{...listeners}
			depth={depth}
			icon={entry.isFolder ? <FolderIcon /> : <FileIcon />}
			label={entry.name}
			muted={!entry.isFolder}
			className={isDragging ? "opacity-50" : undefined}
			style={
				transform
					? { transform: `translate3d(${transform.x}px, ${transform.y}px, 0)` }
					: undefined
			}
			expander={
				entry.isFolder ? (
					<button
						type="button"
						aria-label={
							expanded ? t("collapse", "Collapse") : t("expand", "Expand")
						}
						onClick={(event) => {
							event.stopPropagation();
							onToggle();
						}}
						className="flex size-4 items-center justify-center text-muted-foreground"
					>
						{expanded ? (
							<ChevronDownIcon className="size-3" />
						) : (
							<ChevronRightIcon className="size-3" />
						)}
					</button>
				) : undefined
			}
			onSelect={entry.isFolder ? onToggle : onOpen}
		/>
	);

	return (
		<>
			<ContextMenu>
				<ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
				<ContextMenuContent className="w-56">
					{!entry.isFolder && (
						<ContextMenuItem onSelect={onOpen}>
							<FileIcon className="size-3.5" />
							{t("openPreview", "Open preview")}
						</ContextMenuItem>
					)}
					{entry.isFolder && (
						<ContextMenuItem onSelect={onUpload}>
							<UploadIcon className="size-3.5" />
							{t("uploadFilesHere", "Upload files here")}
						</ContextMenuItem>
					)}
					<ContextMenuSeparator />
					<ContextMenuItem variant="destructive" onSelect={onDelete}>
						<Trash2Icon className="size-3.5" />
						{t("delete", "Delete")}
					</ContextMenuItem>
				</ContextMenuContent>
			</ContextMenu>
			{entry.isFolder && expanded && renderChildren()}
		</>
	);
}
