"use client";
import {
	DndContext,
	type DragEndEvent,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { createId } from "@paralleldrive/cuid2";
import type Dexie from "dexie";
import type { EntityTable } from "dexie";
import {
	Bookmark,
	Cable,
	Database,
	ExternalLink,
	FolderClosed,
	Globe,
	GripVertical,
	Loader2,
	type LucideIcon,
	Sparkles,
	Trash2,
	WifiOff,
	Workflow,
} from "lucide-react";
import { useCallback, useState, type ComponentType } from "react";
import { motion } from "framer-motion";
import { isTauri } from "../lib/platform";
import { Avatar, AvatarFallback, AvatarImage } from "./ui/avatar";
import { Button } from "./ui/button";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "./ui/dialog";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { RadioGroup, RadioGroupItem } from "./ui/radio-group";
import {
	SidebarGroup,
	SidebarGroupLabel,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	useSidebar,
} from "./ui/sidebar";
import { AnimatedNewProjectIcon } from "./animated-icons/animated-plus";
import { AutoPlayNewProjectIcon } from "./animated-icons/animated-plus-autoplay";
import { AnimatedPinIcon } from "./animated-icons";

const MotionSidebarMenuButton = motion.create(SidebarMenuButton);

const iconVariants = {
	initial: { scale: 1, rotate: 0 },
	hover: {
		scale: 1.1,
		rotate: 5,
		transition: { type: "spring", stiffness: 400, damping: 10 },
	},
};

export interface IShortcut {
	id: string;
	profileId: string;
	label: string;
	path: string;
	appId?: string;
	icon?: string;
	order: number;
	createdAt: string;
}

interface PredefinedShortcut {
	id: string;
	label: string;
	icon: LucideIcon | ComponentType<{ className?: string }>;
	action: () => void;
}

interface ShortcutsProps<TBackend, TAppMetadata> {
	// Database
	db: Dexie & { shortcuts: EntityTable<IShortcut, "id"> };
	shortcuts: IShortcut[] | undefined;
	currentProfileId: string | undefined;

	// Navigation
	pathname: string;
	onNavigate: (path: string) => void;

	// Backend integration
	backend: TBackend;
	appMetadata: TAppMetadata[] | undefined;
	getAppMetadataById: (
		appId: string,
		metadata: TAppMetadata[],
	) => { name?: string; icon?: string } | null;
	getBoardsByAppId: (
		backend: TBackend,
		appId: string,
	) => Promise<Array<{ id: string }>>;

	// Toast notifications
	toast: {
		success: (message: string) => void;
		error: (message: string) => void;
	};

	// Start Coding feature
	auth?: {
		isAuthenticated?: boolean;
	};
	onCreateProject?: (projectName: string, isOnline: boolean) => Promise<void>;
	bits?: Array<{ id: string }>;
}

export function Shortcuts<TBackend, TAppMetadata>({
	db,
	shortcuts,
	currentProfileId,
	pathname,
	onNavigate,
	backend,
	appMetadata,
	getAppMetadataById,
	getBoardsByAppId,
	toast,
	auth,
	onCreateProject,
	bits,
}: ShortcutsProps<TBackend, TAppMetadata>) {
	const { state: sidebarState } = useSidebar();
	const [startCodingOpen, setStartCodingOpen] = useState(false);
	const [projectName, setProjectName] = useState("");
	const [isOnline, setIsOnline] = useState(true);
	const [isCreating, setIsCreating] = useState(false);

	// Helper to get page type from path
	const getPageType = (
		path: string,
	): { type: string; icon: LucideIcon } | null => {
		if (path.includes("/flow") || path.includes("/library/config/flows")) {
			return { type: "workflow", icon: Workflow };
		}
		if (path.includes("/library/config/events")) {
			return { type: "event", icon: Cable };
		}
		if (path.includes("/library/config/explore")) {
			return { type: "data", icon: Database };
		}
		if (path.includes("/library/config/storage")) {
			return { type: "storage", icon: FolderClosed };
		}
		return null;
	};

	// Helper to get app metadata by ID
	const getAppMetadata = (appId: string) => {
		if (!appMetadata) return null;
		return getAppMetadataById(appId, appMetadata);
	};

	const handleStartCoding = useCallback(async () => {
		if (!projectName.trim()) {
			toast.error("Please enter a project name");
			return;
		}

		if (isOnline && !auth?.isAuthenticated) {
			toast.error("You must be logged in to create an online project");
			return;
		}

		if (!onCreateProject) {
			toast.error("Project creation is not configured");
			return;
		}

		setIsCreating(true);
		try {
			await onCreateProject(projectName.trim(), isOnline);
			toast.success("Project created! 🎉");
			setStartCodingOpen(false);
			setProjectName("");
			setIsOnline(true);
		} catch (error) {
			console.error("Failed to create project:", error);
			toast.error("Failed to create project");
		} finally {
			setIsCreating(false);
		}
	}, [projectName, isOnline, auth?.isAuthenticated, onCreateProject, toast]);

	const handleAddCurrentLocation = useCallback(async () => {
		if (!currentProfileId) {
			toast.error("No profile selected");
			return;
		}

		const fullPath =
			typeof window !== "undefined"
				? window.location.pathname + window.location.search
				: pathname;

		// Extract appId from various URL patterns
		let appId: string | null = null;
		const appMatch = fullPath.match(/[?&]app=([^&]+)/);
		const idMatch = fullPath.match(/[?&]id=([^&]+)/);
		appId = appMatch?.[1] || idMatch?.[1] || null;

		// If no app ID found, check if this is a flow page and find the app by board ID
		if (!appId && pathname === "/flow" && appMetadata) {
			const boardIdMatch = fullPath.match(/[?&]id=([^&]+)/);
			const boardId = boardIdMatch?.[1];

			if (boardId) {
				// Search through all apps to find which one contains this board
				for (const appData of appMetadata) {
					try {
						const boards = await getBoardsByAppId(
							backend,
							(appData as any).id || (appData as any)[0]?.id,
						);
						if (boards?.some((board) => board.id === boardId)) {
							appId = (appData as any).id || (appData as any)[0]?.id;
							break;
						}
					} catch (error) {
						console.error(`Failed to fetch boards for app:`, error);
					}
				}
			}
		}

		// Try to get app metadata for better label
		const metadata = appId ? getAppMetadata(appId) : null;

		const pathParts = pathname.split("/").filter(Boolean);
		const fallbackLabel = pathParts[pathParts.length - 1] || "Home";
		const label =
			metadata?.name ||
			fallbackLabel.charAt(0).toUpperCase() + fallbackLabel.slice(1);

		const shortcut: IShortcut = {
			id: createId(),
			profileId: currentProfileId,
			label,
			path: fullPath,
			appId: appId || undefined,
			order: (shortcuts?.length || 0) + 1,
			createdAt: new Date().toISOString(),
		};

		try {
			await db.shortcuts.add(shortcut);
			toast.success("Shortcut added");
		} catch (error) {
			console.error("Failed to add shortcut:", error);
			toast.error("Failed to add shortcut");
		}
	}, [
		currentProfileId,
		pathname,
		shortcuts,
		getAppMetadata,
		appMetadata,
		backend,
		getBoardsByAppId,
		db,
		toast,
	]);

	const predefinedShortcuts: PredefinedShortcut[] = onCreateProject
		? [
				{
					id: "start-coding",
					label: "Create Flow",
					icon: AnimatedNewProjectIcon,
					action: () => {
						if (!auth?.isAuthenticated) {
							setIsOnline(false);
						}
						setStartCodingOpen(true);
					},
				},
			]
		: [];

	// Handle drag end for shortcuts reordering
	const sensors = useSensors(
		useSensor(PointerSensor, {
			activationConstraint: {
				distance: 8,
			},
		}),
	);

	const handleDragEnd = useCallback(
		async (event: DragEndEvent) => {
			const { active, over } = event;

			if (!over || active.id === over.id || !shortcuts) {
				return;
			}

			const oldIndex = shortcuts.findIndex((s) => s.id === active.id);
			const newIndex = shortcuts.findIndex((s) => s.id === over.id);

			if (oldIndex === -1 || newIndex === -1) {
				return;
			}

			// Reorder the shortcuts
			const reordered = [...shortcuts];
			const [moved] = reordered.splice(oldIndex, 1);
			reordered.splice(newIndex, 0, moved);

			// Update order values
			const updates = reordered.map((shortcut, index) => ({
				...shortcut,
				order: index,
			}));

			// Update in database
			try {
				await db.transaction("rw", db.shortcuts, async () => {
					for (const update of updates) {
						await db.shortcuts.update(update.id, { order: update.order });
					}
				});
				toast.success("Shortcuts reordered");
			} catch (error) {
				console.error("Failed to reorder shortcuts:", error);
				toast.error("Failed to reorder shortcuts");
			}
		},
		[shortcuts, db, toast],
	);

	if (predefinedShortcuts.length === 0 && (shortcuts?.length || 0) === 0) {
		return null;
	}

	return (
		<>
			<SidebarGroup>
				<SidebarGroupLabel>Shortcuts</SidebarGroupLabel>
				<SidebarMenu>
						{predefinedShortcuts.map((shortcut) => (
						<SidebarMenuItem key={shortcut.id}>
							<MotionSidebarMenuButton
								onClick={shortcut.action}
								tooltip={shortcut.label}
								initial="initial"
								whileHover="hover"
							>
								<motion.div variants={iconVariants}>
									<shortcut.icon className="size-4" />
								</motion.div>
								<span>{shortcut.label}</span>
							</MotionSidebarMenuButton>
						</SidebarMenuItem>
					))}

					<DndContext
						sensors={sensors}
						collisionDetection={closestCenter}
						onDragEnd={handleDragEnd}
					>
						<SortableContext
							items={shortcuts?.map((s) => s.id) ?? []}
							strategy={verticalListSortingStrategy}
						>
							{shortcuts?.map((shortcut) => (
								<SortableShortcutItem
									key={shortcut.id}
									shortcut={shortcut}
									pathname={pathname}
									sidebarState={sidebarState}
									db={db}
									toast={toast}
									getAppMetadata={getAppMetadata}
									getPageType={getPageType}
								/>
							))}
						</SortableContext>
					</DndContext>

					<SidebarMenuItem>
						<MotionSidebarMenuButton
							onClick={handleAddCurrentLocation}
							tooltip="Add Current Location"
							initial="initial"
							whileHover="hover"
						>
							<motion.div variants={iconVariants}>
								<AnimatedPinIcon className="size-4" />
							</motion.div>
							<span>Add Current Location</span>
						</MotionSidebarMenuButton>
					</SidebarMenuItem>
				</SidebarMenu>
			</SidebarGroup>

			{onCreateProject && (
				<Dialog open={startCodingOpen} onOpenChange={setStartCodingOpen}>
					<DialogContent>
						<DialogHeader>
							<DialogTitle className="flex items-center gap-2">
								<AutoPlayNewProjectIcon className="h-5 w-5"/>
								Create Flow
							</DialogTitle>
							<DialogDescription>
								Create a new project with all embedding models from your current
								profile
							</DialogDescription>
						</DialogHeader>
						<div className="grid gap-4 py-4">
							<div className="grid gap-2">
								<Label htmlFor="project-name">Project Name</Label>
								<Input
									id="project-name"
									placeholder="My Awesome Project"
									value={projectName}
									onChange={(e) => setProjectName(e.target.value)}
									onKeyDown={(e) => {
										if (e.key === "Enter" && !isCreating) {
											handleStartCoding();
										}
									}}
									disabled={isCreating}
								/>
							</div>
							<div className="grid gap-3">
								<Label>Connectivity</Label>
								<RadioGroup
									value={isOnline ? "online" : "offline"}
									onValueChange={(value) => {
										if (value === "online" && !auth?.isAuthenticated) {
											toast.error("Please log in to create online projects");
											return;
										}
										setIsOnline(value === "online");
									}}
									disabled={isCreating}
								>
									<div className="flex items-center space-x-2 relative">
										<RadioGroupItem
											value="online"
											id="online"
											disabled={!auth?.isAuthenticated || isCreating}
										/>
										<Label
											htmlFor="online"
											className={`flex items-center gap-2 font-normal ${
												auth?.isAuthenticated
													? "cursor-pointer"
													: "cursor-not-allowed opacity-50"
											}`}
										>
											<Globe className="h-4 w-4" />
											Online - Sync with cloud
											{!auth?.isAuthenticated && (
												<span className="text-xs text-muted-foreground ml-1">
													(Login required)
												</span>
											)}
										</Label>
									</div>
									{isTauri() ? (
										<div className="flex items-center space-x-2">
											<RadioGroupItem value="offline" id="offline" />
											<Label
												htmlFor="offline"
												className="flex items-center gap-2 font-normal cursor-pointer"
											>
												<WifiOff className="h-4 w-4" />
												Offline - Local only
											</Label>
										</div>
									) : (
										<div className="flex items-center space-x-2 opacity-50">
											<RadioGroupItem value="offline" id="offline" disabled />
											<Label
												htmlFor="offline"
												className="flex items-center gap-2 font-normal cursor-not-allowed"
											>
												<WifiOff className="h-4 w-4" />
												Offline - Local only
												<a
													href="https://flow-like.com/download"
													target="_blank"
													rel="noopener noreferrer"
													className="text-xs text-primary hover:underline flex items-center gap-1 ml-1"
													onClick={(e) => e.stopPropagation()}
												>
													(Get Studio <ExternalLink className="h-3 w-3" />)
												</a>
											</Label>
										</div>
									)}
								</RadioGroup>
							</div>
						</div>
						<DialogFooter>
							<DialogClose asChild>
								<Button variant="outline" disabled={isCreating}>
									Cancel
								</Button>
							</DialogClose>
							<Button onClick={handleStartCoding} disabled={isCreating}>
								{isCreating ? (
									<>
										<Loader2 className="mr-2 h-4 w-4 animate-spin" />
										Creating...
									</>
								) : (
									<>
										<Sparkles className="mr-2 h-4 w-4" />
										Create Project
									</>
								)}
							</Button>
						</DialogFooter>
					</DialogContent>
				</Dialog>
			)}
		</>
	);
}

interface SortableShortcutItemProps {
	shortcut: IShortcut;
	pathname: string;
	sidebarState: "expanded" | "collapsed";
	db: Dexie & { shortcuts: EntityTable<IShortcut, "id"> };
	toast: {
		success: (message: string) => void;
		error: (message: string) => void;
	};
	getAppMetadata: (appId: string) => { name?: string; icon?: string } | null;
	getPageType: (path: string) => { type: string; icon: LucideIcon } | null;
}

function SortableShortcutItem({
	shortcut,
	pathname,
	sidebarState,
	db,
	toast,
	getAppMetadata,
	getPageType,
}: SortableShortcutItemProps) {
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({
		id: shortcut.id,
	});

	const style: React.CSSProperties = {
		transform: transform ? CSS.Transform.toString(transform) : undefined,
		transition,
		opacity: isDragging ? 0.5 : 1,
	};

	const metadata = shortcut.appId ? getAppMetadata(shortcut.appId) : null;
	const pageType = getPageType(shortcut.path);
	const PageIcon = pageType?.icon;

	return (
		<SidebarMenuItem ref={setNodeRef} style={style}>
			<div className="group flex items-center w-full gap-1">
				{sidebarState === "expanded" && (
					<div
						{...attributes}
						{...listeners}
						className="cursor-grab active:cursor-grabbing p-1 hover:bg-accent rounded"
					>
						<GripVertical className="h-4 w-4 text-muted-foreground" />
					</div>
				)}
				<MotionSidebarMenuButton
					asChild
					className="flex-1 flex-row items-center"
					tooltip={shortcut.label}
					variant={pathname === shortcut.path ? "outline" : "default"}
				>
					<motion.a href={shortcut.path} className="flex items-center gap-2" initial="initial" whileHover="hover">
						{metadata ? (
							<motion.div variants={iconVariants} className="relative shrink-0">
								<Avatar className="h-6 w-6 -left-1">
									<AvatarImage
										src={metadata.icon ?? "/app-logo.webp"}
										alt={metadata.name ?? "App"}
										className="object-cover rounded-md"
									/>
									<AvatarFallback className="text-[9px] rounded-md">
										{(metadata.name ?? "A").substring(0, 2).toUpperCase()}
									</AvatarFallback>
								</Avatar>
								{PageIcon && (
									<div className="absolute -top-0.5 -right-0.5 bg-background rounded-full p-0.5">
										<PageIcon className="h-2.5 w-2.5 text-muted-foreground" />
									</div>
								)}
							</motion.div>
						) : (
							<motion.div variants={iconVariants}>
								<Bookmark className="h-4 w-4" />
							</motion.div>
						)}
						<span>{shortcut.label}</span>
					</motion.a>
				</MotionSidebarMenuButton>
				{sidebarState === "expanded" && (
					<Button
						variant="ghost"
						size="icon"
						className="h-8 w-8 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
						onClick={async (e) => {
							e.preventDefault();
							e.stopPropagation();
							try {
								await db.shortcuts.delete(shortcut.id);
								toast.success("Shortcut removed");
							} catch (error) {
								console.error("Failed to delete shortcut:", error);
								toast.error("Failed to remove shortcut");
							}
						}}
					>
						<Trash2 className="h-4 w-4" />
					</Button>
				)}
			</div>
		</SidebarMenuItem>
	);
}
