import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	ChevronLeftIcon,
	Download,
	ExternalLink,
	FileText,
	FilterIcon,
	GlobeIcon,
	GridIcon,
	ImageIcon,
	ListIcon,
	MaximizeIcon,
	MinimizeIcon,
	Music,
	SearchIcon,
	SortAscIcon,
	VideoIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useIsMobile } from "../../../hooks/use-mobile";
import { cn, humanFileSize } from "../../../lib";
import {
	Badge,
	Button,
	Dialog,
	DialogClose,
	DialogContent,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	Input,
	ResizableHandle,
	ResizablePanel,
	ResizablePanelGroup,
	Separator,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui";
import {
	FilePreviewer,
	PdfFrame,
	canPreview,
	isCode,
	isText,
} from "../../ui/file-previewer";
import { type ProcessedAttachment, getDisplayFileName } from "./attachment";

type SortKey = "name" | "type" | "size";
type FilterKey = ProcessedAttachment["type"] | "all";

const SORT_OPTIONS: readonly { key: SortKey; label: string }[] = [
	{ key: "name", label: "Name" },
	{ key: "type", label: "Type" },
	{ key: "size", label: "Size" },
];

const FILTER_OPTIONS: readonly { key: FilterKey; label: string }[] = [
	{ key: "all", label: "All Files" },
	{ key: "image", label: "Images" },
	{ key: "video", label: "Videos" },
	{ key: "audio", label: "Audio" },
	{ key: "pdf", label: "PDFs" },
	{ key: "document", label: "Documents" },
	{ key: "website", label: "Websites" },
];

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
	return canPreview(file.url, file.name || undefined);
};

export async function downloadFile(file: ProcessedAttachment): Promise<void> {
	const isTauriEnv =
		typeof window !== "undefined" &&
		!!(
			(window as any).__TAURI__ ||
			(window as any).__TAURI_IPC__ ||
			(window as any).__TAURI_INTERNALS__
		);

	if (isTauriEnv) {
		try {
			const { save } = await import("@tauri-apps/plugin-dialog");
			const { writeFile } = await import("@tauri-apps/plugin-fs");

			const response = await fetch(file.url);
			const blob = await response.blob();
			const arrayBuffer = await blob.arrayBuffer();

			const filePath = await save({
				defaultPath: file.name,
				filters: [{ name: i18next.t('allFiles', 'All Files'), extensions: ["*"] }],
			});

			if (filePath) {
				await writeFile(filePath, new Uint8Array(arrayBuffer));
			}
			return;
		} catch (e) {
			console.warn("Tauri save failed, falling back to browser download", e);
		}
	}

	// Browser fallback - fetch and trigger download
	try {
		const response = await fetch(file.url);
		const blob = await response.blob();
		const blobUrl = URL.createObjectURL(blob);

		const link = document.createElement("a");
		link.href = blobUrl;
		link.download = file.name || "file";
		document.body.appendChild(link);
		link.click();
		document.body.removeChild(link);
		URL.revokeObjectURL(blobUrl);
	} catch (e) {
		// Ultimate fallback - open in new tab
		window.open(file.url, "_blank", "noopener,noreferrer");
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
	file.isDataUrl ? (
		<Download className={className} />
	) : (
		<ExternalLink className={className} />
	);

export function FileDialog({
	files,
	handleFileClick,
	open: controlledOpen,
	onOpenChange,
	initialSelectedFile,
	trigger,
}: Readonly<FileDialogProps>) {
	const { t } = useTranslation("chat");
	const [internalOpen, setInternalOpen] = useState(false);
	const [searchQuery, setSearchQuery] = useState("");
	const [viewMode, setViewMode] = useState<"grid" | "list">("list");
	const [sortBy, setSortBy] = useState<SortKey>("name");
	const [sortOrder, setSortOrder] = useState<"asc" | "desc">("asc");
	const [filterType, setFilterType] = useState<FilterKey>("all");
	const [selectedFile, setSelectedFile] = useState<ProcessedAttachment | null>(
		initialSelectedFile ?? null,
	);
	const [isPreviewMaximized, setIsPreviewMaximized] = useState(false);
	const isMobile = useIsMobile();

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
		filteredFiles.forEach((file) => {
			counts[file.type] = (counts[file.type] || 0) + 1;
		});
		return counts;
	}, [filteredFiles]);

	const handleFileSelect = useCallback(
		(file: ProcessedAttachment) => {
			if (canPreview(file)) {
				setSelectedFile(selectedFile?.url === file.url ? null : file);
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
		<button className="text-muted-foreground hover:text-foreground transition-colors">
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
			handleFileClick={handleFileClick}
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
										<span className="sr-only">{t('backToFiles', 'Back to files')}</span>
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
											<span className="truncate">{t('referencesLength', 'References ({{length}})', { length: files.length })}</span>
										</>
									)}
								</DialogTitle>
								{previewTakesOver && selectedFile && (
									<Button
										variant="ghost"
										size="icon"
										className="size-10 shrink-0"
										onClick={() => handleFileClick(selectedFile)}
									>
										<FileActionIcon file={selectedFile} className="size-4" />
										<span className="sr-only">
											{selectedFile.isDataUrl ? "Download" : "Open"}
										</span>
									</Button>
								)}
								<DialogClose asChild>
									<Button
										variant="ghost"
										size="icon"
										className="size-10 shrink-0 md:size-9"
									>
										<XIcon className="size-4" />
										<span className="sr-only">{t('close', 'Close')}</span>
									</Button>
								</DialogClose>
							</div>
						</DialogHeader>

						{/* Header Controls */}
						{!previewTakesOver && (
							<div className="flex shrink-0 flex-col gap-2 px-3 pt-2 md:gap-4 md:px-0 md:pt-0">
								<div className="-mx-1 flex flex-row items-center gap-2 overflow-x-auto px-1 pb-0.5 text-sm text-muted-foreground md:mx-0 md:flex-wrap md:overflow-x-visible md:px-0">
									<Badge variant="secondary" className="shrink-0 px-2 py-1">{t('lengthFiles', '{{length}} files', { length: filteredFiles.length })}</Badge>
									{filterType !== "all" && (
										<Badge
											variant="default"
											className="shrink-0 px-2 py-1 capitalize"
										>{t('filterFiltertype', 'Filter: {{filterType}}', { filterType })}</Badge>
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
											placeholder={t('searchFiles', 'Search files...')}
											className="h-10 pl-10 md:h-9"
											value={searchQuery}
											onChange={(e) => setSearchQuery(e.target.value)}
										/>
									</div>

									<div className="hidden items-center gap-1 text-sm text-muted-foreground lg:flex">
										<span>{t('sortBy', 'Sort by:')}</span>
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
												<span className="sr-only">{t('sortFiles', 'Sort files')}</span>
											</Button>
										</DropdownMenuTrigger>
										<DropdownMenuContent align="end">
											{SORT_OPTIONS.map(({ key, label }) => (
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
													{t('switchTo', 'Switch to')} {viewMode === "grid" ? "list" : "grid"} view
												</span>
											</Button>
										</TooltipTrigger>
										<TooltipContent>
											{t('switchTo', 'Switch to')} {viewMode === "grid" ? "list" : "grid"} view
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
												<span className="sr-only">{t('filterFiles', 'Filter files')}</span>
											</Button>
										</DropdownMenuTrigger>
										<DropdownMenuContent align="end">
											<DropdownMenuLabel>{t('filterByType', 'Filter by Type')}</DropdownMenuLabel>
											<DropdownMenuSeparator />
											{FILTER_OPTIONS.map(({ key, label }) => (
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
											{t('preview', 'Preview -')} {getDisplayFileName(selectedFile.name)}
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
										key={selectedFile.url}
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
												{t('filesReferences', 'Files & References')}
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
												<h3 className="font-medium text-sm">{t('preview2', 'Preview')}</h3>
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
													key={selectedFile.url}
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
										{t('filesReferences', 'Files & References')}
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
	handleFileClick: (file: ProcessedAttachment) => void;
	canPreview: (file: ProcessedAttachment) => boolean;
}

function FileList({
	files,
	viewMode,
	selectedFile,
	onFileSelect,
	handleFileClick,
	canPreview,
}: FileListProps) {
	const { t } = useTranslation("chat");
	if (files.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
				<FileText className="h-6 w-6 text-muted-foreground" />
				<p className="text-sm text-muted-foreground">{t('noMatchingFiles', 'No matching files')}</p>
			</div>
		);
	}

	return (
		<div
			className={`grid gap-2 ${viewMode === "grid" ? "grid-cols-2 md:grid-cols-3 lg:grid-cols-4" : "grid-cols-1"}`}
		>
			{files.map((file, index) => (
				<FileItem
					grid={viewMode === "grid"}
					key={index}
					file={file}
					isSelected={selectedFile?.url === file.url}
					onSelect={onFileSelect}
					handleFileClick={handleFileClick}
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
	handleFileClick: (file: ProcessedAttachment) => void;
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
			<Badge variant="secondary" className="text-xs px-1 py-0 h-4">{i18next.t('pagePagenumber', 'Page {{pageNumber}}', { pageNumber: file.pageNumber })}</Badge>
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
	handleFileClick,
	canPreview,
}: Readonly<FileItemProps>) {
	const previewable = canPreview(file);
	const cardClasses = cn(
		"group relative w-full rounded-lg border border-border/50 bg-linear-to-r from-background to-muted/10 transition-all duration-200",
		isSelected && "border-primary bg-primary/5",
		previewable ? "hover:border-primary/50" : "opacity-75",
	);

	// Touch devices never get :hover, so the action button stays visible there and
	// only fades in on pointer devices.
	const actionVisibility =
		`opacity-100 transition-opacity md:opacity-0 md:group-hover:opacity-100 md:group-focus-within:opacity-100`;

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
					onClick={(e) => {
						e.stopPropagation();
						handleFileClick(file);
					}}
					className={cn(
						"absolute top-1 right-1 size-8 md:top-2 md:right-2 md:size-7",
						actionVisibility,
					)}
				>
					<FileActionIcon file={file} />
					<span className="sr-only">
						{file.isDataUrl ? "Download" : "Open"}
					</span>
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
				onClick={(e) => {
					e.stopPropagation();
					handleFileClick(file);
				}}
				className={cn("h-9 shrink-0 gap-1 px-2.5 md:h-8", actionVisibility)}
			>
				<FileActionIcon file={file} />
				<span className="hidden sm:inline">
					{file.isDataUrl ? "Download" : "Open"}
				</span>
				<span className="sr-only sm:hidden">
					{file.isDataUrl ? "Download" : "Open"}
				</span>
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
	const handleFileClick = (file: ProcessedAttachment) => {
		if (file.isDataUrl) {
			if (file.type === "image") {
				const newWindow = window.open();
				if (newWindow) {
					newWindow.document.write(
						`<img src="${file.url}" style="max-width: 100%; height: auto;" />`,
					);
				}
			} else {
				const link = document.createElement("a");
				link.href = file.url;
				link.download = file.name ?? "file";
				document.body.appendChild(link);
				link.click();
				document.body.removeChild(link);
			}
		} else {
			window.open(file.url, "_blank", "noopener,noreferrer");
		}
	};

	const imageClasses = `max-w-full max-h-full object-contain rounded-md`;
	const videoClasses = `max-w-full max-h-full rounded-md`;
	const showTextPreview = isText(file.url) || isCode(file.url);
	const fallbackClasses =
		"flex flex-col items-center justify-center gap-4 h-full p-4 text-center md:p-8";

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
		case "other":
			if (showTextPreview) {
				return (
					<div className="w-full h-full">
						<FilePreviewer url={file.url} />
					</div>
				);
			}
			return (
				<div className={fallbackClasses}>
					{getFileIcon(file.type)}
					<p className="text-sm text-muted-foreground">
						{t('previewNotAvailableForThisFileType', 'Preview not available for this file type')}
					</p>
					<Button
						variant="outline"
						onClick={() => handleFileClick(file)}
						className="gap-2"
					>
						<FileActionIcon file={file} className="w-4 h-4" />
						{file.isDataUrl ? "Download" : "Open"}
					</Button>
				</div>
			);
		default:
			return (
				<div className={fallbackClasses}>
					{getFileIcon(file.type)}
					<p className="text-sm text-muted-foreground">
						{t('previewNotAvailableForThisFileType', 'Preview not available for this file type')}
					</p>
					<Button
						variant="outline"
						onClick={() => handleFileClick(file)}
						className="gap-2"
					>
						<FileActionIcon file={file} className="w-4 h-4" />
						{file.isDataUrl ? "Download" : "Open"}
					</Button>
				</div>
			);
	}
}
