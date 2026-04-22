import { IconBinary, IconPdf } from "@tabler/icons-react";
import {
	BracesIcon,
	Database,
	EllipsisVerticalIcon,
	ExternalLinkIcon,
	FileArchive,
	FileAudioIcon,
	FileIcon,
	FileImageIcon,
	FileSpreadsheetIcon,
	FileTextIcon,
	FileVideoIcon,
	FolderIcon,
	FolderOpenIcon,
	Music,
	PresentationIcon,
	Settings,
	Zap,
} from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { type INode, type IStorageItem, humanFileSize } from "../../lib";
import { convertJsonToUint8Array } from "../../lib/uint8";
import {
	Badge,
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
	DropdownMenuTrigger,
	canPreview,
	isAudio,
	isCode,
	isImage,
	isText,
	isVideo,
} from "../ui";

export interface AppEntry {
	name: string;
	path: string;
	is_default: boolean;
}

const TEMPLATE = JSON.parse(`{
  "nodes": [
    {
      "id": "e1l3erwbnvnxnnv97524oiuq",
      "name": "child",
      "friendly_name": "Child",
      "comment": "",
      "coordinates": [
        1502.9323,
        -1082.7615,
        0
      ],
      "pins": {
        "mvgidpho004uf2ntonjyt1ji": {
          "id": "mvgidpho004uf2ntonjyt1ji",
          "name": "path",
          "friendly_name": "Path",
          "pin_type": "Output",
          "data_type": "Struct",
          "value_type": "Normal",
          "depends_on": [],
          "connected_to": [],
          "index": 1
        },
        "w8k4qi9sq7265ium4c3l6qg8": {
          "id": "w8k4qi9sq7265ium4c3l6qg8",
          "name": "parent_path",
          "friendly_name": "Path",
          "pin_type": "Input",
          "data_type": "Struct",
          "value_type": "Normal",
          "depends_on": [
            "mgn6k0wfu3owigtms65ngqts"
          ],
          "connected_to": [],
          "index": 1
        },
        "x56ex8kn2uoq37rd8xitawbh": {
          "id": "x56ex8kn2uoq37rd8xitawbh",
          "name": "child_name",
          "friendly_name": "Child",
          "pin_type": "Input",
          "data_type": "String",
          "value_type": "Normal",
          "depends_on": [],
          "connected_to": [],
          "default_value": [
            34,
            116,
            101,
            115,
            116,
            47,
            97,
            98,
            99,
            34
          ],
          "index": 2
        }
      }
    },
    {
      "id": "bt5orj8f06apjvbbz4isbapr",
      "name": "path_from_upload_dir",
      "friendly_name": "Upload Dir",
      "comment": "",
      "coordinates": [
        1315.6329,
        -1082.7726,
        0
      ],
      "pins": {
        "mgn6k0wfu3owigtms65ngqts": {
          "id": "mgn6k0wfu3owigtms65ngqts",
          "name": "path",
          "friendly_name": "Path",
          "pin_type": "Output",
          "data_type": "Struct",
          "value_type": "Normal",
          "depends_on": [],
          "connected_to": [
            "w8k4qi9sq7265ium4c3l6qg8"
          ],
          "index": 1
        }
      }
    }
  ],
  "comments": []
}`);

export function FileOrFolder({
	file,
	highlight,
	changePrefix,
	loadFile,
	shareFile,
	deleteFile,
	downloadFile,
	revealInExplorer,
	openWithApp,
	listAppsForFile,
}: Readonly<{
	file: IStorageItem;
	highlight: boolean;
	changePrefix?: (prefix: string) => void;
	loadFile?: (file: string) => void;
	shareFile?: (file: string, e: any) => void;
	deleteFile?: (file: string) => void;
	downloadFile?: (file: string) => void;
	revealInExplorer?: (location: string) => void;
	openWithApp?: (location: string, appPath?: string) => void;
	listAppsForFile?: (location: string) => Promise<AppEntry[]>;
}>) {
	const relativeSegments = useCallback(() => {
		const segments = file.location.split("/").filter(Boolean);
		if (segments[0] === "apps" && segments.length >= 3) {
			return segments.slice(3);
		}
		if (segments[0] === "users" && segments.length >= 4) {
			return segments.slice(4);
		}
		return segments;
	}, [file.location]);

	const copyPath = useCallback(
		(isFolder: boolean) => {
			const childNode = (TEMPLATE.nodes as INode[]).findIndex(
				(node) => node.name === "child",
			);
			if (childNode === -1) return;
			const segments = relativeSegments();

			if (isFolder) {
				const location = file.location.split("/").pop()?.slice(1, -7) ?? "";
				const parentPath = segments.slice(0, -1).join("/");
				TEMPLATE.nodes[childNode].pins.w8k4qi9sq7265ium4c3l6qg8.default_value =
					convertJsonToUint8Array(`${parentPath}/${location}`);
				navigator.clipboard.writeText(JSON.stringify(TEMPLATE));
				toast.success("Path copied to clipboard");
				return;
			}

			TEMPLATE.nodes[childNode].pins.x56ex8kn2uoq37rd8xitawbh.default_value =
				convertJsonToUint8Array(segments.join("/"));
			navigator.clipboard.writeText(JSON.stringify(TEMPLATE));
			toast.success("Path copied to clipboard");
		},
		[file.location, relativeSegments],
	);

	if (file.is_dir) {
		return (
			<div
				className={`group relative rounded-lg border border-border/50 p-3 w-full transition-all duration-200 hover:border-primary/50 hover:shadow-md bg-linear-to-r from-background to-muted/20 ${
					highlight ? "border-primary bg-primary/5 shadow-sm" : ""
				}`}
			>
				<button
					className="w-full flex flex-row justify-between items-center"
					onClick={() => {
						changePrefix?.(file.location.split("/").pop() ?? file.location);
					}}
				>
					<div className="flex flex-row items-center gap-3">
						<div className="p-2 rounded-md bg-primary/10 group-hover:bg-primary/20 transition-colors">
							<FolderIcon className="w-5 h-5 text-primary" />
						</div>
						<div className="flex flex-col items-start">
							<p className="line-clamp-1 text-start font-medium text-foreground text-sm sm:text-base">
								{file.location.split("/").pop()}
							</p>
							<Badge
								variant="secondary"
								className="text-xs mt-1 px-1 py-0 h-4 sm:h-5 sm:px-2 sm:py-1"
							>
								Folder
							</Badge>
						</div>
					</div>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								className="opacity-0 group-hover:opacity-100 transition-opacity"
								variant="ghost"
								size="sm"
								onClick={(e) => {
									e.stopPropagation();
									e.preventDefault();
								}}
							>
								<EllipsisVerticalIcon className="h-4 w-4" />
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end">
							<DropdownMenuLabel>Folder Actions</DropdownMenuLabel>
							<DropdownMenuSeparator />
							{typeof revealInExplorer !== "undefined" && (
								<DropdownMenuItem
									onClick={(e) => {
										e.preventDefault();
										e.stopPropagation();
										revealInExplorer(file.location);
									}}
								>
									<FolderOpenIcon className="h-4 w-4 mr-2" />
									Show in File Manager
								</DropdownMenuItem>
							)}
							<DropdownMenuItem
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									copyPath(true);
								}}
							>
								Copy Path
							</DropdownMenuItem>
							{typeof deleteFile !== "undefined" && (
								<>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										className="bg-destructive text-destructive-foreground focus:text-destructive-foreground"
										onClick={(e) => {
											e.preventDefault();
											e.stopPropagation();
											deleteFile?.(file.location.split("/").pop() ?? "");
										}}
									>
										Delete
									</DropdownMenuItem>
								</>
							)}
						</DropdownMenuContent>
					</DropdownMenu>
				</button>
			</div>
		);
	}

	return (
		<div
			className={`group relative rounded-lg border border-border/50 p-3 w-full transition-all duration-200 bg-linear-to-r from-background to-muted/10 ${
				highlight ? "border-primary bg-primary/5 shadow-sm" : ""
			} ${
				canPreview(file.location)
					? "hover:border-primary/50 hover:shadow-md cursor-pointer"
					: "cursor-not-allowed opacity-75"
			}`}
		>
			<button
				className="w-full flex flex-row justify-between items-center"
				onClick={() => {
					if (canPreview(file.location)) loadFile?.(file.location);
				}}
			>
				<div className="flex flex-row items-center gap-3 flex-1 min-w-0">
					<div
						className={`p-2 rounded-md transition-colors ${
							canPreview(file.location)
								? "bg-primary/10 group-hover:bg-primary/20"
								: "bg-muted/50"
						}`}
					>
						<IconForFile file={file} />
					</div>
					<div className="flex flex-col items-start flex-1 min-w-0">
						<p className="line-clamp-1 text-start font-medium text-foreground truncate w-full text-sm sm:text-base">
							{file.location.split("/").pop()}
						</p>
						<div className="flex items-center gap-1 sm:gap-2 mt-1">
							<Badge
								variant="outline"
								className="text-xs px-1 py-0 h-4 sm:h-5 sm:px-2 sm:py-1"
							>
								{humanFileSize(file.size, true)}
							</Badge>
							<FileTypeBadge filename={file.location} />
						</div>
					</div>
				</div>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button
							className="opacity-0 group-hover:opacity-100 transition-opacity"
							variant="ghost"
							size="sm"
							onClick={(e) => {
								e.stopPropagation();
								e.preventDefault();
							}}
						>
							<EllipsisVerticalIcon className="h-4 w-4" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end">
						<DropdownMenuLabel>File Actions</DropdownMenuLabel>
						<DropdownMenuSeparator />
						{typeof openWithApp !== "undefined" && (
							<OpenWithMenu
								location={file.location}
								openWithApp={openWithApp}
								listAppsForFile={listAppsForFile}
							/>
						)}
						{typeof revealInExplorer !== "undefined" && (
							<DropdownMenuItem
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									revealInExplorer(file.location);
								}}
							>
								<FolderOpenIcon className="h-4 w-4 mr-2" />
								Show in File Manager
							</DropdownMenuItem>
						)}
						<DropdownMenuItem
							onClick={(e) => {
								e.preventDefault();
								e.stopPropagation();
								copyPath(false);
							}}
						>
							Copy Path
						</DropdownMenuItem>
						{typeof downloadFile !== "undefined" && (
							<DropdownMenuItem
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									downloadFile?.(file.location);
								}}
							>
								Download
							</DropdownMenuItem>
						)}
						{typeof shareFile !== "undefined" && (
							<DropdownMenuItem
								disabled
								onClick={(e) => {
									e.preventDefault();
									e.stopPropagation();
									shareFile?.(file.location.split("/").pop() ?? "", e);
								}}
							>
								Share
							</DropdownMenuItem>
						)}
						{typeof deleteFile !== "undefined" && (
							<>
								<DropdownMenuSeparator />
								<DropdownMenuItem
									className="bg-destructive text-destructive-foreground focus:text-destructive-foreground"
									onClick={(e) => {
										e.preventDefault();
										e.stopPropagation();
										deleteFile?.(file.location.split("/").pop() ?? "");
									}}
								>
									Delete
								</DropdownMenuItem>
							</>
						)}
					</DropdownMenuContent>
				</DropdownMenu>
			</button>
		</div>
	);
}

function OpenWithMenu({
	location,
	openWithApp,
	listAppsForFile,
}: {
	location: string;
	openWithApp: (location: string, appPath?: string) => void;
	listAppsForFile?: (location: string) => Promise<AppEntry[]>;
}) {
	const [apps, setApps] = useState<AppEntry[]>([]);
	const [loaded, setLoaded] = useState(false);

	const loadApps = useCallback(async () => {
		if (loaded || !listAppsForFile) return;
		try {
			const result = await listAppsForFile(location);
			setApps(result);
		} catch {
			// ignore — fallback to simple open
		}
		setLoaded(true);
	}, [loaded, listAppsForFile, location]);

	const defaultApp = apps.find((a) => a.is_default);

	if (!listAppsForFile) {
		return (
			<DropdownMenuItem
				onClick={(e) => {
					e.preventDefault();
					e.stopPropagation();
					openWithApp(location);
				}}
			>
				<ExternalLinkIcon className="h-4 w-4 mr-2" />
				Open with Default App
			</DropdownMenuItem>
		);
	}

	return (
		<DropdownMenuSub onOpenChange={(open) => open && loadApps()}>
			<DropdownMenuSubTrigger
				onClick={(e) => {
					e.preventDefault();
					e.stopPropagation();
					if (defaultApp) {
						openWithApp(location, defaultApp.path);
					} else {
						openWithApp(location);
					}
				}}
			>
				<ExternalLinkIcon className="h-4 w-4 mr-2" />
				{defaultApp ? `Open with ${defaultApp.name}` : "Open File"}
			</DropdownMenuSubTrigger>
			<DropdownMenuSubContent>
				{!loaded && (
					<DropdownMenuItem disabled>Loading apps...</DropdownMenuItem>
				)}
				{loaded && apps.length === 0 && (
					<DropdownMenuItem disabled>No apps found</DropdownMenuItem>
				)}
				{apps.map((app) => (
					<DropdownMenuItem
						key={app.path}
						onClick={(e) => {
							e.preventDefault();
							e.stopPropagation();
							openWithApp(location, app.path);
						}}
					>
						{app.name}
						{app.is_default && (
							<Badge
								variant="secondary"
								className="ml-2 text-[10px] px-1 py-0 h-4"
							>
								Default
							</Badge>
						)}
					</DropdownMenuItem>
				))}
			</DropdownMenuSubContent>
		</DropdownMenuSub>
	);
}

function FileTypeBadge({ filename }: { filename: string }) {
	const extension = filename.split(".").pop()?.toLowerCase();
	const badgeClassName = "text-xs px-1 py-0 h-4 sm:h-5 sm:px-2 sm:py-1";

	if (isImage(filename))
		return (
			<Badge variant="secondary" className={badgeClassName}>
				Image
			</Badge>
		);
	if (isVideo(filename))
		return (
			<Badge variant="secondary" className={badgeClassName}>
				Video
			</Badge>
		);
	if (isAudio(filename))
		return (
			<Badge variant="secondary" className={badgeClassName}>
				Audio
			</Badge>
		);
	if (isCode(filename))
		return (
			<Badge variant="secondary" className={badgeClassName}>
				Code
			</Badge>
		);
	if (isText(filename))
		return (
			<Badge variant="secondary" className={badgeClassName}>
				Text
			</Badge>
		);

	switch (extension) {
		case "pdf":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					PDF
				</Badge>
			);
		case "zip":
		case "rar":
		case "7z":
		case "tar":
		case "gz":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					Archive
				</Badge>
			);
		case "xlsx":
		case "xls":
		case "csv":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					Sheet
				</Badge>
			);
		case "pptx":
		case "ppt":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					Slides
				</Badge>
			);
		case "sql":
		case "db":
		case "sqlite":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					DB
				</Badge>
			);
		case "json":
		case "xml":
		case "yaml":
		case "yml":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					Data
				</Badge>
			);
		case "exe":
		case "msi":
		case "app":
		case "deb":
		case "rpm":
			return (
				<Badge variant="secondary" className={badgeClassName}>
					Exec
				</Badge>
			);
		default:
			return (
				<Badge variant="outline" className={badgeClassName}>
					{extension?.toUpperCase() || "File"}
				</Badge>
			);
	}
}

function IconForFile({ file }: Readonly<{ file: IStorageItem }>) {
	const className = `w-5 h-5 ${canPreview(file.location) ? "text-primary" : "text-muted-foreground"}`;
	const extension = file.location.split(".").pop()?.toLowerCase();

	// PDF files
	if (file.location.endsWith(".pdf")) return <IconPdf className={className} />;

	// Images
	if (isImage(file.location)) return <FileImageIcon className={className} />;

	// Videos
	if (isVideo(file.location)) return <FileVideoIcon className={className} />;

	// Audio
	if (isAudio(file.location)) return <FileAudioIcon className={className} />;

	// Code files
	if (isCode(file.location)) return <BracesIcon className={className} />;

	// Text files
	if (isText(file.location)) return <FileTextIcon className={className} />;

	// Specific file types
	switch (extension) {
		case "zip":
		case "rar":
		case "7z":
		case "tar":
		case "gz":
			return <FileArchive className={className} />;
		case "xlsx":
		case "xls":
		case "csv":
			return <FileSpreadsheetIcon className={className} />;
		case "pptx":
		case "ppt":
			return <PresentationIcon className={className} />;
		case "sql":
		case "db":
		case "sqlite":
			return <Database className={className} />;
		case "json":
		case "xml":
		case "yaml":
		case "yml":
			return <BracesIcon className={className} />;
		case "exe":
		case "msi":
		case "app":
		case "deb":
		case "rpm":
			return <Zap className={className} />;
		case "conf":
		case "config":
		case "ini":
		case "env":
			return <Settings className={className} />;
		case "mp3":
		case "wav":
		case "flac":
		case "aac":
			return <Music className={className} />;
		default:
			// Files without extension (binary)
			if (!file.location.split("/").pop()?.includes("."))
				return <IconBinary className={className} />;
			return <FileIcon className={className} />;
	}
}
