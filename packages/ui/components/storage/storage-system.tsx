"use client";

import {
	FilesIcon,
	FolderPlusIcon,
	GridIcon,
	LayoutGridIcon,
	LinkIcon,
	ListIcon,
	MaximizeIcon,
	MinimizeIcon,
	SearchIcon,
	SortAscIcon,
	UploadIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	BulkUploadPartialFailureError,
	type BulkUploadProgressCallback,
	type IBulkUploadProgress,
	type IStorageItem,
	type IStorageUploadOptions,
	useBackend,
	useInvoke,
} from "../..";
import { humanFileSize } from "../../lib/utils";
import {
	Badge,
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	EmptyState,
	FilePreviewer,
	Input,
	Progress,
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
	Separator,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	isCode,
	isText,
} from "../ui";
import { StorageBreadcrumbs } from "./storage-breadcrumbs";
import { FileOrFolder } from "./storage-file-or-folder";

interface StorageOperationResult {
	prefix: string;
	url?: string;
	error?: string;
}

/** Compact "time remaining" for the upload panel. */
function formatEta(seconds?: number): string | null {
	if (seconds === undefined || !Number.isFinite(seconds) || seconds <= 0) {
		return null;
	}
	if (seconds < 60) return `${Math.ceil(seconds)}s left`;
	if (seconds < 3600) return `${Math.round(seconds / 60)}m left`;
	return `${(seconds / 3600).toFixed(1)}h left`;
}

/**
 * File list.
 *
 * A folder can hold tens of thousands of entries — uploading one is now
 * possible, so browsing one has to be too. Every row opts into
 * `content-visibility`, so the browser skips layout and paint for rows that
 * are off-screen. JS windowing was tried here and could not repaint reliably:
 * the re-renders the virtualizer schedules on itself never land in this page's
 * React tree, so the list stayed blank until something else re-rendered it.
 */
function StorageItemGrid({
	items,
	viewMode,
	renderItem,
}: Readonly<{
	items: readonly IStorageItem[];
	viewMode: "grid" | "list";
	renderItem: (file: IStorageItem) => ReactNode;
}>) {
	return (
		<div className="flex-1 min-h-0 overflow-y-auto">
			<div
				className={`grid gap-2 ${viewMode === "grid" ? "grid-cols-2 md:grid-cols-3 lg:grid-cols-4" : "grid-cols-1"}`}
			>
				{items.map((file) => (
					<div
						key={file.location}
						className="min-w-0 [content-visibility:auto] [contain-intrinsic-size:auto_72px]"
					>
						{renderItem(file)}
					</div>
				))}
			</div>
		</div>
	);
}

interface StorageOperations {
	listStorageItems: (appId: string, prefix: string) => Promise<IStorageItem[]>;
	deleteStorageItems: (appId: string, prefixes: string[]) => Promise<void>;
	downloadStorageItems: (
		appId: string,
		prefixes: string[],
	) => Promise<StorageOperationResult[]>;
	uploadStorageItems: (
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	) => Promise<void>;
	writeStorageItems?: (items: StorageOperationResult[]) => Promise<void>;
}

export function StorageSystem({
	appId,
	prefix,
	updatePrefix,
	fileToUrl,
	title = "Storage",
	storageScopeKey = "shared",
	operations,
	revealInExplorer,
	openWithApp,
	listAppsForFile,
}: Readonly<{
	appId: string;
	prefix: string;
	updatePrefix: (prefix: string) => void;
	fileToUrl: (prefix: string) => Promise<string>;
	title?: string;
	storageScopeKey?: string;
	operations?: StorageOperations;
	revealInExplorer?: (location: string) => void;
	openWithApp?: (location: string, appPath?: string) => void;
	listAppsForFile?: (
		location: string,
	) => Promise<import("./storage-file-or-folder").AppEntry[]>;
}>) {
	// Responsive helper: detect small screens (<= Tailwind 'sm')
	const useIsSmallScreen = () => {
		const [isSmall, setIsSmall] = useState(false);
		useEffect(() => {
			if (typeof window === "undefined") return;
			const mql = window.matchMedia("(max-width: 640px)");
			const onChange = (e: MediaQueryListEvent) => setIsSmall(e.matches);
			const legacyOnChange = () => setIsSmall(mql.matches);
			// Set initial
			setIsSmall(mql.matches);
			// Subscribe (support older Safari)
			try {
				mql.addEventListener("change", onChange);
			} catch {
				// Fallback for Safari < 14
				// @ts-ignore
				mql.addListener(legacyOnChange);
			}
			return () => {
				try {
					mql.removeEventListener("change", onChange);
				} catch {
					// Fallback for Safari < 14
					// @ts-ignore
					mql.removeListener(legacyOnChange);
				}
			};
		}, []);
		return isSmall;
	};
	const isSmallScreen = useIsSmallScreen();
	const fileReference = useRef<HTMLInputElement>(null);
	const folderReference = useRef<HTMLInputElement>(null);
	const backend = useBackend();
	const storageApi = useMemo<StorageOperations>(
		() => ({
			listStorageItems: (targetAppId, targetPrefix) =>
				operations?.listStorageItems(targetAppId, targetPrefix) ??
				backend.storageState.listStorageItems(targetAppId, targetPrefix),
			deleteStorageItems: (targetAppId, prefixes) =>
				operations?.deleteStorageItems(targetAppId, prefixes) ??
				backend.storageState.deleteStorageItems(targetAppId, prefixes),
			downloadStorageItems: (targetAppId, prefixes) =>
				operations?.downloadStorageItems(targetAppId, prefixes) ??
				backend.storageState.downloadStorageItems(targetAppId, prefixes),
			uploadStorageItems: (
				targetAppId,
				targetPrefix,
				files,
				onProgress,
				uploadOptions,
			) =>
				operations?.uploadStorageItems(
					targetAppId,
					targetPrefix,
					files,
					onProgress,
					uploadOptions,
				) ??
				backend.storageState.uploadStorageItems(
					targetAppId,
					targetPrefix,
					files,
					onProgress,
					uploadOptions,
				),
			writeStorageItems:
				operations?.writeStorageItems ??
				backend.storageState.writeStorageItems?.bind(backend.storageState),
		}),
		[backend.storageState, operations],
	);
	const [preview, setPreview] = useState({
		url: "",
		file: "",
	});
	const [uploadProgress, setUploadProgress] = useState<{
		isUploading: boolean;
		progress: number;
		fileCount: number;
		currentFile: string;
		detail?: IBulkUploadProgress;
	}>({
		isUploading: false,
		progress: 0,
		fileCount: 0,
		currentFile: "",
	});
	const uploadAbort = useRef<AbortController | null>(null);
	const files = useInvoke(
		storageApi.listStorageItems,
		storageApi,
		[appId, prefix],
		true,
		[storageScopeKey],
	);

	// ---------- Virtual folders (sessionStorage) ----------
	const [creatingFolder, setCreatingFolder] = useState(false);
	const [newFolderName, setNewFolderName] = useState("");
	const [virtualFoldersHere, setVirtualFoldersHere] = useState<string[]>([]);

	const storeKey = useMemo(
		() => `vfolders:${storageScopeKey}:${appId}`,
		[appId, storageScopeKey],
	);
	const normalizePrefix = useCallback(
		(p: string) => p.replace(/^\/+|\/+$/g, ""),
		[],
	);
	const currentParentKey = useMemo(
		() => normalizePrefix(prefix),
		[prefix, normalizePrefix],
	);

	type VFMap = Record<string, string[]>; // parentPrefix -> [childFolderNames]
	const readVF = useCallback((): VFMap => {
		try {
			const raw = sessionStorage.getItem(storeKey);
			return raw ? (JSON.parse(raw) as VFMap) : {};
		} catch {
			return {};
		}
	}, [storeKey]);
	const writeVF = useCallback(
		(map: VFMap) => {
			try {
				sessionStorage.setItem(storeKey, JSON.stringify(map));
			} catch {
				// ignore
			}
		},
		[storeKey],
	);

	useEffect(() => {
		const map = readVF();
		setVirtualFoldersHere(map[currentParentKey] ?? []);
	}, [currentParentKey, readVF]);

	const addVirtualFolder = useCallback(
		(name: string) => {
			const clean = name.trim();
			if (!clean) {
				toast.error("Folder name cannot be empty");
				return false;
			}
			if (/^[.]{1,2}$/.test(clean)) {
				toast.error("Reserved name");
				return false;
			}
			if (/[\\\/:*?"<>|]/.test(clean)) {
				toast.error("Invalid characters in name");
				return false;
			}

			// check duplicates against visible folders (backend + virtual)
			const existingFolderNames = new Set(
				(files.data ?? [])
					.filter((f) => f.is_dir)
					.map((f) => (f.location.split("/").pop() ?? "").toLowerCase()),
			);
			for (const v of virtualFoldersHere)
				existingFolderNames.add(v.toLowerCase());
			if (existingFolderNames.has(clean.toLowerCase())) {
				toast.error("A folder with that name already exists");
				return false;
			}

			const all = readVF();
			const next = new Set(all[currentParentKey] ?? []);
			next.add(clean);
			all[currentParentKey] = Array.from(next);
			writeVF(all);
			setVirtualFoldersHere(all[currentParentKey]);
			toast.success("Folder created");
			return true;
		},
		[files.data, virtualFoldersHere, currentParentKey, readVF, writeVF],
	);

	// Merge backend items with virtual folders for current prefix
	const filesWithVirtual = useMemo<IStorageItem[]>(() => {
		const base = (files.data ?? []).slice();
		const have = new Set(base.map((f) => f.location));
		const basePrefixNorm = normalizePrefix(prefix);
		const locFor = (name: string) =>
			basePrefixNorm ? `${basePrefixNorm}/${name}` : name;
		const virtualItems: IStorageItem[] = virtualFoldersHere
			.filter((name) => !have.has(locFor(name)))
			.map(
				(name) =>
					({
						location: locFor(name),
						is_dir: true,
						size: 0,
						last_modified: new Date().toISOString(),
					}) as IStorageItem,
			);
		return [...base, ...virtualItems];
	}, [files.data, virtualFoldersHere, prefix, normalizePrefix]);

	const [searchQuery, setSearchQuery] = useState("");
	const [viewMode, setViewMode] = useState<"grid" | "list">("list");
	const [sortBy, setSortBy] = useState<"name" | "date" | "size" | "type">(
		"name",
	);
	const [sortOrder, setSortOrder] = useState<"asc" | "desc">("asc");
	const [isPreviewMaximized, setIsPreviewMaximized] = useState(false);

	const cancelUpload = useCallback(() => {
		uploadAbort.current?.abort();
	}, []);

	const processFiles = useCallback(
		async (inputFiles: File[]) => {
			if (inputFiles.length === 0) return;
			const fileList = Array.from(inputFiles);

			const controller = new AbortController();
			uploadAbort.current = controller;

			setUploadProgress({
				isUploading: true,
				progress: 0,
				fileCount: fileList.length,
				currentFile: fileList[0]?.name || "",
			});

			const settle = () =>
				setUploadProgress({
					isUploading: false,
					progress: 0,
					fileCount: 0,
					currentFile: "",
				});

			try {
				await storageApi.uploadStorageItems(
					appId,
					prefix,
					fileList,
					(progress, detail) => {
						setUploadProgress((prev) => ({
							...prev,
							progress,
							detail,
							currentFile: detail?.currentFile ?? prev.currentFile,
						}));
					},
					{ signal: controller.signal },
				);

				settle();
				if (controller.signal.aborted) {
					toast.info("Upload cancelled");
				} else {
					toast.success(
						fileList.length === 1
							? "File uploaded successfully"
							: `${fileList.length.toLocaleString()} files uploaded successfully`,
					);
				}
			} catch (error) {
				console.error(error);
				settle();
				// A run the user stopped is not a failure, even if the files it
				// had already given up on surface as one.
				if (controller.signal.aborted) {
					toast.info("Upload cancelled");
				} else if (error instanceof BulkUploadPartialFailureError) {
					const { uploaded, failed } = error.result;
					toast.error(
						`${failed.length.toLocaleString()} of ${(uploaded + failed.length).toLocaleString()} files failed to upload`,
						{ description: `${failed[0]?.path}: ${failed[0]?.error}` },
					);
				} else {
					toast.error("Failed to upload files", {
						description: error instanceof Error ? error.message : undefined,
					});
				}
			} finally {
				uploadAbort.current = null;
				// Always refetch: a cancelled or partial run still wrote files.
				files.refetch();
			}
		},
		[prefix, storageApi, appId, files.refetch],
	);

	const loadFile = useCallback(
		async (file: string) => {
			if (preview.file === file) {
				setPreview((old) => ({ ...old, file: "", url: "" }));
				return;
			}

			const url = await storageApi.downloadStorageItems(appId, [file]);

			if (url.length === 0 || !url[0]?.url) {
				toast.error("Failed to load file preview");
				return;
			}

			const fileUrl = url[0].url;

			setPreview({
				url: fileUrl,
				file,
			});
		},
		[appId, preview, storageApi],
	);

	const saveFile = useCallback(
		async (fileContent: string) => {
			if (!preview.file) {
				toast.error("No file selected");
				return;
			}

			try {
				const blob = new Blob([fileContent], { type: "text/plain" });
				const fileName = preview.file.split("/").pop() || "file";
				const file = new File([blob], fileName, { type: "text/plain" });

				await storageApi.uploadStorageItems(appId, prefix, [file], undefined);

				await files.refetch();
			} catch (error) {
				console.error("Failed to save file:", error);
				throw error;
			}
		},
		[appId, prefix, preview.file, storageApi, files],
	);

	const isFileEditable = useCallback((fileUrl: string, fileName?: string) => {
		return isCode(fileUrl, fileName) || isText(fileUrl, fileName);
	}, []);

	const downloadFile = useCallback(
		async (file: string) => {
			if (preview.file === file) {
				setPreview((old) => ({ ...old, file: "", url: "" }));
				return;
			}

			const signedUrl = await storageApi.downloadStorageItems(appId, [file]);

			if (signedUrl.length === 0 || !signedUrl[0]?.url) {
				toast.error("Failed to load file preview");
				return;
			}

			if (storageApi.writeStorageItems) {
				await storageApi.writeStorageItems(signedUrl);
				return;
			}

			const fileUrl = signedUrl[0].url;
			const fileName =
				fileUrl.split("/").pop()?.split("?")[0] || "downloaded_file";
			const fileContent = await fetch(fileUrl).then((res) => res.blob());
			const blob = new Blob([fileContent], {
				type: "application/octet-stream",
			});
			const url = URL.createObjectURL(blob);
			const a = document.createElement("a");
			a.href = url;
			a.download = fileName;
			document.body.appendChild(a);
			a.click();
			document.body.removeChild(a);
			URL.revokeObjectURL(url);
		},
		[appId, preview, storageApi],
	);

	const filteredFiles = useMemo(
		() =>
			filesWithVirtual?.filter((file) =>
				file.location
					.split("/")
					.pop()
					?.toLowerCase()
					.includes(searchQuery.toLowerCase()),
			) ?? [],
		[filesWithVirtual, searchQuery],
	);

	const sortedFiles = useMemo(
		() =>
			[...filteredFiles].sort((a, b) => {
				const getName = (file: IStorageItem) =>
					file.location.split("/").pop() ?? "";
				const isFolder = (file: IStorageItem) => file.is_dir;

				// Always sort folders first
				if (isFolder(a) && !isFolder(b)) return -1;
				if (!isFolder(a) && isFolder(b)) return 1;

				let comparison = 0;

				switch (sortBy) {
					case "name":
						comparison = getName(a).localeCompare(getName(b));
						break;
					case "date":
						comparison =
							new Date(a.last_modified ?? 0).getTime() -
							new Date(b.last_modified ?? 0).getTime();
						break;
					case "size":
						comparison = (a.size ?? 0) - (b.size ?? 0);
						break;
					case "type": {
						const extA = getName(a).split(".").pop() ?? "";
						const extB = getName(b).split(".").pop() ?? "";
						comparison = extA.localeCompare(extB);
						break;
					}
				}

				return sortOrder === "asc" ? comparison : -comparison;
			}),
		[filteredFiles, sortBy, sortOrder],
	);

	// One definition for both render sites. They used to carry byte-identical
	// copies of this prop block, differing only in whether navigating into a
	// folder also closes the open preview.
	const renderFile = useCallback(
		(file: IStorageItem, clearPreviewOnNavigate: boolean) => (
			<FileOrFolder
				highlight={preview.file === file.location}
				file={file}
				changePrefix={(newPrefix) => {
					if (clearPreviewOnNavigate) setPreview({ url: "", file: "" });
					updatePrefix(`${prefix}/${newPrefix}`);
				}}
				loadFile={loadFile}
				revealInExplorer={revealInExplorer}
				openWithApp={openWithApp}
				listAppsForFile={listAppsForFile}
				deleteFile={async (target) => {
					try {
						await storageApi.deleteStorageItems(appId, [`${prefix}/${target}`]);
						toast.success("Deleted successfully");
					} catch (error) {
						console.error(error);
						toast.error("Failed to delete");
					} finally {
						await files.refetch();
					}
				}}
				shareFile={async (target) => {
					const downloadLinks = await storageApi.downloadStorageItems(appId, [
						target,
					]);
					const firstItem = downloadLinks[0];
					if (!firstItem?.url) return;
					try {
						await navigator.clipboard.writeText(firstItem.url);
						toast.success("Copied download link to clipboard");
					} catch (error) {
						console.error("Failed to copy link to clipboard:", error);
					}
				}}
				downloadFile={(target) => {
					downloadFile(target);
				}}
			/>
		),
		[
			appId,
			downloadFile,
			files.refetch,
			listAppsForFile,
			loadFile,
			openWithApp,
			prefix,
			preview.file,
			revealInExplorer,
			storageApi,
			updatePrefix,
		],
	);

	const fileCount = filesWithVirtual?.filter((f) => !f.is_dir).length ?? 0;
	const folderCount = filesWithVirtual?.filter((f) => f.is_dir).length ?? 0;

	return (
		<div className="flex flex-col w-full h-full max-h-full overflow-hidden">
			<input
				ref={fileReference}
				type="file"
				className="hidden"
				id="file-upload"
				multiple
				onChange={(e) => {
					if (!e.target.files) return;
					const filesArray = Array.from(e.target.files);
					processFiles(filesArray);
					e.target.value = "";
				}}
			/>

			<input
				ref={folderReference}
				type="file"
				className="hidden"
				id="folder-upload"
				// @ts-ignore - webkitdirectory and directory are non-standard attributes
				webkitdirectory=""
				directory=""
				multiple
				onChange={(e) => {
					if (!e.target.files) return;
					const filesArray = Array.from(e.target.files);
					processFiles(filesArray);
					e.target.value = "";
				}}
			/>

			{/* Upload Progress Indicator */}
			{uploadProgress.isUploading && (
				<div className="mx-4 mt-4 p-4 border rounded-lg bg-card shrink-0">
					<div className="flex items-center justify-between gap-4 mb-2">
						<div className="flex items-center gap-2 min-w-0">
							<UploadIcon className="h-4 w-4 text-primary animate-pulse shrink-0" />
							<span className="text-sm font-medium truncate">
								{uploadProgress.detail
									? `Uploading ${uploadProgress.detail.completedFiles.toLocaleString()} of ${uploadProgress.detail.totalFiles.toLocaleString()} files`
									: `Uploading ${uploadProgress.fileCount.toLocaleString()} file${uploadProgress.fileCount !== 1 ? "s" : ""}`}
							</span>
						</div>
						<div className="flex items-center gap-2 shrink-0">
							<span className="text-sm text-muted-foreground tabular-nums">
								{uploadProgress.progress.toFixed(1)}%
							</span>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 px-2"
								onClick={cancelUpload}
							>
								<XIcon className="h-3.5 w-3.5 mr-1" />
								Cancel
							</Button>
						</div>
					</div>
					<Progress value={uploadProgress.progress} className="mb-2" />
					<div className="flex items-center justify-between gap-4 text-xs text-muted-foreground">
						<p className="truncate">{uploadProgress.currentFile}</p>
						{uploadProgress.detail && (
							<p className="shrink-0 tabular-nums">
								{[
									`${humanFileSize(uploadProgress.detail.uploadedBytes)} / ${humanFileSize(uploadProgress.detail.totalBytes)}`,
									uploadProgress.detail.bytesPerSecond > 0
										? `${humanFileSize(uploadProgress.detail.bytesPerSecond)}/s`
										: null,
									formatEta(uploadProgress.detail.etaSeconds),
									uploadProgress.detail.failedFiles > 0
										? `${uploadProgress.detail.failedFiles.toLocaleString()} failed`
										: null,
								]
									.filter(Boolean)
									.join(" · ")}
							</p>
						)}
					</div>
				</div>
			)}

			{/* Header Section */}
			<div className="flex flex-col gap-4 px-4 pt-4 shrink-0">
				<div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
					<h2 className="text-2xl font-semibold tracking-tight">{title}</h2>
					<div className="flex items-center gap-2 flex-wrap justify-end">
						<div className="hidden sm:flex items-center gap-2">
							<div className="flex items-center gap-1 text-sm text-muted-foreground">
								<span>Sort by:</span>
								<span className="font-medium text-foreground capitalize">
									{sortBy === "date" ? "Date modified" : sortBy}
								</span>
								<span className="text-xs">
									{sortOrder === "asc" ? "↑" : "↓"}
								</span>
							</div>
							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button variant="outline" size="icon">
										<SortAscIcon className="h-4 w-4" />
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end">
									<DropdownMenuItem
										onClick={() => {
											setSortBy("name");
											setSortOrder(
												sortBy === "name" && sortOrder === "asc"
													? "desc"
													: "asc",
											);
										}}
										className="flex items-center justify-between"
									>
										Name
										{sortBy === "name" && (
											<span className="text-xs text-muted-foreground">
												{sortOrder === "asc" ? " ↑" : " ↓"}
											</span>
										)}
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() => {
											setSortBy("date");
											setSortOrder(
												sortBy === "date" && sortOrder === "asc"
													? "desc"
													: "asc",
											);
										}}
										className="flex items-center justify-between"
									>
										Date modified
										{sortBy === "date" && (
											<span className="text-xs text-muted-foreground">
												{sortOrder === "asc" ? " ↑" : " ↓"}
											</span>
										)}
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() => {
											setSortBy("size");
											setSortOrder(
												sortBy === "size" && sortOrder === "asc"
													? "desc"
													: "asc",
											);
										}}
										className="flex items-center justify-between"
									>
										Size
										{sortBy === "size" && (
											<span className="text-xs text-muted-foreground">
												{sortOrder === "asc" ? " ↑" : " ↓"}
											</span>
										)}
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() => {
											setSortBy("type");
											setSortOrder(
												sortBy === "type" && sortOrder === "asc"
													? "desc"
													: "asc",
											);
										}}
										className="flex items-center justify-between"
									>
										Type
										{sortBy === "type" && (
											<span className="text-xs text-muted-foreground">
												{sortOrder === "asc" ? " ↑" : " ↓"}
											</span>
										)}
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
						</div>

						{/* View toggle */}
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="outline"
									size="icon"
									onClick={() =>
										setViewMode(viewMode === "grid" ? "list" : "grid")
									}
								>
									{viewMode === "grid" ? (
										<ListIcon className="h-4 w-4" />
									) : (
										<GridIcon className="h-4 w-4" />
									)}
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								Switch to {viewMode === "grid" ? "list" : "grid"} view
							</TooltipContent>
						</Tooltip>
						<Separator orientation="vertical" className="h-6" />

						{/* New virtual folder */}
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="outline"
									className="gap-2"
									onClick={() => setCreatingFolder((v) => !v)}
								>
									<FolderPlusIcon className="h-4 w-4" />
									<span className="hidden sm:inline">New Folder</span>
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								Create a virtual folder (session only)
							</TooltipContent>
						</Tooltip>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="outline"
									className="gap-2"
									onClick={() => fileReference.current?.click()}
								>
									<UploadIcon className="h-4 w-4" />
									<span className="hidden sm:inline">Upload Files</span>
								</Button>
							</TooltipTrigger>
							<TooltipContent>Upload files to current folder</TooltipContent>
						</Tooltip>

						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="outline"
									className="gap-2"
									onClick={() => folderReference.current?.click()}
								>
									<FolderPlusIcon className="h-4 w-4" />
									<span className="hidden sm:inline">Upload Folder</span>
								</Button>
							</TooltipTrigger>
							<TooltipContent>Upload entire folder</TooltipContent>
						</Tooltip>
					</div>
				</div>

				<div className="flex flex-col sm:flex-row sm:items-end gap-2 mt-2 sm:justify-between">
					<div className="overflow-x-auto whitespace-nowrap max-w-full">
						<StorageBreadcrumbs
							appId={appId}
							prefix={prefix}
							updatePrefix={(prefix) => updatePrefix(prefix)}
						/>
					</div>
					{(filesWithVirtual.length ?? 0) > 0 && (
						<div className="relative w-full sm:w-auto">
							<SearchIcon className="absolute left-3 top-1/2 transform -translate-y-1/2 h-4 w-4 text-muted-foreground" />
							<Input
								placeholder="Search files and folders..."
								className="pl-10 w-full"
								value={searchQuery}
								onChange={(e) => setSearchQuery(e.target.value)}
							/>
						</div>
					)}
				</div>

				{/* Inline create folder row */}
				{creatingFolder && (
					<div className="flex items-center gap-2 px-4">
						<Input
							placeholder="Folder name"
							value={newFolderName}
							onChange={(e) => setNewFolderName(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter") {
									if (addVirtualFolder(newFolderName)) {
										setNewFolderName("");
										setCreatingFolder(false);
									}
								}
								if (e.key === "Escape") {
									setCreatingFolder(false);
									setNewFolderName("");
								}
							}}
						/>
						<Button
							variant="default"
							onClick={() => {
								if (addVirtualFolder(newFolderName)) {
									setNewFolderName("");
									setCreatingFolder(false);
								}
							}}
						>
							Create
						</Button>
						<Button
							variant="ghost"
							onClick={() => {
								setCreatingFolder(false);
								setNewFolderName("");
							}}
						>
							Cancel
						</Button>
					</div>
				)}
			</div>

			<Separator className="shrink-0 my-4" />

			{/* Content Section */}
			{(filesWithVirtual.length ?? 0) === 0 && (
				<div className="flex flex-col flex-1 min-h-0 w-full relative px-4 pb-4">
					<EmptyState
						className="w-full h-full max-w-full border-2 border-dashed border-muted-foreground/25 rounded-lg"
						title="No Files Found"
						description="Get started by creating a folder or uploading your first files to this storage space"
						action={[
							{
								label: "New Folder",
								onClick: () => setCreatingFolder(true),
							},
							{
								label: "Upload Files",
								onClick: () => fileReference.current?.click(),
							},
							{
								label: "Upload Folder",
								onClick: () => folderReference.current?.click(),
							},
						]}
						icons={[LayoutGridIcon, FilesIcon, LinkIcon]}
					/>
				</div>
			)}

			{(filesWithVirtual.length ?? 0) > 0 && (
				<div className="flex flex-col flex-1 min-h-0 px-4 pb-4">
					{preview.url !== "" && (
						<>
							{(isSmallScreen || isPreviewMaximized) && (
								<div className="fixed inset-0 z-50 bg-background">
									<div className="flex flex-col h-full w-full">
										<div className="p-4 border-b bg-background flex items-center justify-between shrink-0">
											<h3 className="font-medium text-lg">
												Preview - {preview.file.split("/").pop()}
											</h3>
											<Button
												variant="ghost"
												size="sm"
												onClick={() => {
													if (isSmallScreen) {
														setPreview((p) => ({ ...p, url: "", file: "" }));
													} else {
														setIsPreviewMaximized(false);
													}
												}}
												className="h-8 w-8 p-0"
											>
												{isSmallScreen ? (
													<XIcon className="h-4 w-4" />
												) : (
													<MinimizeIcon className="h-4 w-4" />
												)}
											</Button>
										</div>
										<div className="flex-1 min-h-0 overflow-auto">
											<FilePreviewer
												url={preview.url}
												filename={preview.file.split("/").pop()}
												editable={isFileEditable(preview.url, preview.file)}
												onSave={saveFile}
											/>
										</div>
									</div>
								</div>
							)}
							{!isSmallScreen && !isPreviewMaximized && (
								<ResizablePanelGroup
									direction="horizontal"
									autoSaveId={"file_viewer"}
									className="border rounded-lg flex-1 min-h-0"
								>
									<ResizablePanel className="flex flex-col p-4 bg-background min-h-0">
										<div
											key={sortBy}
											className="flex flex-col flex-1 min-h-0 gap-2"
										>
											<div className="flex items-center gap-2 shrink-0">
												<h3 className="font-medium text-sm text-muted-foreground">
													Files & Folders
												</h3>
												<Badge
													variant="secondary"
													className="px-2 py-1 text-xs"
												>
													{fileCount} files
												</Badge>
												<Badge
													variant="secondary"
													className="px-2 py-1 text-xs"
												>
													{folderCount} folders
												</Badge>
											</div>
											<StorageItemGrid
												items={sortedFiles}
												viewMode="list"
												renderItem={(file) => renderFile(file, false)}
											/>
										</div>
									</ResizablePanel>
									<ResizableHandle className="mx-2" />
									<ResizablePanel className="flex flex-col p-4 bg-background min-h-0">
										<div className="flex flex-col flex-1 min-h-0 bg-muted/50 rounded-md border">
											<div className="p-2 border-b bg-background rounded-t-md flex items-center justify-between shrink-0">
												<h3 className="font-medium text-sm">Preview</h3>
												<Button
													variant="ghost"
													size="sm"
													onClick={() => setIsPreviewMaximized(true)}
													className="h-6 w-6 p-0"
												>
													<MaximizeIcon className="h-3 w-3" />
												</Button>
											</div>
											<div className="flex-1 min-h-0 overflow-auto">
												<FilePreviewer
													url={preview.url}
													filename={preview.file.split("/").pop()}
													editable={isFileEditable(preview.url, preview.file)}
													onSave={saveFile}
												/>
											</div>
										</div>
									</ResizablePanel>
								</ResizablePanelGroup>
							)}
						</>
					)}
					{preview.url === "" && (
						<div className="flex flex-col flex-1 min-h-0 border rounded-lg p-3 sm:p-4 bg-background">
							<div className="flex items-center gap-2 mb-2 shrink-0">
								<h3 className="font-medium text-sm text-muted-foreground">
									Files & Folders
								</h3>
								<Badge variant="secondary" className="px-2 py-1 text-xs">
									{fileCount} files
								</Badge>
								<Badge variant="secondary" className="px-2 py-1 text-xs">
									{folderCount} folders
								</Badge>
							</div>
							<StorageItemGrid
								items={sortedFiles}
								viewMode={viewMode}
								renderItem={(file) => renderFile(file, true)}
							/>
						</div>
					)}
				</div>
			)}
		</div>
	);
}
