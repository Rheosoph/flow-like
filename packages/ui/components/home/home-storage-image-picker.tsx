"use client";

import { useQuery } from "@tanstack/react-query";
import {
	Check,
	ChevronRight,
	Folder,
	ImageIcon,
	Loader2,
	Search,
} from "lucide-react";
import { useId, useState } from "react";
import { asArray } from "../../lib/response-shape";
import {
	STORAGE_ROOT_PREFIX,
	sortStorageEntries,
	storageDisplayName,
	storagePrefixTrail,
	storageTreeEntry,
} from "../../lib/storage-tree";
import { useBackend, useBackendReady } from "../../state/backend-state";
import { matchesAccept } from "../builder/asset-path";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { useHomeScope } from "./home-content/shared";

export interface HomeStorageImagePickerProps {
	appId: string;
	value: string;
	onChange: (path: string) => void;
}

export function HomeStorageImagePicker({
	appId,
	value,
	onChange,
}: HomeStorageImagePickerProps) {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const scope = useHomeScope();
	const searchId = useId();
	const navigationKey = JSON.stringify([...scope, appId]);
	const [navigation, setNavigation] = useState({
		key: navigationKey,
		prefix: STORAGE_ROOT_PREFIX,
		search: "",
	});
	// Reset before querying so another app or viewer never receives the old folder.
	const { prefix, search } =
		navigation.key === navigationKey
			? navigation
			: { prefix: STORAGE_ROOT_PREFIX, search: "" };
	if (navigation.key !== navigationKey)
		setNavigation({ key: navigationKey, prefix, search });
	const listing = useQuery({
		queryKey: ["home", ...scope, "storage-image-picker", appId, prefix],
		queryFn: () => backend.storageState.listStorageItems(appId, prefix),
		enabled: backendReady && appId.length > 0,
		staleTime: 30_000,
		retry: false,
	});
	const entries = sortStorageEntries(
		asArray(listing.data)
			.map((item) => storageTreeEntry(item, prefix, "app"))
			.filter(
				(entry) =>
					entry.name &&
					(entry.isFolder || matchesAccept(entry.path, "image")) &&
					entry.name.toLowerCase().includes(search.trim().toLowerCase()),
			),
	);
	const navigate = (nextPrefix: string) =>
		setNavigation({ key: navigationKey, prefix: nextPrefix, search: "" });
	const loading = !backendReady || listing.isPending;

	if (!appId)
		return (
			<p className="text-xs text-muted-foreground">
				Choose an app to browse its images.
			</p>
		);

	return (
		<div className="space-y-2">
			<nav
				aria-label="Storage folders"
				className="flex flex-wrap items-center gap-1"
			>
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
							className="h-7 max-w-full px-2 text-xs"
							disabled={folder === prefix}
							aria-current={folder === prefix ? "page" : undefined}
							title={folder || "App storage"}
							onClick={() => navigate(folder)}
						>
							<span className="truncate">
								{folder ? storageDisplayName(folder) : "App storage"}
							</span>
						</Button>
					</div>
				))}
			</nav>
			<div className="relative">
				<label htmlFor={searchId} className="sr-only">
					Search this folder
				</label>
				<Search
					aria-hidden="true"
					className="absolute left-2.5 top-2.5 size-4 text-muted-foreground"
				/>
				<Input
					id={searchId}
					value={search}
					onChange={(event) =>
						setNavigation({
							key: navigationKey,
							prefix,
							search: event.target.value,
						})
					}
					placeholder="Search this folder…"
					className="pl-9"
				/>
			</div>
			<div className="max-h-64 overflow-y-auto rounded-lg border border-border/60 p-1">
				{loading ? (
					<output className="flex items-center gap-2 px-2 py-4 text-xs text-muted-foreground">
						<Loader2 aria-hidden="true" className="size-4 animate-spin" />
						Loading images…
					</output>
				) : listing.isError ? (
					<div role="alert" className="space-y-2 px-2 py-3">
						<p className="text-xs text-muted-foreground">
							Images could not load. Check your connection and access to this
							app.
						</p>
						<Button
							type="button"
							size="sm"
							variant="outline"
							onClick={() => void listing.refetch()}
						>
							Try again
						</Button>
					</div>
				) : entries.length === 0 ? (
					<output className="block px-2 py-4 text-xs text-muted-foreground">
						{search.trim()
							? "No matching images or folders."
							: "No images or folders here."}
					</output>
				) : (
					<ul aria-label="Images and folders" className="space-y-0.5">
						{entries.map((entry) => (
							<li key={entry.path}>
								<Button
									type="button"
									variant={
										value === entry.path && !entry.isFolder
											? "secondary"
											: "ghost"
									}
									size="sm"
									className="w-full justify-start text-xs"
									aria-label={
										entry.isFolder
											? `Open folder ${entry.name}`
											: `Select image ${entry.name}`
									}
									aria-pressed={
										entry.isFolder ? undefined : value === entry.path
									}
									title={entry.path}
									onClick={() =>
										entry.isFolder ? navigate(entry.path) : onChange(entry.path)
									}
								>
									{entry.isFolder ? (
										<Folder aria-hidden="true" className="size-4" />
									) : (
										<ImageIcon aria-hidden="true" className="size-4" />
									)}
									<span className="truncate">{entry.name}</span>
									{entry.isFolder ? (
										<ChevronRight
											aria-hidden="true"
											className="ml-auto size-3"
										/>
									) : value === entry.path ? (
										<Check aria-hidden="true" className="ml-auto size-3" />
									) : null}
								</Button>
							</li>
						))}
					</ul>
				)}
			</div>
			{value && (
				<div className="flex items-start justify-between gap-2 text-[11px]">
					<p className="min-w-0 break-all text-muted-foreground">
						Selected: <span aria-label="Selected image">{value}</span>
					</p>
					<button
						type="button"
						className="shrink-0 rounded underline underline-offset-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						onClick={() => onChange("")}
						aria-label="Clear selected image"
					>
						Clear
					</button>
				</div>
			)}
		</div>
	);
}
