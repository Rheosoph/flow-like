"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Archive,
	ChevronRight,
	FileText,
	Files,
	Folder,
	FolderMinus,
	FolderOpen,
	FolderPlus,
	Image,
	Music,
	Search,
	Trash2,
	Video,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { humanFileSize } from "../../../../lib";
import {
	Badge,
	Button,
	Card,
	Checkbox,
	Dialog,
	DialogContent,
	DialogHeader,
	DialogTitle,
	Input,
	ScrollArea,
	Separator,
} from "../../../ui";

interface FileManagerDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	files: File[];
	onRemoveFile: (index: number) => void;
	onRemoveFiles: (indices: number[]) => void;
	onClearAll: () => void;
}

interface FileNode {
	name: string;
	type: "file" | "folder";
	path: string;
	file?: File;
	originalIndex?: number;
	children?: FileNode[];
	isExpanded?: boolean;
}

export function FileManagerDialog({
	open,
	onOpenChange,
	files,
	onRemoveFile,
	onRemoveFiles,
	onClearAll,
}: Readonly<FileManagerDialogProps>) {
	const { t } = useTranslation("chat");
	const [searchTerm, setSearchTerm] = useState("");
	const [selectedFiles, setSelectedFiles] = useState<Set<number>>(new Set());
	const [expandedFolders, setExpandedFolders] = useState<Set<string>>(
		new Set(["root"]),
	);

	const getFileIcon = (file: File) => {
		if (file.type.startsWith("image/")) return Image;
		if (file.type.startsWith("video/")) return Video;
		if (file.type.startsWith("audio/")) return Music;
		if (
			file.type.includes("zip") ||
			file.type.includes("rar") ||
			file.type.includes("tar")
		)
			return Archive;
		return FileText;
	};

	const getFileTypeColor = (file: File) => {
		if (file.type.startsWith("image/"))
			return `bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-950 dark:text-emerald-300 dark:border-emerald-800`;
		if (file.type.startsWith("video/"))
			return `bg-violet-50 text-violet-700 border-violet-200 dark:bg-violet-950 dark:text-violet-300 dark:border-violet-800`;
		if (file.type.startsWith("audio/"))
			return `bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-950 dark:text-amber-300 dark:border-amber-800`;
		if (
			file.type.includes("zip") ||
			file.type.includes("rar") ||
			file.type.includes("tar")
		)
			return `bg-orange-50 text-orange-700 border-orange-200 dark:bg-orange-950 dark:text-orange-300 dark:border-orange-800`;
		return `bg-slate-50 text-slate-700 border-slate-200 dark:bg-slate-950 dark:text-slate-300 dark:border-slate-800`;
	};

	// Build hierarchical file tree based on path prefixes
	const fileTree = useMemo(() => {
		const root: FileNode = {
			name: "root",
			type: "folder",
			path: "",
			children: [],
		};

		files.forEach((file, index) => {
			if (
				searchTerm &&
				!file.name.toLowerCase().includes(searchTerm.toLowerCase())
			) {
				return;
			}

			const pathParts = file.webkitRelativePath
				? file.webkitRelativePath.split("/").filter(Boolean)
				: [file.name];

			let currentNode = root;
			let currentPath = "";

			// Navigate/create folder structure
			for (let i = 0; i < pathParts.length - 1; i++) {
				const folderName = pathParts[i];
				currentPath = currentPath ? `${currentPath}/${folderName}` : folderName;

				let folderNode = currentNode.children?.find(
					(child) => child.name === folderName && child.type === "folder",
				);

				if (!folderNode) {
					folderNode = {
						name: folderName,
						type: "folder",
						path: currentPath,
						children: [],
						isExpanded: expandedFolders.has(currentPath),
					};
					currentNode.children?.push(folderNode);
				}

				currentNode = folderNode;
			}

			// Add file node
			const fileName = pathParts[pathParts.length - 1];
			const filePath = currentPath ? `${currentPath}/${fileName}` : fileName;

			currentNode.children?.push({
				name: fileName,
				type: "file",
				path: filePath,
				file,
				originalIndex: index,
			});
		});

		// Sort: folders first, then files, both alphabetically
		const sortNodes = (nodes: FileNode[]) => {
			nodes.sort((a, b) => {
				if (a.type !== b.type) {
					return a.type === "folder" ? -1 : 1;
				}
				return a.name.localeCompare(b.name);
			});

			nodes.forEach((node) => {
				if (node.children) {
					sortNodes(node.children);
				}
			});
		};

		if (root.children) {
			sortNodes(root.children);
		}

		return root;
	}, [files, searchTerm, expandedFolders]);

	const toggleFolder = (path: string) => {
		setExpandedFolders((prev) => {
			const next = new Set(prev);
			if (next.has(path)) {
				next.delete(path);
			} else {
				next.add(path);
			}
			return next;
		});
	};

	const toggleFileSelection = (index: number) => {
		setSelectedFiles((prev) => {
			const next = new Set(prev);
			if (next.has(index)) {
				next.delete(index);
			} else {
				next.add(index);
			}
			return next;
		});
	};

	const selectAllInFolder = (node: FileNode) => {
		const indices: number[] = [];

		const collectIndices = (n: FileNode) => {
			if (n.type === "file" && n.originalIndex !== undefined) {
				indices.push(n.originalIndex);
			}
			n.children?.forEach(collectIndices);
		};

		collectIndices(node);

		setSelectedFiles((prev) => {
			const next = new Set(prev);
			indices.forEach((index) => next.add(index));
			return next;
		});
	};

	const deselectAllInFolder = (node: FileNode) => {
		const indices: number[] = [];

		const collectIndices = (n: FileNode) => {
			if (n.type === "file" && n.originalIndex !== undefined) {
				indices.push(n.originalIndex);
			}
			n.children?.forEach(collectIndices);
		};

		collectIndices(node);

		setSelectedFiles((prev) => {
			const next = new Set(prev);
			indices.forEach((index) => next.delete(index));
			return next;
		});
	};

	const hasSelectedFilesInFolder = (node: FileNode): boolean => {
		if (node.type === "file") {
			return selectedFiles.has(node.originalIndex!);
		}
		return (
			node.children?.some((child) => hasSelectedFilesInFolder(child)) || false
		);
	};

	const getFolderSelectionState = (
		node: FileNode,
	): "none" | "partial" | "all" => {
		const indices: number[] = [];

		const collectIndices = (n: FileNode) => {
			if (n.type === "file" && n.originalIndex !== undefined) {
				indices.push(n.originalIndex);
			}
			n.children?.forEach(collectIndices);
		};

		collectIndices(node);

		if (indices.length === 0) return "none";

		const selectedCount = indices.filter((index) =>
			selectedFiles.has(index),
		).length;
		if (selectedCount === 0) return "none";
		if (selectedCount === indices.length) return "all";
		return "partial";
	};

	const deselectAll = () => {
		setSelectedFiles(new Set());
	};

	const removeSelectedFiles = () => {
		onRemoveFiles(Array.from(selectedFiles));
		setSelectedFiles(new Set());
	};

	const renderFileNode = (node: FileNode, depth = 0): React.ReactNode => {
		if (node.type === "folder") {
			const isExpanded = expandedFolders.has(node.path);
			const fileCount = countFiles(node);
			const selectionState = getFolderSelectionState(node);
			const hasSelected = hasSelectedFilesInFolder(node);

			const getIndicesInFolder = (folderNode: FileNode): number[] => {
				const indices: number[] = [];
				const collectIndices = (n: FileNode) => {
					if (n.type === "file" && n.originalIndex !== undefined) {
						indices.push(n.originalIndex);
					}
					n.children?.forEach(collectIndices);
				};
				collectIndices(folderNode);
				return indices;
			};

			const folderIndices = getIndicesInFolder(node);
			const selectedCount = folderIndices.filter((index) =>
				selectedFiles.has(index),
			).length;

			return (
				<div key={node.path} className="space-y-1">
					<div
						className={`group transition-all duration-200 hover:shadow-sm ${hasSelected ? "border-primary shadow-sm bg-primary/5" : "border-transparent hover:border-border"}`}
					>
						<div
							className="flex items-center justify-between p-3 cursor-pointer"
							style={{ paddingLeft: `${depth * 20 + 12}px` }}
						>
							<button
								className="flex items-center gap-3 flex-1 min-w-0"
								onClick={() => toggleFolder(node.path)}
							>
								<ChevronRight
									className={`w-4 h-4 text-muted-foreground transition-transform duration-200 ${isExpanded ? "rotate-90" : ""}`}
								/>
								{isExpanded ? (
									<FolderOpen
										className={`w-5 h-5 ${hasSelected ? "text-primary" : "text-foreground"}`}
									/>
								) : (
									<Folder
										className={`w-5 h-5 ${hasSelected ? "text-primary" : "text-foreground"}`}
									/>
								)}
								<span
									className={`font-medium truncate ${hasSelected ? "text-primary" : ""}`}
								>
									{node.name || "Root"}
								</span>
								<Badge
									variant={hasSelected ? "default" : "secondary"}
									className="text-xs shrink-0"
								>
									{fileCount}
								</Badge>
								{selectionState === "partial" && (
									<Badge className="text-xs bg-primary/10 text-primary border-primary/30 shrink-0">{t('selectedcountSelected', '{{selectedCount}} selected', { selectedCount })}</Badge>
								)}
								{selectionState === "all" && (
									<Badge className="text-xs shrink-0">{t('allSelected', 'All selected')}</Badge>
								)}
							</button>
							<div className="flex items-center gap-2 opacity-0 group-hover:opacity-100 transition-opacity duration-200">
								{selectionState !== "none" && (
									<Button
										variant="ghost"
										size="sm"
										onClick={(e) => {
											e.stopPropagation();
											deselectAllInFolder(node);
										}}
										className="h-8 px-3 text-xs hover:bg-destructive/10 hover:text-destructive"
									>
										<FolderMinus className="w-3 h-3 mr-1" />
										{t('deselect', 'Deselect')}
									</Button>
								)}
								{selectionState !== "all" && (
									<Button
										variant="ghost"
										size="sm"
										onClick={(e) => {
											e.stopPropagation();
											selectAllInFolder(node);
										}}
										className="h-8 px-3 text-xs hover:bg-primary/10 hover:text-primary"
									>
										<FolderPlus className="w-3 h-3 mr-1" />
										{t('select', 'Select')}
									</Button>
								)}
							</div>
						</div>
					</div>

					{isExpanded && node.children && (
						<div className="space-y-1 ml-2 border-l border-border/40 pl-2">
							{node.children.map((child) => renderFileNode(child, depth + 1))}
						</div>
					)}
				</div>
			);
		}

		// File node
		const isSelected = selectedFiles.has(node.originalIndex!);
		const FileIcon = getFileIcon(node.file!);

		return (
			<Card
				key={node.originalIndex}
				className={`group transition-all duration-200 hover:shadow-sm ${isSelected ? "border-primary shadow-sm bg-primary/5" : "border-transparent hover:border-border"}`}
				style={{ marginLeft: `${depth * 20 + 12}px` }}
			>
				<div className="flex items-center gap-4 p-3">
					<Checkbox
						checked={isSelected}
						onCheckedChange={() => toggleFileSelection(node.originalIndex!)}
						className="shrink-0"
					/>

					{/* File Preview/Icon */}
					{node.file!.type.startsWith("image/") ? (
						<div className="relative shrink-0">
							<img
								src={URL.createObjectURL(node.file!)}
								alt={node.name}
								className="w-12 h-12 object-cover rounded-lg border border-border shadow-sm"
								onLoad={(e) =>
									URL.revokeObjectURL((e.target as HTMLImageElement).src)
								}
							/>
							<div className="absolute -top-1 -right-1 w-5 h-5 bg-background rounded-full flex items-center justify-center border border-border shadow-sm">
								<Image className="w-2.5 h-2.5 text-emerald-600" />
							</div>
						</div>
					) : (
						<div className="w-12 h-12 bg-muted/50 rounded-lg border border-border flex items-center justify-center shrink-0">
							<FileIcon className="w-6 h-6 text-muted-foreground" />
						</div>
					)}

					{/* File Info */}
					<div className="flex-1 min-w-0 space-y-1">
						<div className="font-medium text-sm truncate">{node.name}</div>
						<div className="flex items-center gap-2">
							<span className="text-xs text-muted-foreground">
								{humanFileSize(node.file!.size, true)}
							</span>
							<Badge
								variant="outline"
								className={`text-xs ${getFileTypeColor(node.file!)}`}
							>
								{node.file!.type.split("/")[0] || "file"}
							</Badge>
						</div>
					</div>

					{/* Remove Button */}
					<Button
						variant="ghost"
						size="sm"
						onClick={() => onRemoveFile(node.originalIndex!)}
						className="h-8 w-8 p-0 opacity-0 group-hover:opacity-100 hover:bg-destructive/10 hover:text-destructive transition-all duration-200 shrink-0"
					>
						<Trash2 className="w-4 h-4" />
					</Button>
				</div>
			</Card>
		);
	};

	const countFiles = (node: FileNode): number => {
		if (node.type === "file") return 1;
		return (
			node.children?.reduce((count, child) => count + countFiles(child), 0) || 0
		);
	};

	const totalSize = files.reduce((acc, file) => acc + file.size, 0);
	const filteredCount = countFiles(fileTree);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-6xl h-[90vh] flex flex-col gap-0">
				<DialogHeader className="space-y-6 pb-4">
					<DialogTitle className="flex items-center justify-between">
						<div className="flex items-center gap-3">
							<Files className="w-5 h-5" />
							<span>{t('manageAttachedFiles', 'Manage Attached Files')}</span>
						</div>
						<div className="flex items-center gap-3">
							<Badge variant="outline" className="text-sm font-normal">{t('lengthFiles', '{{length}} files', { length: files.length })}</Badge>
							<Badge variant="outline" className="text-sm font-normal">
								{humanFileSize(totalSize, true)}
							</Badge>
						</div>
					</DialogTitle>

					{/* Search and Actions */}
					<div className="flex gap-3 items-center">
						<div className="relative flex-1">
							<Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 text-muted-foreground" />
							<Input
								placeholder={t('searchFiles', 'Search files...')}
								value={searchTerm}
								onChange={(e) => setSearchTerm(e.target.value)}
								className="pl-10 h-10"
							/>
						</div>
						{selectedFiles.size > 0 && (
							<>
								<Button
									variant="outline"
									onClick={deselectAll}
									className="h-10"
								>
									<X className="w-4 h-4 mr-2" />
									{t('deselectAll', 'Deselect All')}
								</Button>
								<Button
									variant="destructive"
									onClick={removeSelectedFiles}
									className="h-10"
								>
									<Trash2 className="w-4 h-4 mr-2" />{t('removeSize', 'Remove {{size}}', { size: selectedFiles.size })}</Button>
							</>
						)}
						<Button
							variant="outline"
							onClick={onClearAll}
							className="h-10 hover:bg-destructive/10 hover:text-destructive hover:border-destructive/20"
						>
							<Trash2 className="w-4 h-4 mr-2" />
							{t('clearAll2', 'Clear All')}
						</Button>
					</div>
				</DialogHeader>

				<Separator />

				{/* File Tree */}
				<ScrollArea className="flex-1 -mx-6 px-6">
					<div className="space-y-2 py-4">
						{fileTree.children && fileTree.children.length > 0 ? (
							fileTree.children.map((node) => renderFileNode(node))
						) : (
							<div className="text-center py-16">
								<div className="w-20 h-20 mx-auto mb-6 rounded-full bg-muted/30 flex items-center justify-center">
									<Files className="w-10 h-10 text-muted-foreground/50" />
								</div>
								<h3 className="text-lg font-semibold mb-2">{t('noFilesFound', 'No files found')}</h3>
								<p className="text-muted-foreground max-w-sm mx-auto">
									{searchTerm
										? t('tryAdjustingYourSearchTermsToFindFiles', 'Try adjusting your search terms to find files')
										: t('noFilesHaveBeenAttachedToThisConversationYet', 'No files have been attached to this conversation yet')}
								</p>
							</div>
						)}
					</div>
				</ScrollArea>

				<Separator />

				{/* Footer Stats */}
				<div className="flex items-center justify-between pt-4 text-sm text-muted-foreground">
					<span className="flex items-center gap-2">
						<Badge variant="secondary" className="text-xs">{t('filteredcountOfLength', '{{filteredCount}} of {{length}}', { filteredCount, length: files.length })}</Badge>
						{t('filesShown', 'files shown')}
					</span>
					<span className="font-medium">
						{t('total', 'Total:')} {humanFileSize(totalSize, true)}
					</span>
				</div>
			</DialogContent>
		</Dialog>
	);
}
