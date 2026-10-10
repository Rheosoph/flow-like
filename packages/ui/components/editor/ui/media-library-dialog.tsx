"use client";

import { Folder } from "lucide-react";
import { useEditorRef } from "platejs/react";
import { useEffect, useState } from "react";
import {
	type IStorageTreeEntry,
	sortStorageEntries,
	storageDisplayName,
	storagePrefixTrail,
	storageTreeEntry,
} from "../../../lib/storage-tree";
import { useBackend } from "../../../state/backend-state";
import { matchesAccept } from "../../builder/asset-path";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { toStorageUrl, useEditorUpload } from "../upload-context";

export function MediaLibraryDialog({
	open,
	onOpenChange,
	nodeType,
}: { open: boolean; onOpenChange: (open: boolean) => void; nodeType: string }) {
	const editor = useEditorRef();
	return (
		<MediaLibraryPickerDialog
			open={open}
			onOpenChange={onOpenChange}
			accept={
				nodeType === "img" ? "image" : nodeType === "file" ? "all" : nodeType
			}
			onSelect={(asset) =>
				editor.tf.insertNodes({
					type: nodeType,
					url: asset.url,
					name: asset.name,
					children: [{ text: "" }],
				})
			}
		/>
	);
}

export interface MediaLibraryAsset {
	url: string;
	path: string;
	name: string;
}

export function MediaLibraryPickerDialog({
	open,
	onOpenChange,
	accept = "all",
	onSelect,
	fileFilter,
}: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	accept?: string;
	onSelect: (asset: MediaLibraryAsset) => void;
	fileFilter?: (entry: IStorageTreeEntry) => boolean;
}) {
	const backend = useBackend();
	const { appId, scope } = useEditorUpload();
	const [prefix, setPrefix] = useState("");
	const [search, setSearch] = useState("");
	const [entries, setEntries] = useState<readonly IStorageTreeEntry[]>([]);
	const [error, setError] = useState("");
	const [loading, setLoading] = useState(false);
	const [attempt, setAttempt] = useState(0);
	// biome-ignore lint/correctness/useExhaustiveDependencies: A different storage owner must start at its root folder.
	useEffect(() => {
		setPrefix("");
		setSearch("");
		setEntries([]);
	}, [appId, scope]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: The retry counter deliberately restarts a failed folder request.
	useEffect(() => {
		if (!open || !appId) return;
		let cancelled = false;
		setLoading(true);
		setError("");
		setEntries([]);
		const list =
			scope === "user"
				? backend.storageState.listStorageItemsUser
				: backend.storageState.listStorageItems;
		void list
			.call(backend.storageState, appId, prefix)
			.then((items) => {
				if (!cancelled)
					setEntries(
						sortStorageEntries(
							items.map((item) => storageTreeEntry(item, prefix, scope)),
						),
					);
			})
			.catch(() => {
				if (!cancelled)
					setError(
						"The media library could not load. Check your access and try again.",
					);
			})
			.finally(() => {
				if (!cancelled) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [open, appId, scope, prefix, attempt, backend.storageState]);
	const visible = entries.filter(
		(entry) =>
			(entry.isFolder ||
				(matchesAccept(entry.path, accept) &&
					(!fileFilter || fileFilter(entry)))) &&
			entry.name.toLowerCase().includes(search.toLowerCase()),
	);
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>Media library</DialogTitle>
					<DialogDescription>
						Choose a file from the configured app storage.
					</DialogDescription>
				</DialogHeader>
				{!appId ? (
					<p>This editor has no app storage configured.</p>
				) : (
					<>
						<nav aria-label="Media folders" className="flex flex-wrap gap-1">
							{storagePrefixTrail(prefix).map((folder) => (
								<Button
									key={folder}
									size="sm"
									variant="ghost"
									disabled={folder === prefix}
									onClick={() => {
										setPrefix(folder);
										setSearch("");
									}}
								>
									{folder
										? storageDisplayName(folder)
										: scope === "user"
											? "My files"
											: "App files"}
								</Button>
							))}
						</nav>
						<Input
							aria-label="Search media folder"
							placeholder="Search this folder"
							value={search}
							onChange={(event) => setSearch(event.target.value)}
						/>
						<div className="max-h-80 overflow-y-auto">
							{loading ? (
								<output>Loading media…</output>
							) : error ? (
								<div role="alert">
									<p>{error}</p>
									<Button onClick={() => setAttempt((value) => value + 1)}>
										Try again
									</Button>
								</div>
							) : visible.length === 0 ? (
								<p>No matching media or folders.</p>
							) : (
								<ul>
									{visible.map((entry) => (
										<li key={entry.path}>
											<Button
												className="w-full justify-start"
												variant="ghost"
												onClick={() => {
													if (entry.isFolder) {
														setPrefix(entry.path);
														setSearch("");
														return;
													}
													onSelect({
														url: toStorageUrl("", entry.path, { appId, scope }),
														name: entry.name,
														path: entry.path,
													});
													onOpenChange(false);
												}}
											>
												{entry.isFolder ? <Folder className="size-4" /> : null}
												{entry.name}
											</Button>
										</li>
									))}
								</ul>
							)}
						</div>
					</>
				)}
			</DialogContent>
		</Dialog>
	);
}
