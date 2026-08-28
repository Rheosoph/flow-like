import {
	EllipsisVerticalIcon,
	ExternalLinkIcon,
	FolderIcon,
	FolderOpenIcon,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks";
import { type IStorageItem, humanFileSize } from "../../lib";
import { buildStoragePathNodes } from "../../lib/storage-path-nodes";
import { useBackend } from "../../state/backend-state";
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
	FileTypeBadge,
	FileTypeIcon,
	canPreview,
} from "../ui";

export interface AppEntry {
	name: string;
	path: string;
	is_default: boolean;
}

export function FileOrFolder({
	appId,
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
	appId: string;
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
	// Both storage scopes report `location` differently — cloud sends an absolute object
	// key, desktop a relative path — so the root-relative path is recovered by stripping
	// whichever layout prefix is present, and that also tells us which scope this is.
	const relative = useMemo(() => {
		const segments = file.location.split("/").filter(Boolean);
		if (segments[0] === "apps" && segments.length >= 3) {
			return { scope: "app" as const, path: segments.slice(3).join("/") };
		}
		if (segments[0] === "users" && segments.length >= 4) {
			return { scope: "user" as const, path: segments.slice(4).join("/") };
		}
		return { scope: "app" as const, path: segments.join("/") };
	}, [file.location]);

	const backend = useBackend();
	const catalog = useInvoke(backend.boardState.getCatalog, backend.boardState, [
		appId,
	]);

	const copyPath = useCallback(() => {
		const fragment = buildStoragePathNodes({
			catalog: catalog.data,
			scope: relative.scope,
			path: relative.path,
		});
		if (!fragment) {
			toast.error("Path nodes are not available");
			return;
		}
		navigator.clipboard.writeText(
			JSON.stringify({ nodes: fragment.nodes, comments: [] }),
		);
		toast.success("Path copied to clipboard");
	}, [catalog.data, relative]);

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
									copyPath();
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
						<FileTypeIcon
							name={file.location}
							className={`w-5 h-5 ${canPreview(file.location) ? "text-primary" : "text-muted-foreground"}`}
						/>
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
							<FileTypeBadge
								filename={file.location}
								className="text-xs px-1 py-0 h-4 sm:h-5 sm:px-2 sm:py-1"
							/>
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
								copyPath();
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
