import { i18n as i18next, useTranslation } from "@flow-like/locales";
import SortAscIcon from "lucide-react/dist/esm/icons/arrow-up-narrow-wide.js";
import CheckIcon from "lucide-react/dist/esm/icons/check.js";
import ChevronLeftIcon from "lucide-react/dist/esm/icons/chevron-left.js";
import Download from "lucide-react/dist/esm/icons/download.js";
import ExternalLink from "lucide-react/dist/esm/icons/external-link.js";
import FileText from "lucide-react/dist/esm/icons/file-text.js";
import FilterIcon from "lucide-react/dist/esm/icons/funnel.js";
import GlobeIcon from "lucide-react/dist/esm/icons/globe.js";
import GridIcon from "lucide-react/dist/esm/icons/grid-3x3.js";
import ImageIcon from "lucide-react/dist/esm/icons/image.js";
import ListIcon from "lucide-react/dist/esm/icons/list.js";
import MaximizeIcon from "lucide-react/dist/esm/icons/maximize.js";
import MinimizeIcon from "lucide-react/dist/esm/icons/minimize.js";
import Music from "lucide-react/dist/esm/icons/music.js";
import SearchIcon from "lucide-react/dist/esm/icons/search.js";
import VideoIcon from "lucide-react/dist/esm/icons/video.js";
import XIcon from "lucide-react/dist/esm/icons/x.js";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useIsMobile } from "../../../hooks/use-mobile";
import { openExternalUrl } from "../../../lib/open-external";
import { isTauri } from "../../../lib/platform";
import { cn, humanFileSize } from "../../../lib/utils";
import {
	fetchFileBlob,
	saveBlobAsFile,
	shareFileOnIOS,
} from "../../storage/save-download";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "../../ui/dialog";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import {
	FilePreviewer,
	PdfFrame,
	canPreview,
	isCode,
	isText,
} from "../../ui/file-previewer";
import { Input } from "../../ui/input";
import {
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
} from "../../ui/resizable";
import { Separator } from "../../ui/separator";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui/tooltip";
import {
	type ProcessedAttachment,
	attachmentKey,
	getDisplayFileName,
} from "./attachment";
import {
	AttachmentCraftPreview,
	attachmentCraftViewer,
} from "./attachment-craft-preview";

type SortKey = "name" | "type" | "size";
type FilterKey = ProcessedAttachment["type"] | "all";

interface FileDialogProps {
	files: ProcessedAttachment[];
	handleFileClick: (file: ProcessedAttachment) => void;
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	initialSelectedFile?: ProcessedAttachment | null;
	trigger?: React.ReactNode;
}

/**
 * The classifier already resolved the kind from the declared mime type, so an
 * extension-less URL — a `blob:` handle, a signed key without a suffix — must not
 * demote a known media file back to a download.
 */
export const canPreviewFile = (file: ProcessedAttachment) => {
	if (
		file.type === "image" ||
		file.type === "video" ||
		file.type === "audio" ||
		file.type === "pdf"
	) {
		return true;
	}
	return (
		!!attachmentCraftViewer(file) ||
		canPreview(file.url, file.name || undefined)
	);
};

async function fetchAttachment(
	file: ProcessedAttachment,
	fileName: string,
): Promise<Blob | null> {
	const preparing = file.isDataUrl
		? undefined
		: toast.loading(
				i18next.t("common:preparingDownload", "Preparing download…"),
			);
	try {
		return await fetchFileBlob(file.url, fileName);
	} catch (error) {
		console.warn("Fetching the attachment failed:", error);
		if (!/^https?:/i.test(file.url)) throw error;
		await openExternalUrl(file.url);
		return null;
	} finally {
		if (preparing !== undefined) toast.dismiss(preparing);
	}
}

async function saveAttachment(file: ProcessedAttachment): Promise<void> {
	const fileName = file.name || "file";
	const blob = await fetchAttachment(file, fileName);
	if (!blob) return;

	if (await shareFileOnIOS(blob, fileName)) return;

	if (isTauri()) {
		try {
			const { save } = await import("@tauri-apps/plugin-dialog");
			const { writeFile } = await import("@tauri-apps/plugin-fs");
			const filePath = await save({
				defaultPath: fileName,
				filters: [
					{ name: i18next.t("allFiles", "All Files"), extensions: ["*"] },
				],
			});
			if (filePath) {
				await writeFile(filePath, new Uint8Array(await blob.arrayBuffer()));
			}
			return;
		} catch (e) {
			console.warn("Tauri save failed, falling back to browser download", e);
		}
	}

	saveBlobAsFile(blob, fileName);
}

/** Saves the file: the share sheet on iOS, a save dialog in the desktop app, a download in browsers. Never throws. */
export async function downloadFile(file: ProcessedAttachment): Promise<void> {
	try {
		await saveAttachment(file);
	} catch (error) {
		console.error("Saving the attachment failed:", error);
		toast.error(i18next.t("common:downloadFailed", "Download failed"));
	}
}

const opensAsLink = (file: ProcessedAttachment) => file.type === "website";

/**
 * The action button beside a file. It never previews: the dialog already does
 * that, and the chat's click handler would only re-select the open file.
 */
async function openOrSaveFile(file: ProcessedAttachment): Promise<void> {
	if (file.access) return;
	if (!opensAsLink(file)) return downloadFile(file);
	try {
		await openExternalUrl(file.url);
	} catch (error) {
		console.error("Opening the link failed:", error);
	}
}

export const getFileIcon = (type: ProcessedAttachment["type"]) => {
	switch (type) {
		case "image":
			return <ImageIcon className="w-4 h-4" />;
		case "video":
			return <VideoIcon className="w-4 h-4" />;
		case "audio":
			return <Music className="w-4 h-4" />;
		case "pdf":
			return <FileText className="w-4 h-4" />;
		case "document":
			return <FileText className="w-4 h-4" />;
		case "website":
			return <GlobeIcon className="w-4 h-4" />;
		default:
			return <Download className="w-4 h-4" />;
	}
};

const FileActionIcon = ({
	file,
	className = "w-3 h-3",
}: {
	file: ProcessedAttachment;
	className?: string;
}) =>
	opensAsLink(file) ? (
		<ExternalLink className={className} />
	) : (
		<Download className={className} />
	);

function useFileActionLabel() {
	const { t } = useTranslation("common");
	return useCallback(
		(file: ProcessedAttachment) =>
			opensAsLink(file) ? t("open", "Open") : t("download", "Download"),
		[t],
	);
}

export function FileDialog({
	files,
	handleFileClick,
	open: controlledOpen,
	onOpenChange,
	initialSelectedFile,
	trigger,
}: Readonly<FileDialogProps>) {
	const { t } = useTranslation("chat");
	const sortOptions: readonly { key: SortKey; label: string }[] = [
		{ key: "name", label: t("attachments.sort.name", "Name") },
		{ key: "type", label: t("attachments.sort.type", "Type") },
		{ key: "size", label: t("attachments.sort.size", "Size") },
	];
	const filterOptions: readonly { key: FilterKey; label: string }[] = [
		{ key: "all", label: t("attachments.filter.all", "All files") },
		{ key: "image", label: t("attachments.filter.image", "Images") },
		{ key: "video", label: t("attachments.filter.video", "Videos") },
		{ key: "audio", label: t("attachments.filter.audio", "Audio") },
		{ key: "pdf", label: t("attachments.filter.pdf", "PDFs") },
		{ key: "document", label: t("attachments.filter.document", "Documents") },
		{ key: "website", label: t("attachments.filter.website", "Websites") },
	];

	const [internalOpen, setInternalOpen] = useState(false);
	const [searchQuery, setSearchQuery] = useState("");
	const [viewMode, setViewMode] = useState<"grid" | "list">("list");
	const [sortBy, setSortBy] = useState<SortKey>("name");
	const [sortOrder, setSortOrder] = useState<"asc" | "desc">("asc");
	const [filterType, setFilterType] = useState<FilterKey>("all");
	const [pickedFile, setSelectedFile] = useState<ProcessedAttachment | null>(
		initialSelectedFile ?? null,
	);
	const selectedFile = useMemo(
		() =>
			pickedFile
				? (files.find(
						(file) => attachmentKey(file) === attachmentKey(pickedFile),
					) ?? pickedFile)
				: null,
		[files, pickedFile],
	);
	const [isPreviewMaximized, setIsPreviewMaximized] = useState(false);
	const isMobile = useIsMobile();
	const actionLabel = useFileActionLabel();

	const isControlled = controlledOpen !== undefined;
	const isOpen = isControlled ? controlledOpen : internalOpen;

	const handleOpenChange = useCallback(
		(open: boolean) => {
			if (!isControlled) {
				setInternalOpen(open);
			}
			onOpenChange?.(open);
			if (open && initialSelectedFile) {
				setSelectedFile(initialSelectedFile);
			}
		},
		[isControlled, onOpenChange, initialSelectedFile],
	);

	// Re-arm the selection on every open, not just when the file changes — on
	// phones the preview is the whole sheet, so tapping the same attachment twice
	// has to land on the preview again after the user navigated back.
	useEffect(() => {
		if (isOpen && initialSelectedFile) {
			setSelectedFile(initialSelectedFile);
		}
	}, [isOpen, initialSelectedFile]);

	const filteredFiles = useMemo(() => {
		return files.filter((file) => {
			const matchesSearch =
				file.name?.toLowerCase().includes(searchQuery.toLowerCase()) ?? true;
			const matchesType = filterType === "all" || file.type === filterType;
			return matchesSearch && matchesType;
		});
	}, [files, searchQuery, filterType]);

	const sortedFiles = useMemo(() => {
		return [...filteredFiles].sort((a, b) => {
			let comparison = 0;

			switch (sortBy) {
				case "name":
					comparison = (a.name ?? "").localeCompare(b.name ?? "");
					break;
				case "type":
					comparison = a.type.localeCompare(b.type);
					break;
				case "size":
					comparison = (a.size ?? 0) - (b.size ?? 0);
					break;
			}

			return sortOrder === "asc" ? comparison : -comparison;
		});
	}, [filteredFiles, sortBy, sortOrder]);

	const canPreview = useCallback((file: ProcessedAttachment) => {
		return canPreviewFile(file);
	}, []);

	const fileTypeCount = useMemo(() => {
		const counts: Record<string, number> = {};
		for (const file of filteredFiles) {
			counts[file.type] = (counts[file.type] || 0) + 1;
		}
		return counts;
	}, [filteredFiles]);

	const handleFileSelect = useCallback(
		(file: ProcessedAttachment) => {
			if (canPreview(file)) {
				setSelectedFile(
					selectedFile && attachmentKey(selectedFile) === attachmentKey(file)
						? null
						: file,
				);
			} else {
				handleFileClick(file);
			}
		},
		[selectedFile, canPreview, handleFileClick],
	);

	const toggleSort = useCallback(
		(key: SortKey) => {
			setSortBy(key);
			setSortOrder(sortBy === key && sortOrder === "asc" ? "desc" : "asc");
		},
		[sortBy, sortOrder],
	);

	const defaultTrigger = (
		<button
			type="button"
			className="text-muted-foreground hover:text-foreground transition-colors"
		>
			<Badge
				variant="secondary"
				className="cursor-pointer hover:bg-secondary/80 transition-colors gap-1 text-xs h-6 rounded-full"
			>
				<FileText className="w-3 h-3" />
				{files.length}
			</Badge>
		</button>
	);

	// On phones the preview takes over the whole sheet, so the browsing chrome
	// (counts, search, sorting) collapses while a file is open.
	const previewTakesOver = isMobile && !!selectedFile;

	const fileList = (
		<FileList
			files={sortedFiles}
			viewMode={viewMode}
			selectedFile={isMobile ? null : selectedFile}
			onFileSelect={handleFileSelect}
			canPreview={canPreview}
		/>
	);

	return (
		<TooltipProvider>
			<div>
				<Dialog open={isOpen} onOpenChange={handleOpenChange}>
					{trigger !== null && (
						<DialogTrigger asChild>{trigger ?? defaultTrigger}</DialogTrigger>
					)}
					<DialogContent
						showCloseButton={false}
						className={cn(
							"flex flex-col gap-0 overflow-hidden p-0",
							"h-dvh max-h-dvh w-dvw max-w-dvw rounded-none border-0 sm:max-w-dvw",
							"md:h-[calc(100dvh-5rem)] md:max-h-[calc(100dvh-5rem)] md:w-[calc(100dvw-5rem)] md:max-w-[calc(100dvw-5rem)] md:gap-4 md:rounded-xl md:border md:p-6",
						)}
						style={
							isMobile
								? {
										paddingTop: "var(--fl-safe-top, 0px)",
										paddingBottom: "var(--fl-safe-bottom, 0px)",
									}
								: undefined
						}
					>
						<DialogHeader className="shrink-0 px-3 pt-3 md:p-0">
							<div className="flex flex-row items-center gap-1 md:gap-2">
								{previewTakesOver && (
									<Button
										variant="ghost"
										size="icon"
										className="size-10 shrink-0"
										onClick={() => setSelectedFile(null)}
									>
										<ChevronLeftIcon className="size-5" />
										<span className="sr-only">
											{t("backToFiles", "Back to files")}
										</span>
									</Button>
								)}
								<DialogTitle className="flex min-w-0 flex-1 items-center gap-2 text-base md:text-lg">
									{previewTakesOver && selectedFile ? (
										<span className="truncate">
											{getDisplayFileName(selectedFile.name)}
										</span>
									) : (
										<>
											<FileText className="size-4 shrink-0" />
											<span className="truncate">
												{t("referencesLength", "References ({{length}})", {
													length: files.length,
												})}
											</span>
										</>
									)}
								</DialogTitle>
								{previewTakesOver && selectedFile && (
									<Button
										variant="ghost"
										size="icon"
										className="size-10 shrink-0"
										disabled={!!selectedFile.access}
										onClick={() => void openOrSaveFile(selectedFile)}
									>
										<FileActionIcon file={selectedFile} className="size-4" />
										<span className="sr-only">{actionLabel(selectedFile)}</span>
									</Button>
								)}
								<DialogClose asChild>
									<Button
										variant="ghost"
										size="icon"
										className="size-10 shrink-0 md:size-9"
									>
										<XIcon className="size-4" />
										<span className="sr-only">{t("close", "Close")}</span>
									</Button>
								</DialogClose>
							</div>
						</DialogHeader>

						{/* Header Controls */}
						{!previewTakesOver && (
							<div className="flex shrink-0 flex-col gap-2 px-3 pt-2 md:gap-4 md:px-0 md:pt-0">
								<div className="-mx-1 flex flex-row items-center gap-2 overflow-x-auto px-1 pb-0.5 text-sm text-muted-foreground md:mx-0 md:flex-wrap md:overflow-x-visible md:px-0">
									<Badge variant="secondary" className="shrink-0 px-2 py-1">
										{t("lengthFiles", "{{length}} files", {
											length: filteredFiles.length,
										})}
									</Badge>
									{filterType !== "all" && (
										<Badge
											variant="default"
											className="shrink-0 px-2 py-1 capitalize"
										>
											{t("filterFiltertype", "Filter: {{filterType}}", {
												filterType,
											})}
										</Badge>
									)}
									{Object.entries(fileTypeCount).map(([type, count]) => (
										<Badge
											key={type}
											variant="outline"
											className="shrink-0 px-2 py-1 capitalize"
										>
											{count} {type}
										</Badge>
									))}
								</div>

								<div className="flex flex-row items-center gap-2">
									{/* Search */}
									<div className="relative min-w-0 flex-1">
										<SearchIcon className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
										<Input
											placeholder={t("searchFiles", "Search files...")}
											className="h-10 pl-10 md:h-9"
											value={searchQuery}
											onChange={(e) => setSearchQuery(e.target.value)}
										/>
									</div>

									<div className="hidden items-center gap-1 text-sm text-muted-foreground lg:flex">
										<span>{t("sortBy", "Sort by:")}</span>
										<span className="font-medium text-foreground capitalize">
											{sortBy}
										</span>
										<span className="text-xs">
											{sortOrder === "asc" ? "↑" : "↓"}
										</span>
									</div>

									{/* Sort Controls */}
									<DropdownMenu>
										<DropdownMenuTrigger asChild>
											<Button
												variant="outline"
												size="icon"
												className="size-10 shrink-0 md:size-9"
											>
												<SortAscIcon className="h-4 w-4" />
												<span className="sr-only">
													{t("sortFiles", "Sort files")}
												</span>
											</Button>
										</DropdownMenuTrigger>
										<DropdownMenuContent align="end">
											{sortOptions.map(({ key, label }) => (
												<DropdownMenuItem
													key={key}
													onClick={() => toggleSort(key)}
													className="flex items-center justify-between"
												>
													{label}
													{sortBy === key && (
														<span className="text-xs text-muted-foreground">
															{sortOrder === "asc" ? " ↑" : " ↓"}
														</span>
													)}
												</DropdownMenuItem>
											))}
										</DropdownMenuContent>
									</DropdownMenu>

									{/* View Mode Toggle */}
									<Tooltip>
										<TooltipTrigger asChild>
											<Button
												variant="outline"
												size="icon"
												className="size-10 shrink-0 md:size-9"
												onClick={() =>
													setViewMode(viewMode === "grid" ? "list" : "grid")
												}
											>
												{viewMode === "grid" ? (
													<ListIcon className="h-4 w-4" />
												) : (
													<GridIcon className="h-4 w-4" />
												)}
												<span className="sr-only">
													{viewMode === "grid"
														? t(
																"attachments.switchToList",
																"Switch to list view",
															)
														: t(
																"attachments.switchToGrid",
																"Switch to grid view",
															)}
												</span>
											</Button>
										</TooltipTrigger>
										<TooltipContent>
											{viewMode === "grid"
												? t("attachments.switchToList", "Switch to list view")
												: t("attachments.switchToGrid", "Switch to grid view")}
										</TooltipContent>
									</Tooltip>

									<Separator
										orientation="vertical"
										className="hidden h-6 md:block"
									/>

									{/* Filter by Type */}
									<DropdownMenu>
										<DropdownMenuTrigger asChild>
											<Button
												variant="outline"
												size="icon"
												className="size-10 shrink-0 md:size-9"
											>
												<FilterIcon className="h-4 w-4" />
												<span className="sr-only">
													{t("filterFiles", "Filter files")}
												</span>
											</Button>
										</DropdownMenuTrigger>
										<DropdownMenuContent align="end">
											<DropdownMenuLabel>
												{t("filterByType", "Filter by Type")}
											</DropdownMenuLabel>
											<DropdownMenuSeparator />
											{filterOptions.map(({ key, label }) => (
												<DropdownMenuItem
													key={key}
													onClick={() => setFilterType(key)}
												>
													{filterType === key && (
														<CheckIcon className="w-4 h-4 mr-2" />
													)}
													{label}
												</DropdownMenuItem>
											))}
										</DropdownMenuContent>
									</DropdownMenu>
								</div>
							</div>
						)}

						{!previewTakesOver && (
							<Separator className="mt-2 shrink-0 md:mt-0" />
						)}

						{/* Content Section */}
						<div className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden">
							{!isMobile && isPreviewMaximized && selectedFile && (
								<div className="fixed inset-0 z-50 bg-background flex grow flex-col h-full">
									<div className="p-4 border-b bg-background flex items-center justify-between">
										<h3 className="font-medium text-lg">
											{t("preview", "Preview -")}{" "}
											{getDisplayFileName(selectedFile.name)}
										</h3>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => setIsPreviewMaximized(false)}
											className="h-8 w-8 p-0"
										>
											<MinimizeIcon className="h-4 w-4" />
										</Button>
									</div>
									<div className="flex flex-col grow overflow-auto h-full min-h-full">
										<FileDialogPreview file={selectedFile} maximized={true} />
									</div>
								</div>
							)}

							{isMobile && selectedFile && (
								<div className="min-h-0 flex-1 overflow-hidden bg-muted/20">
									<FileDialogPreview
										key={attachmentKey(selectedFile)}
										file={selectedFile}
									/>
								</div>
							)}

							{isMobile && !selectedFile && (
								<div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-3 py-3">
									{fileList}
								</div>
							)}

							{!isMobile && selectedFile && (
								<ResizablePanelGroup
									direction="horizontal"
									autoSaveId="attachment_viewer"
									className="border rounded-lg flex-1 min-h-0"
								>
									<ResizablePanel
										defaultSize={60}
										className="flex flex-col gap-2 overflow-hidden p-4 bg-background"
									>
										<div className="flex flex-col flex-1 min-h-0 gap-2">
											<h3 className="font-medium text-sm text-muted-foreground mb-2 shrink-0">
												{t("filesReferences", "Files & References")}
											</h3>
											<div className="flex flex-col gap-2 flex-1 min-h-0 overflow-auto">
												{fileList}
											</div>
										</div>
									</ResizablePanel>
									<ResizableHandle className="mx-2" />
									<ResizablePanel
										defaultSize={40}
										className="flex flex-col gap-2 p-4 bg-background min-h-0"
									>
										<div className="flex flex-col flex-1 min-h-0 bg-muted/50 rounded-md border">
											<div className="p-2 border-b bg-background rounded-t-md flex items-center justify-between shrink-0">
												<h3 className="font-medium text-sm">
													{t("preview2", "Preview")}
												</h3>
												<Button
													variant="ghost"
													size="sm"
													onClick={() => setIsPreviewMaximized(true)}
													className="h-6 w-6 p-0"
												>
													<MaximizeIcon className="h-3 w-3" />
												</Button>
											</div>
											<div className="flex-1 min-h-0 overflow-hidden">
												<FileDialogPreview
													key={attachmentKey(selectedFile)}
													file={selectedFile}
												/>
											</div>
										</div>
									</ResizablePanel>
								</ResizablePanelGroup>
							)}

							{!isMobile && !selectedFile && (
								<div className="flex flex-col grow overflow-auto gap-2 border rounded-lg p-4 bg-background">
									<h3 className="font-medium text-sm text-muted-foreground mb-2">
										{t("filesReferences", "Files & References")}
									</h3>
									{fileList}
								</div>
							)}
						</div>
					</DialogContent>
				</Dialog>
			</div>
		</TooltipProvider>
	);
}

interface FileListProps {
	files: ProcessedAttachment[];
	viewMode: "grid" | "list";
	selectedFile: ProcessedAttachment | null;
	onFileSelect: (file: ProcessedAttachment) => void;
	canPreview: (file: ProcessedAttachment) => boolean;
}

function FileList({
	files,
	viewMode,
	selectedFile,
	onFileSelect,
	canPreview,
}: FileListProps) {
	const { t } = useTranslation("chat");
	if (files.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
				<FileText className="h-6 w-6 text-muted-foreground" />
				<p className="text-sm text-muted-foreground">
					{t("noMatchingFiles", "No matching files")}
				</p>
			</div>
		);
	}

	return (
		<div
			className={`grid gap-2 ${viewMode === "grid" ? "grid-cols-2 md:grid-cols-3 lg:grid-cols-4" : "grid-cols-1"}`}
		>
			{files.map((file) => (
				<FileItem
					grid={viewMode === "grid"}
					key={attachmentKey(file)}
					file={file}
					isSelected={
						selectedFile !== null &&
						attachmentKey(selectedFile) === attachmentKey(file)
					}
					onSelect={onFileSelect}
					canPreview={canPreview}
				/>
			))}
		</div>
	);
}

interface FileItemProps {
	grid: boolean;
	file: ProcessedAttachment;
	isSelected: boolean;
	onSelect: (file: ProcessedAttachment) => void;
	canPreview: (file: ProcessedAttachment) => boolean;
}

const FileThumbnail = ({
	file,
	canPreview,
	size,
}: {
	file: ProcessedAttachment;
	canPreview: boolean;
	size: "sm" | "md";
}) => {
	if (file.type === "image") {
		return (
			<div
				className={cn(
					"relative flex items-center justify-center overflow-hidden rounded-md",
					size === "sm" ? "h-8 w-8" : "h-10 w-10",
				)}
			>
				<img
					src={file.url}
					alt={file.name}
					className="w-full h-full object-cover rounded-sm"
					onError={(e) => {
						// Fallback to icon if image fails to load
						e.currentTarget.style.display = "none";
						const iconElement = e.currentTarget
							.nextElementSibling as HTMLElement;
						if (iconElement) iconElement.style.display = "block";
					}}
				/>
				<div className="hidden">{getFileIcon(file.type)}</div>
			</div>
		);
	}

	return (
		<div
			className={cn(
				"relative overflow-hidden rounded-md transition-colors",
				size === "sm" ? "p-2" : "p-3",
				canPreview ? "bg-primary/10 group-hover:bg-primary/20" : "bg-muted/50",
			)}
		>
			{getFileIcon(file.type)}
		</div>
	);
};

const FileMetaBadges = ({
	file,
	align,
}: {
	file: ProcessedAttachment;
	align: "center" | "start";
}) => (
	<div
		className={cn(
			"flex flex-wrap items-center gap-1",
			align === "center" ? "justify-center" : "mt-1",
		)}
	>
		<Badge variant="outline" className="text-xs px-1 py-0 h-4 capitalize">
			{file.type}
		</Badge>
		{file.pageNumber !== undefined && (
			<Badge variant="secondary" className="text-xs px-1 py-0 h-4">
				{i18next.t("pagePagenumber", "Page {{pageNumber}}", {
					pageNumber: file.pageNumber,
				})}
			</Badge>
		)}
		{file.size && (
			<Badge variant="secondary" className="text-xs px-1 py-0 h-4">
				{humanFileSize(file.size)}
			</Badge>
		)}
	</div>
);

function FileItem({
	grid,
	file,
	isSelected,
	onSelect,
	canPreview,
}: Readonly<FileItemProps>) {
	const actionLabel = useFileActionLabel()(file);
	const previewable = canPreview(file);
	const cardClasses = cn(
		"group relative w-full rounded-lg border border-border/50 bg-linear-to-r from-background to-muted/10 transition-all duration-200",
		isSelected && "border-primary bg-primary/5",
		previewable ? "hover:border-primary/50" : "opacity-75",
	);

	// Touch devices never get :hover, so the action button stays visible there and
	// only fades in on pointer devices.
	const actionVisibility =
		"opacity-100 transition-opacity md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100";

	if (grid) {
		return (
			<div className={cn(cardClasses, "p-2")}>
				<button
					type="button"
					className="w-full flex flex-col items-center gap-2"
					onClick={() => onSelect(file)}
				>
					<FileThumbnail file={file} canPreview={previewable} size="md" />
					<div className="flex flex-col items-center gap-1 w-full overflow-hidden">
						<p className="max-w-full w-full text-center font-medium text-foreground text-xs leading-tight line-clamp-1">
							{getDisplayFileName(file.name)}
						</p>
						{file.previewText && (
							<p className="text-xs text-muted-foreground line-clamp-1 w-full text-center">
								{file.previewText}
							</p>
						)}
						<FileMetaBadges file={file} align="center" />
					</div>
				</button>

				{/* Download/Open button as overlay */}
				<Button
					variant="outline"
					size="icon"
					disabled={!!file.access}
					onClick={(e) => {
						e.stopPropagation();
						void openOrSaveFile(file);
					}}
					className={cn(
						"absolute top-1 right-1 size-8 md:top-2 md:right-2 md:size-7",
						actionVisibility,
					)}
				>
					<FileActionIcon file={file} />
					<span className="sr-only">{actionLabel}</span>
				</Button>
			</div>
		);
	}

	// List mode
	return (
		<div className={cn(cardClasses, "flex flex-row items-center gap-2 p-2.5")}>
			<button
				type="button"
				className="flex min-w-0 flex-1 flex-row items-center gap-3 text-left"
				onClick={() => onSelect(file)}
			>
				<FileThumbnail file={file} canPreview={previewable} size="sm" />
				<div className="flex min-w-0 flex-1 flex-col items-start">
					<p className="line-clamp-1 w-full text-start text-sm font-medium text-foreground">
						{getDisplayFileName(file.name)}
					</p>
					{file.previewText && (
						<p className="text-xs text-muted-foreground line-clamp-1 w-full mt-0.5">
							{file.previewText}
						</p>
					)}
					<FileMetaBadges file={file} align="start" />
				</div>
			</button>
			<Button
				variant="outline"
				size="sm"
				disabled={!!file.access}
				onClick={(e) => {
					e.stopPropagation();
					void openOrSaveFile(file);
				}}
				className={cn("h-9 shrink-0 gap-1 px-2.5 md:h-8", actionVisibility)}
			>
				<FileActionIcon file={file} />
				<span className="hidden sm:inline">{actionLabel}</span>
				<span className="sr-only sm:hidden">{actionLabel}</span>
			</Button>
		</div>
	);
}

interface FileDialogPreviewProps {
	file: ProcessedAttachment;
	maximized?: boolean;
}

export function FileDialogPreview({ file }: Readonly<FileDialogPreviewProps>) {
	const { t } = useTranslation("chat");
	const actionLabel = useFileActionLabel()(file);

	const imageClasses = "max-w-full max-h-full object-contain rounded-md";
	const videoClasses = "max-w-full max-h-full rounded-md";
	const showTextPreview = isText(file.url) || isCode(file.url);
	const noPreview = (
		<div className="flex flex-col items-center justify-center gap-4 h-full p-4 text-center md:p-8">
			{getFileIcon(file.type)}
			<p className="text-sm text-muted-foreground">
				{t(
					"previewNotAvailableForThisFileType",
					"Preview not available for this file type",
				)}
			</p>
			<Button
				variant="outline"
				disabled={!!file.access}
				onClick={() => void openOrSaveFile(file)}
				className="gap-2"
			>
				<FileActionIcon file={file} className="w-4 h-4" />
				{actionLabel}
			</Button>
		</div>
	);

	switch (file.type) {
		case "image":
			return (
				<div className="flex justify-center items-center h-full p-2 md:p-4">
					<img src={file.url} alt={file.name} className={imageClasses} />
				</div>
			);
		case "video":
			return (
				<div className="flex justify-center items-center h-full p-2 md:p-4">
					{/* biome-ignore lint/a11y/useMediaCaption: chat attachments carry no caption track */}
					<video controls className={videoClasses} poster={file.thumbnailUrl}>
						<source src={file.url} />
						Your browser does not support the video tag.
					</video>
				</div>
			);
		case "audio":
			return (
				<div className="flex flex-col items-center justify-center gap-4 h-full p-4 md:p-8">
					<Music className="w-12 h-12 text-muted-foreground md:w-16 md:h-16" />
					<p className="text-base font-medium text-center break-all md:text-lg">
						{getDisplayFileName(file.name)}
					</p>
					{/* biome-ignore lint/a11y/useMediaCaption: chat attachments carry no caption track */}
					<audio controls className="w-full max-w-md">
						<source src={file.url} />
						Your browser does not support the audio tag.
					</audio>
				</div>
			);
		case "pdf":
			return (
				<div className="w-full h-full">
					<PdfFrame
						url={file.url}
						page={file.pageNumber}
						filename={file.name}
					/>
				</div>
			);
		case "document":
		case "other": {
			const craft = attachmentCraftViewer(file);
			if (craft) {
				return (
					<AttachmentCraftPreview
						file={file}
						viewer={craft.viewer}
						filename={craft.filename}
					/>
				);
			}
			if (showTextPreview) {
				return (
					<div className="w-full h-full">
						<FilePreviewer url={file.url} />
					</div>
				);
			}
			return noPreview;
		}
		default:
			return noPreview;
	}
}
