"use client";

import { useTranslation } from "@flow-like/locales";
import { isCancelledError } from "@tanstack/react-query";
import {
	FilesIcon,
	FolderPlusIcon,
	GridIcon,
	LayoutGridIcon,
	LinkIcon,
	ListIcon,
	LockIcon,
	MaximizeIcon,
	MinimizeIcon,
	PencilIcon,
	RefreshCwIcon,
	SearchIcon,
	SortAscIcon,
	UploadIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import {
	Suspense,
	lazy,
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import {
	BulkUploadPartialFailureError,
	type BulkUploadProgressCallback,
	type IStorageItem,
	type IStorageListOptions,
	type IStorageUploadOptions,
	storageDisplayName,
	useBackend,
	useInvoke,
	useQueryClient,
} from "../..";
import { asArray } from "../../lib/response-shape";
import { normalizeStorageLocation } from "../../lib/storage-tree";
import type { ILockedItem } from "../../state/backend-state/locked-storage-state";
import {
	Badge,
	Button,
	Dialog,
	DialogContent,
	DialogTitle,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	EmptyState,
	FilePreviewer,
	Input,
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
	Separator,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	canPreview,
	isAudio,
	isCode,
	isImage,
	isPdf,
	isText,
	isVideo,
} from "../ui";
import { craftViewer } from "./craft-viewers";
import { LockedFileRow } from "./locked-file-row";
import { LockedStorageLayer } from "./locked-storage-dialogs";
import {
	type ILockedStorageEntry,
	isLockedEntry,
} from "./locked-storage-entries";
import { LockedVersionList } from "./locked-version-list";
import { saveUrlAsFile } from "./save-download";
import { StorageBreadcrumbs } from "./storage-breadcrumbs";
import { createStorageCraftFile, storageCraftSaveName } from "./storage-craft";
import { isStorageDocx } from "./storage-docx";
import { useStorageDocxHistory } from "./storage-docx-history";
import { FileOrFolder } from "./storage-file-or-folder";
import {
	IDLE_UPLOAD_PROGRESS,
	type IUploadProgressState,
	UploadProgressPanel,
} from "./storage-upload-progress";
import { useLockedStorage } from "./use-locked-storage";

const CraftDocument = lazy(() =>
	import("./craft-document").then((module) => ({
		default: module.CraftDocument,
	})),
);
const WordcraftDocument = lazy(() =>
	import("./wordcraft-document").then((module) => ({
		default: module.WordcraftDocument,
	})),
);

const EMPTY_PREVIEW = {
	url: "",
	file: "",
	folder: "",
	appId: "",
	storageScopeKey: "",
	/** Display name when `file` is not a storage path. */
	name: "",
	/** Locked files preview read-only and never open an editor. */
	locked: false,
};

function hasInlineMediaPreview(file: string) {
	const filename = storageDisplayName(file);
	return (
		isPdf(file, filename) ||
		isImage(file, filename) ||
		isAudio(file, filename) ||
		isVideo(file, filename)
	);
}

interface StorageOperationResult {
	prefix: string;
	url?: string;
	error?: string;
}

const NO_VIRTUAL_FOLDERS: string[] = [];

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
	listStorageItems: (
		appId: string,
		prefix: string,
		options?: IStorageListOptions,
	) => Promise<IStorageItem[]>;
	deleteStorageItems: (appId: string, prefixes: string[]) => Promise<void>;
	renameStorageItem: (
		appId: string,
		prefix: string,
		newName: string,
		isDir: boolean,
	) => Promise<void>;
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
	enableDocx = false,
	enableCraftEditors = false,
	canWrite = true,
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
	enableDocx?: boolean;
	enableCraftEditors?: boolean;
	canWrite?: boolean;
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
	const { t } = useTranslation("common");
	const fileReference = useRef<HTMLInputElement>(null);
	const folderReference = useRef<HTMLInputElement>(null);
	const backend = useBackend();
	const queryClient = useQueryClient();
	const storageApi = useMemo<StorageOperations>(
		() => ({
			listStorageItems: (targetAppId, targetPrefix, options) =>
				operations?.listStorageItems(targetAppId, targetPrefix, options) ??
				backend.storageState.listStorageItems(
					targetAppId,
					targetPrefix,
					options,
				),
			deleteStorageItems: (targetAppId, prefixes) =>
				operations?.deleteStorageItems(targetAppId, prefixes) ??
				backend.storageState.deleteStorageItems(targetAppId, prefixes),
			renameStorageItem: (targetAppId, targetPrefix, newName, isDir) =>
				operations?.renameStorageItem(
					targetAppId,
					targetPrefix,
					newName,
					isDir,
				) ??
				backend.storageState.renameStorageItem(
					targetAppId,
					targetPrefix,
					newName,
					isDir,
				),
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
	const [preview, setPreview] = useState(EMPTY_PREVIEW);
	const previewToolTarget =
		preview.appId === appId && preview.storageScopeKey === storageScopeKey
			? {
					appId: preview.appId,
					surfaceId: `storage:${preview.storageScopeKey}`,
					componentId: preview.file,
				}
			: undefined;
	const [inlineEditorOpen, setInlineEditorOpen] = useState(false);
	const editorFor = useCallback(
		(file: string) =>
			enableCraftEditors || (enableDocx && isStorageDocx(file))
				? craftViewer(file)
				: undefined,
		[enableCraftEditors, enableDocx],
	);
	const conversionTarget = useRef({ source: "", id: "" });
	const docxDirty = useRef(false);
	const docxBusy = useRef(false);
	const previewRequest = useRef(0);
	const onDocxDirtyChange = useCallback((dirty: boolean) => {
		docxDirty.current = dirty;
	}, []);
	const onDocxBusyChange = useCallback((busy: boolean) => {
		docxBusy.current = busy;
	}, []);
	const confirmPreviewLeave = useCallback(() => {
		if (docxBusy.current) {
			toast.info("Wait for the document to finish saving.");
			return false;
		}
		return (
			!docxDirty.current || window.confirm("Discard unsaved document changes?")
		);
	}, []);
	const dismissPreview = useCallback(() => {
		previewRequest.current++;
		docxDirty.current = false;
		setInlineEditorOpen(false);
		setPreview(EMPTY_PREVIEW);
	}, []);
	const { openHistory, closeHistory } = useStorageDocxHistory(
		confirmPreviewLeave,
		dismissPreview,
	);
	const closePreview = useCallback(async () => {
		if (!confirmPreviewLeave() || !(await closeHistory())) return false;
		dismissPreview();
		return true;
	}, [closeHistory, confirmPreviewLeave, dismissPreview]);
	const navigatePrefix = useCallback(
		async (nextPrefix: string) => {
			if (await closePreview()) updatePrefix(nextPrefix);
		},
		[closePreview, updatePrefix],
	);
	useEffect(() => {
		const protectDocument = (event: BeforeUnloadEvent) => {
			if (!docxDirty.current && !docxBusy.current) return;
			event.preventDefault();
			event.returnValue = "";
		};
		const protectLinkNavigation = (event: MouseEvent) => {
			if (
				(!docxDirty.current && !docxBusy.current) ||
				event.button !== 0 ||
				event.metaKey ||
				event.ctrlKey ||
				event.shiftKey ||
				event.altKey
			)
				return;
			const anchor =
				event.target instanceof Element
					? event.target.closest("a[href]")
					: null;
			if (
				!(anchor instanceof HTMLAnchorElement) ||
				anchor.download ||
				anchor.target === "_blank"
			)
				return;
			const destination = new URL(anchor.href, window.location.href);
			if (
				destination.pathname === window.location.pathname &&
				destination.search === window.location.search
			)
				return;
			if (!confirmPreviewLeave()) {
				event.preventDefault();
				event.stopImmediatePropagation();
			}
		};
		window.addEventListener("beforeunload", protectDocument);
		document.addEventListener("click", protectLinkNavigation, true);
		return () => {
			window.removeEventListener("beforeunload", protectDocument);
			document.removeEventListener("click", protectLinkNavigation, true);
		};
	}, [confirmPreviewLeave]);
	const [uploadProgress, setUploadProgress] =
		useState<IUploadProgressState>(IDLE_UPLOAD_PROGRESS);
	const uploadAbort = useRef<AbortController | null>(null);
	const files = useInvoke(
		storageApi.listStorageItems,
		storageApi,
		[appId, prefix],
		true,
		[storageScopeKey],
	);
	const refreshFiles = useCallback(async () => {
		const queryKey = [
			storageApi.listStorageItems.name || "backendFn",
			appId,
			prefix,
			storageScopeKey,
		];
		try {
			// A listing started before the mutation must not replace its result.
			await queryClient.cancelQueries({ queryKey, exact: true });
			await queryClient.invalidateQueries({
				queryKey,
				exact: true,
				refetchType: "none",
			});
			return await queryClient.fetchQuery({
				queryKey,
				queryFn: () =>
					storageApi.listStorageItems(appId, prefix, { refresh: true }),
				staleTime: 0,
			});
		} catch (error) {
			if (isCancelledError(error)) return;
			console.error("Failed to refresh storage:", error);
			toast.error("Failed to refresh files");
		}
	}, [queryClient, storageApi, appId, prefix, storageScopeKey]);

	// ---------- Virtual folders (sessionStorage) ----------
	const [creatingFolder, setCreatingFolder] = useState(false);
	const [newFolderName, setNewFolderName] = useState("");
	const [virtualFolders, setVirtualFolders] = useState<{
		storeKey: string;
		parentKey: string;
		names: string[];
	} | null>(null);

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
	const virtualFoldersHere =
		virtualFolders?.storeKey === storeKey &&
		virtualFolders.parentKey === currentParentKey
			? virtualFolders.names
			: NO_VIRTUAL_FOLDERS;
	const backendFolderNames = useMemo(
		() =>
			new Set(
				asArray(files.data)
					.filter((file) => file.is_dir)
					.map((file) => storageDisplayName(file.location)),
			),
		[files.data],
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
		const stored = map[currentParentKey] ?? [];
		const names = stored.filter(
			(name) => !backendFolderNames.has(storageDisplayName(name)),
		);
		// Once storage lists a folder, its session placeholder is no longer needed.
		if (names.length !== stored.length) {
			if (names.length > 0) map[currentParentKey] = names;
			else delete map[currentParentKey];
			writeVF(map);
		}
		setVirtualFolders({ storeKey, parentKey: currentParentKey, names });
	}, [backendFolderNames, currentParentKey, readVF, storeKey, writeVF]);

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
				Array.from(backendFolderNames, (name) => name.toLowerCase()),
			);
			for (const v of virtualFoldersHere)
				existingFolderNames.add(storageDisplayName(v).toLowerCase());
			if (existingFolderNames.has(storageDisplayName(clean).toLowerCase())) {
				toast.error("A folder with that name already exists");
				return false;
			}

			const all = readVF();
			const next = new Set(all[currentParentKey] ?? []);
			next.add(clean);
			all[currentParentKey] = Array.from(next);
			writeVF(all);
			setVirtualFolders({
				storeKey,
				parentKey: currentParentKey,
				names: all[currentParentKey],
			});
			toast.success("Folder created");
			return true;
		},
		[
			backendFolderNames,
			virtualFoldersHere,
			currentParentKey,
			readVF,
			storeKey,
			writeVF,
		],
	);

	const locked = useLockedStorage({
		appId,
		prefix,
		storageScopeKey,
		canWrite,
		storageItems: files.data,
		writeStorageItems: storageApi.writeStorageItems,
	});

	// Merge backend items with locked entries and virtual folders for current prefix
	const filesWithVirtual = useMemo<IStorageItem[]>(() => {
		const base = [...asArray(files.data), ...locked.entries];
		const have = new Set(base.map((file) => storageDisplayName(file.location)));
		const basePrefixNorm = normalizePrefix(prefix);
		const locFor = (name: string) =>
			basePrefixNorm ? `${basePrefixNorm}/${name}` : name;
		const virtualItems: IStorageItem[] = virtualFoldersHere
			.filter((name) => !have.has(storageDisplayName(name)))
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
	}, [files.data, locked.entries, virtualFoldersHere, prefix, normalizePrefix]);

	const renameInputId = useId();
	const renameContext = JSON.stringify([appId, storageScopeKey, prefix]);
	const latestRenameContext = useRef(renameContext);
	latestRenameContext.current = renameContext;
	const [renameTarget, setRenameTarget] = useState<{
		file: IStorageItem;
		context: string;
	} | null>(null);
	const [renameName, setRenameName] = useState("");
	const [renameError, setRenameError] = useState("");
	const [renaming, setRenaming] = useState(false);
	const renamePending = useRef(false);
	const activeRename =
		renameTarget?.context === renameContext ? renameTarget : null;
	useEffect(() => {
		setRenameTarget((current) =>
			current?.context === renameContext ? current : null,
		);
	}, [renameContext]);

	const startRename = useCallback(
		(file: IStorageItem) => {
			if (!canWrite || renamePending.current) return;
			setRenameName(storageDisplayName(file.location));
			setRenameError("");
			setRenameTarget({ file, context: renameContext });
		},
		[canWrite, renameContext],
	);

	const refreshAfterRename = useCallback(
		async (source: string, isDir: boolean) => {
			const listName =
				(storageScopeKey === "user"
					? backend.storageState.listStorageItemsUser
					: backend.storageState.listStorageItems
				).name || "backendFn";
			const queries = queryClient.getQueryCache().findAll({
				predicate: ({ queryKey }) =>
					queryKey[1] === appId &&
					typeof queryKey[2] === "string" &&
					((queryKey.length === 4 &&
						queryKey[0] === "listStorageItems" &&
						queryKey[3] === storageScopeKey) ||
						(queryKey.length === 3 &&
							queryKey[0] === listName &&
							(storageScopeKey === "shared" || storageScopeKey === "user"))),
			});
			const results = await Promise.allSettled(
				queries.map(async ({ queryKey }) => {
					await queryClient.cancelQueries({ queryKey, exact: true });
					await queryClient.invalidateQueries({
						queryKey,
						exact: true,
						refetchType: "none",
					});
					const directory = normalizePrefix(queryKey[2] as string);
					if (
						isDir &&
						(directory === source || directory.startsWith(`${source}/`))
					) {
						// Expanded folders must stop exposing paths removed by the rename.
						queryClient.setQueryData(queryKey, []);
						return;
					}
					await queryClient.fetchQuery({
						queryKey,
						queryFn: () =>
							storageApi.listStorageItems(appId, queryKey[2] as string, {
								refresh: true,
							}),
						staleTime: 0,
					});
				}),
			);
			if (results.some((result) => result.status === "rejected")) {
				toast.error("Renamed, but some folders could not be refreshed");
			}
			return await refreshFiles();
		},
		[
			appId,
			backend.storageState,
			normalizePrefix,
			queryClient,
			refreshFiles,
			storageApi,
			storageScopeKey,
		],
	);

	const submitRename = async () => {
		if (!canWrite || !activeRename || renamePending.current) return;
		const { file } = activeRename;
		const oldName = storageDisplayName(file.location);
		if (!renameName.trim()) {
			setRenameError("Name cannot be empty");
			return;
		}
		if (
			renameName === "." ||
			renameName === ".." ||
			/[\\/\p{Cc}]/u.test(renameName)
		) {
			setRenameError(
				"Use a name without slashes, control characters, or dot segments",
			);
			return;
		}
		if (renameName === oldName) {
			setRenameTarget(null);
			return;
		}
		if (
			filesWithVirtual.some(
				(candidate) =>
					candidate.location !== file.location &&
					storageDisplayName(candidate.location).toLowerCase() ===
						renameName.toLowerCase(),
			)
		) {
			setRenameError("A file or folder with that name already exists");
			return;
		}
		const source = normalizeStorageLocation(file.location, prefix);
		const previewPath = preview.file
			? normalizeStorageLocation(preview.file, preview.folder)
			: "";
		const renamingPreview =
			previewPath === source ||
			(file.is_dir && previewPath.startsWith(`${source}/`));
		renamePending.current = true;
		setRenaming(true);
		setRenameError("");
		let renameAttempted = false;
		try {
			if (renamingPreview && !confirmPreviewLeave()) {
				return;
			}
			// A download already in flight still refers to the old location.
			previewRequest.current++;
			const virtualOnly =
				file.is_dir &&
				virtualFoldersHere.some(
					(name) => storageDisplayName(name) === oldName,
				) &&
				!backendFolderNames.has(oldName);
			if (!virtualOnly) {
				renameAttempted = true;
				await storageApi.renameStorageItem(
					appId,
					source,
					renameName,
					!!file.is_dir,
				);
			}
			if (
				renamingPreview &&
				latestRenameContext.current === activeRename.context
			) {
				docxDirty.current = false;
				await closeHistory();
				dismissPreview();
			}
			const renamedItems = virtualOnly
				? undefined
				: await refreshAfterRename(source, !!file.is_dir);
			if (file.is_dir) {
				const renamedFolder = renamedItems?.find(
					(candidate) =>
						candidate.is_dir &&
						storageDisplayName(candidate.location) === renameName,
				);
				const destination = normalizeStorageLocation(
					renamedFolder?.location ?? renameName,
					prefix,
				);
				const folders = readVF();
				const moved: VFMap = Object.create(null);
				for (const [parent, names] of Object.entries(folders)) {
					const nextParent =
						parent === source || parent.startsWith(`${source}/`)
							? `${destination}${parent.slice(source.length)}`
							: parent;
					moved[nextParent] =
						parent === currentParentKey
							? names.map((name) =>
									storageDisplayName(name) === oldName ? renameName : name,
								)
							: names;
				}
				writeVF(moved);
				if (latestRenameContext.current === activeRename.context) {
					setVirtualFolders({
						storeKey,
						parentKey: currentParentKey,
						names: moved[currentParentKey] ?? [],
					});
				}
			}
			setRenameTarget((current) => (current === activeRename ? null : current));
			toast.success(file.is_dir ? "Folder renamed" : "File renamed");
		} catch (error) {
			setRenameError(
				error instanceof Error
					? error.message
					: typeof error === "string"
						? error
						: "Failed to rename item",
			);
			// A failed cloud folder rename can leave some destination copies.
			if (renameAttempted) await refreshFiles();
		} finally {
			renamePending.current = false;
			setRenaming(false);
		}
	};

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

			const settle = () => setUploadProgress(IDLE_UPLOAD_PROGRESS);

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
				void refreshFiles();
			}
		},
		[prefix, storageApi, appId, refreshFiles],
	);

	const loadFile = useCallback(
		async (file: string) => {
			if (preview.file === file) {
				closePreview();
				return;
			}
			if (!confirmPreviewLeave()) return;
			const request = ++previewRequest.current;

			const url = await storageApi.downloadStorageItems(appId, [file]);
			if (request !== previewRequest.current) return;

			if (url.length === 0 || !url[0]?.url) {
				toast.error("Failed to load file preview");
				return;
			}

			const fileUrl = url[0].url;
			const viewer = editorFor(file);
			if (viewer && !hasInlineMediaPreview(file)) openHistory();
			else if (!(await closeHistory())) return;
			if (request !== previewRequest.current) return;

			docxDirty.current = false;
			setInlineEditorOpen(false);
			conversionTarget.current = { source: file, id: crypto.randomUUID() };
			setPreview({
				...EMPTY_PREVIEW,
				url: fileUrl,
				file,
				folder: prefix,
				appId,
				storageScopeKey,
			});
		},
		[
			appId,
			closeHistory,
			closePreview,
			confirmPreviewLeave,
			editorFor,
			openHistory,
			prefix,
			preview.file,
			storageApi,
			storageScopeKey,
		],
	);

	const saveDocx = useCallback(
		async (file: File, onProgress?: BulkUploadProgressCallback) => {
			const viewer = editorFor(preview.file);
			if (!canWrite || !viewer) {
				throw new Error("You do not have permission to save this document.");
			}
			if (conversionTarget.current.source !== preview.file)
				conversionTarget.current = {
					source: preview.file,
					id: crypto.randomUUID(),
				};
			const document = createStorageCraftFile(
				file,
				preview.file,
				viewer,
				conversionTarget.current.id,
			);
			try {
				await storageApi.uploadStorageItems(
					appId,
					preview.folder,
					[document],
					onProgress,
				);
			} finally {
				await refreshFiles();
			}
			toast.success(`Saved ${storageDisplayName(document.name)}`);
		},
		[
			appId,
			canWrite,
			editorFor,
			preview.file,
			preview.folder,
			refreshFiles,
			storageApi,
		],
	);

	const saveFile = useCallback(
		async (fileContent: string) => {
			if (!preview.file) {
				toast.error("No file selected");
				return;
			}

			try {
				const blob = new Blob([fileContent], { type: "text/plain" });
				const fileName = storageDisplayName(preview.file) || "file";
				const file = new File([blob], fileName, { type: "text/plain" });

				await storageApi.uploadStorageItems(appId, prefix, [file], undefined);
			} catch (error) {
				console.error("Failed to save file:", error);
				throw error;
			} finally {
				await refreshFiles();
			}
		},
		[appId, prefix, preview.file, storageApi, refreshFiles],
	);

	const isFileEditable = useCallback((fileUrl: string, fileName?: string) => {
		return isCode(fileUrl, fileName) || isText(fileUrl, fileName);
	}, []);

	const previewName = useMemo(
		() => preview.name || storageDisplayName(preview.file),
		[preview.file, preview.name],
	);
	const previewViewer = preview.locked ? undefined : editorFor(preview.file);
	const hasInlinePreview = hasInlineMediaPreview(previewName);
	const previewEditable =
		!preview.locked && isFileEditable(preview.url, preview.file);
	const isEditorPreview =
		!!previewViewer && (!hasInlinePreview || inlineEditorOpen);
	const inlineEditAction = canWrite && previewViewer && hasInlinePreview && (
		<Button
			variant="outline"
			size="sm"
			aria-label={previewViewer.id === "pdfcraft" ? "Edit PDF" : "Edit file"}
			onClick={() => {
				openHistory();
				setInlineEditorOpen(true);
			}}
		>
			<PencilIcon className="size-4" /> Edit
		</Button>
	);

	const downloadFile = useCallback(
		async (file: string) => {
			if (preview.file === file) {
				closePreview();
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

			await saveUrlAsFile(
				signedUrl[0].url,
				storageDisplayName(file) || "downloaded_file",
			);
		},
		[appId, closePreview, preview, storageApi],
	);

	const openLockedFile = useCallback(
		async (entry: ILockedStorageEntry) => {
			const item = entry.lockedItem;
			const fileName = item.latest?.file_name ?? item.name;
			if (preview.file === entry.location) {
				closePreview();
				return;
			}
			if (!canPreview(fileName)) {
				locked.openDetails(item.id);
				return;
			}
			if (!confirmPreviewLeave()) return;
			const request = ++previewRequest.current;
			try {
				const ticket = await locked.downloadUrl(item);
				if (request !== previewRequest.current) return;
				if (!(await closeHistory())) return;
				docxDirty.current = false;
				setInlineEditorOpen(false);
				setPreview({
					...EMPTY_PREVIEW,
					url: ticket.url,
					file: entry.location,
					folder: prefix,
					appId,
					storageScopeKey,
					name: ticket.file_name,
					locked: true,
				});
			} catch (error) {
				console.error(`Previewing locked file ${item.id} failed:`, error);
				locked.openDetails(item.id);
			}
		},
		[
			appId,
			closeHistory,
			closePreview,
			confirmPreviewLeave,
			locked.downloadUrl,
			locked.openDetails,
			prefix,
			preview.file,
			storageScopeKey,
		],
	);

	const lockedRowActions = useMemo(() => {
		const latestTarget = (item: ILockedItem) =>
			item.latest ? { item, version: item.latest } : null;
		return {
			onDownload: (item: ILockedItem) => void locked.download(item),
			onShowVersions: (item: ILockedItem) => locked.openDetails(item.id),
			onUploadVersion: canWrite
				? (item: ILockedItem) => locked.openUpload({ item })
				: undefined,
			onExtendRetention: locked.canExtendRetention
				? (item: ILockedItem) => {
						const target = latestTarget(item);
						if (target) locked.openRetention(target);
					}
				: undefined,
			onToggleLegalHold: locked.canSetLegalHold
				? (item: ILockedItem) => {
						const target = latestTarget(item);
						if (target) void locked.toggleLegalHold(target);
					}
				: undefined,
			renderVersions: (item: ILockedItem) => (
				<LockedVersionList
					appId={locked.appId}
					item={item}
					queries={locked.queries}
					onDownload={(target, version) =>
						void locked.download(target, version)
					}
					onOpenDetails={locked.openDetails}
				/>
			),
		};
	}, [
		canWrite,
		locked.appId,
		locked.canExtendRetention,
		locked.canSetLegalHold,
		locked.download,
		locked.openDetails,
		locked.openRetention,
		locked.openUpload,
		locked.queries,
		locked.toggleLegalHold,
	]);

	const filteredFiles = useMemo(
		() =>
			filesWithVirtual?.filter((file) =>
				storageDisplayName(file.location)
					.toLowerCase()
					.includes(searchQuery.toLowerCase()),
			) ?? [],
		[filesWithVirtual, searchQuery],
	);

	const sortedFiles = useMemo(
		() =>
			[...filteredFiles].sort((a, b) => {
				const getName = (file: IStorageItem) =>
					storageDisplayName(file.location);
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

	const renderFile = useCallback(
		(file: IStorageItem, clearPreviewOnNavigate: boolean) =>
			isLockedEntry(file) ? (
				<LockedFileRow
					item={file.lockedItem}
					highlight={preview.file === file.location}
					actions={{
						...lockedRowActions,
						onOpen: () => void openLockedFile(file),
					}}
				/>
			) : (
				<FileOrFolder
					appId={appId}
					highlight={preview.file === file.location}
					file={file}
					previewable={editorFor(file.location) ? true : undefined}
					changePrefix={(newPrefix) => {
						if (isEditorPreview || clearPreviewOnNavigate) {
							navigatePrefix(`${prefix}/${newPrefix}`);
						} else {
							updatePrefix(`${prefix}/${newPrefix}`);
						}
					}}
					loadFile={loadFile}
					revealInExplorer={revealInExplorer}
					openWithApp={openWithApp}
					listAppsForFile={listAppsForFile}
					renameFile={canWrite ? startRename : undefined}
					deleteFile={async (target) => {
						const deletingPreview = storageDisplayName(preview.file) === target;
						if (deletingPreview && !confirmPreviewLeave()) return;
						try {
							await storageApi.deleteStorageItems(appId, [
								`${prefix}/${target}`,
							]);
							if (deletingPreview) {
								docxDirty.current = false;
								await closePreview();
							}
							toast.success("Deleted successfully");
						} catch (error) {
							console.error(error);
							toast.error("Failed to delete");
						} finally {
							await refreshFiles();
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
			canWrite,
			closePreview,
			confirmPreviewLeave,
			downloadFile,
			editorFor,
			refreshFiles,
			listAppsForFile,
			isEditorPreview,
			loadFile,
			lockedRowActions,
			navigatePrefix,
			openLockedFile,
			openWithApp,
			prefix,
			preview.file,
			revealInExplorer,
			storageApi,
			startRename,
			updatePrefix,
		],
	);

	const fileCount = filesWithVirtual?.filter((f) => !f.is_dir).length ?? 0;
	const folderCount = filesWithVirtual?.filter((f) => f.is_dir).length ?? 0;

	return (
		<div className="flex flex-col w-full h-full max-h-full overflow-hidden">
			{activeRename && (
				<Dialog
					open
					onOpenChange={(open) => {
						if (!open && !renamePending.current) setRenameTarget(null);
					}}
				>
					<DialogContent aria-describedby={undefined}>
						<DialogTitle>
							Rename {activeRename.file.is_dir ? "folder" : "file"}
						</DialogTitle>
						<form
							className="space-y-4"
							onSubmit={(event) => {
								event.preventDefault();
								void submitRename();
							}}
						>
							<div className="space-y-2">
								<label htmlFor={renameInputId} className="text-sm font-medium">
									Name
								</label>
								<Input
									id={renameInputId}
									autoFocus
									value={renameName}
									disabled={renaming || !canWrite}
									aria-invalid={!!renameError}
									aria-describedby={
										renameError ? `${renameInputId}-error` : undefined
									}
									onFocus={(event) => {
										const extension = activeRename.file.is_dir
											? -1
											: renameName.lastIndexOf(".");
										event.target.setSelectionRange(
											0,
											extension > 0 ? extension : renameName.length,
										);
									}}
									onChange={(event) => {
										setRenameName(event.target.value);
										setRenameError("");
									}}
								/>
								{renameError && (
									<p
										id={`${renameInputId}-error`}
										role="alert"
										className="text-sm text-destructive"
									>
										{renameError}
									</p>
								)}
							</div>
							<div className="flex justify-end gap-2">
								<Button
									type="button"
									variant="outline"
									disabled={renaming}
									onClick={() => setRenameTarget(null)}
								>
									Cancel
								</Button>
								<Button type="submit" disabled={renaming || !canWrite}>
									{renaming ? "Renaming..." : "Rename"}
								</Button>
							</div>
						</form>
					</DialogContent>
				</Dialog>
			)}
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

			{uploadProgress.isUploading && (
				<UploadProgressPanel
					label={
						uploadProgress.detail
							? `Uploading ${uploadProgress.detail.completedFiles.toLocaleString()} of ${uploadProgress.detail.totalFiles.toLocaleString()} files`
							: `Uploading ${uploadProgress.fileCount.toLocaleString()} file${uploadProgress.fileCount !== 1 ? "s" : ""}`
					}
					state={uploadProgress}
					cancelLabel="Cancel"
					onCancel={cancelUpload}
				/>
			)}
			<LockedStorageLayer controller={locked} />

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
						<Button
							variant="outline"
							size="icon"
							aria-label="Refresh files"
							title="Refresh files"
							disabled={files.isFetching}
							onClick={() => void refreshFiles()}
						>
							<RefreshCwIcon className="h-4 w-4" />
						</Button>
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

						{locked.enabled && canWrite && (
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										variant="outline"
										className="gap-2"
										onClick={() => locked.openUpload()}
									>
										<LockIcon className="h-4 w-4" />
										<span className="hidden sm:inline">
											{t("lockedStorage.uploadLocked", "Upload Locked")}
										</span>
									</Button>
								</TooltipTrigger>
								<TooltipContent>
									{t(
										"lockedStorage.uploadLockedHint",
										"Store write-once, versioned files that cannot be changed or deleted during retention",
									)}
								</TooltipContent>
							</Tooltip>
						)}

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
							updatePrefix={navigatePrefix}
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
			{(filesWithVirtual.length ?? 0) === 0 && !isEditorPreview && (
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

			{((filesWithVirtual.length ?? 0) > 0 || isEditorPreview) && (
				<div className="flex flex-col flex-1 min-h-0 px-4 pb-4">
					{preview.url !== "" && isEditorPreview && (
						<Dialog
							open
							onOpenChange={(open) => {
								if (!open) closePreview();
							}}
						>
							<DialogContent
								className="flex h-[calc(100dvh-2rem)] max-w-[calc(100vw-2rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-[calc(100vw-2rem)] [&>[data-slot=dialog-close]]:top-5"
								aria-describedby={undefined}
							>
								<div className="h-full min-h-0 flex-1 overflow-hidden">
									<Suspense
										fallback={
											<div className="px-3 py-2 pr-12">
												<DialogTitle className="truncate text-base">
													{previewName}
												</DialogTitle>
												<p className="mt-2 text-sm text-muted-foreground">
													Loading {previewViewer?.label}...
												</p>
											</div>
										}
									>
										{previewViewer?.id === "wordcraft" ? (
											<WordcraftDocument
												key={preview.file}
												toolTarget={previewToolTarget}
												url={preview.url}
												filename={previewName}
												title={
													<DialogTitle className="min-w-0 truncate text-base">
														{previewName}
													</DialogTitle>
												}
												editable={canWrite}
												onSave={saveDocx}
												onDirtyChange={onDocxDirtyChange}
												onBusyChange={onDocxBusyChange}
											/>
										) : previewViewer ? (
											<CraftDocument
												key={preview.file}
												toolTarget={previewToolTarget}
												viewer={previewViewer}
												initialEditing={inlineEditorOpen}
												outputFilename={storageDisplayName(
													storageCraftSaveName(
														preview.file,
														previewViewer,
														conversionTarget.current.id,
													),
												)}
												url={preview.url}
												filename={previewName}
												title={
													<DialogTitle className="min-w-0 truncate text-base">
														{previewName}
													</DialogTitle>
												}
												editable={canWrite}
												onSave={saveDocx}
												onDirtyChange={onDocxDirtyChange}
												onBusyChange={onDocxBusyChange}
											/>
										) : null}
									</Suspense>
								</div>
							</DialogContent>
						</Dialog>
					)}
					{preview.url !== "" && !isEditorPreview && (
						<>
							{(isSmallScreen || isPreviewMaximized) && (
								<div className="fixed inset-0 z-50 bg-background">
									<div className="flex flex-col h-full w-full">
										<div className="p-4 border-b bg-background flex items-center justify-between shrink-0">
											<h3 className="font-medium text-lg">
												Preview - {previewName}
											</h3>
											<div className="flex items-center gap-2">
												{inlineEditAction}
												<Button
													variant="ghost"
													size="sm"
													onClick={() => {
														if (isSmallScreen) {
															closePreview();
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
										</div>
										<div className="flex-1 min-h-0 overflow-auto">
											<FilePreviewer
												url={preview.url}
												filename={previewName}
												editable={previewEditable}
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
												<div className="flex items-center gap-2">
													{inlineEditAction}
													<Button
														variant="ghost"
														size="sm"
														onClick={() => setIsPreviewMaximized(true)}
														className="h-6 w-6 p-0"
													>
														<MaximizeIcon className="h-3 w-3" />
													</Button>
												</div>
											</div>
											<div className="flex-1 min-h-0 overflow-auto">
												<FilePreviewer
													url={preview.url}
													filename={previewName}
													editable={previewEditable}
													onSave={saveFile}
												/>
											</div>
										</div>
									</ResizablePanel>
								</ResizablePanelGroup>
							)}
						</>
					)}
					{(preview.url === "" || isEditorPreview) && (
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
