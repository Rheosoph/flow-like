"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	ChevronRight,
	File as FileIcon,
	Folder,
	Loader2,
	Search,
} from "lucide-react";
import { useState } from "react";
import { asArray } from "../../lib/response-shape";
import {
	STORAGE_ROOT_PREFIX,
	sortStorageEntries,
	storageDisplayName,
	storagePrefixTrail,
	storageTreeEntry,
} from "../../lib/storage-tree";
import { useBackend, useBackendReady } from "../../state/backend-state";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import type { StoragePick } from "./storage-attach-limits";

export interface StorageAttachDialogProps {
	readonly open: boolean;
	readonly appId: string;
	readonly onPick: (picks: readonly StoragePick[]) => void;
	readonly onCancel: () => void;
}

export function StorageAttachDialog({
	open,
	appId,
	onPick,
	onCancel,
}: Readonly<StorageAttachDialogProps>) {
	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) onCancel();
			}}
		>
			<DialogContent className="max-w-lg">
				{open && (
					<StorageAttachBody
						appId={appId}
						onPick={onPick}
						onCancel={onCancel}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

function StorageAttachBody({
	appId,
	onPick,
	onCancel,
}: Readonly<Omit<StorageAttachDialogProps, "open">>) {
	const { t } = useTranslation();
	const backend = useBackend();
	const backendReady = useBackendReady();
	const [prefix, setPrefix] = useState(STORAGE_ROOT_PREFIX);
	const [search, setSearch] = useState("");
	const [selected, setSelected] = useState<ReadonlyMap<string, StoragePick>>(
		new Map(),
	);
	const listing = useQuery({
		queryKey: ["composer-storage-attach", appId, prefix],
		queryFn: () => backend.storageState.listStorageItems(appId, prefix),
		enabled: backendReady && appId.length > 0,
		staleTime: 30_000,
		retry: false,
	});
	const needle = search.trim().toLowerCase();
	const entries = sortStorageEntries(
		asArray(listing.data)
			.map((item) => storageTreeEntry(item, prefix, "app"))
			.filter((entry) => entry.name?.toLowerCase().includes(needle)),
	);
	const navigate = (next: string) => {
		setPrefix(next);
		setSearch("");
	};
	const toggle = (path: string, name: string, size: number) =>
		setSelected((current) => {
			const next = new Map(current);
			if (!next.delete(path)) next.set(path, { path, name, size });
			return next;
		});
	const loading = !backendReady || listing.isPending;

	return (
		<>
			<DialogHeader>
				<DialogTitle>
					{t("composerStorageTitle", "Attach from storage")}
				</DialogTitle>
				<DialogDescription>
					{t(
						"composerStorageDescription",
						"Choose files from this app's storage.",
					)}
				</DialogDescription>
			</DialogHeader>
			<nav className="flex flex-wrap items-center gap-1">
				{storagePrefixTrail(prefix).map((folder, index) => (
					<div key={folder} className="flex min-w-0 items-center gap-1">
						{index > 0 && (
							<ChevronRight
								aria-hidden="true"
								className="size-3 text-muted-foreground"
							/>
						)}
						<Button
							type="button"
							variant="ghost"
							size="sm"
							className="h-7 px-2 text-xs"
							disabled={folder === prefix}
							onClick={() => navigate(folder)}
						>
							<span className="truncate">
								{folder
									? storageDisplayName(folder)
									: t("composerStorageRoot", "App storage")}
							</span>
						</Button>
					</div>
				))}
			</nav>
			<div className="relative">
				<Search
					aria-hidden="true"
					className="absolute left-2.5 top-2.5 size-4 text-muted-foreground"
				/>
				<Input
					value={search}
					onChange={(event) => setSearch(event.target.value)}
					aria-label={t("composerStorageSearch", "Search this folder")}
					placeholder={t("composerStorageSearch", "Search this folder")}
					className="pl-9"
				/>
			</div>
			<div className="h-64 overflow-y-auto rounded-lg border border-border/60 p-1">
				{loading ? (
					<output className="flex items-center gap-2 px-2 py-4 text-xs text-muted-foreground">
						<Loader2 aria-hidden="true" className="size-4 animate-spin" />
					</output>
				) : listing.isError ? (
					<div role="alert" className="space-y-2 px-2 py-3">
						<p className="text-xs text-muted-foreground">
							{t("composerStorageLoadFailed", "Storage could not be loaded.")}
						</p>
						<Button
							type="button"
							size="sm"
							variant="outline"
							onClick={() => void listing.refetch()}
						>
							{t("tryAgain", "Try again")}
						</Button>
					</div>
				) : entries.length === 0 ? (
					<output className="block px-2 py-4 text-xs text-muted-foreground">
						{needle
							? t("composerStorageNoMatch", "No matching files or folders.")
							: t("composerStorageEmpty", "No files or folders here.")}
					</output>
				) : (
					<ul className="space-y-0.5">
						{entries.map((entry) => (
							<li key={entry.path}>
								{entry.isFolder ? (
									<Button
										type="button"
										variant="ghost"
										size="sm"
										className="w-full justify-start text-xs"
										title={entry.path}
										onClick={() => navigate(entry.path)}
									>
										<Folder aria-hidden="true" className="size-4" />
										<span className="truncate">{entry.name}</span>
										<ChevronRight
											aria-hidden="true"
											className="ml-auto size-3"
										/>
									</Button>
								) : (
									<label
										htmlFor={`storage-attach-${entry.path}`}
										className="flex cursor-pointer items-center gap-2 rounded-md px-3 py-1.5 text-xs hover:bg-accent"
										title={entry.path}
									>
										<Checkbox
											id={`storage-attach-${entry.path}`}
											checked={selected.has(entry.path)}
											onCheckedChange={() =>
												toggle(entry.path, entry.name, entry.item.size ?? 0)
											}
										/>
										<FileIcon aria-hidden="true" className="size-4" />
										<span className="truncate">{entry.name}</span>
									</label>
								)}
							</li>
						))}
					</ul>
				)}
			</div>
			<DialogFooter className="items-center sm:justify-between">
				<span className="text-xs text-muted-foreground">
					{t("composerStorageSelected", "{{count}} selected", {
						count: selected.size,
					})}
				</span>
				<div className="flex gap-2">
					<Button type="button" variant="outline" onClick={onCancel}>
						{t("cancel", "Cancel")}
					</Button>
					<Button
						type="button"
						disabled={selected.size === 0}
						onClick={() => onPick([...selected.values()])}
					>
						{t("composerStorageAttach", "Attach")}
					</Button>
				</div>
			</DialogFooter>
		</>
	);
}
