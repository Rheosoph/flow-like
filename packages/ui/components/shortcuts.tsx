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
import { motion } from "framer-motion";
import {
	Bookmark,
	Cable,
	Database,
	FolderClosed,
	GripVertical,
	type LucideIcon,
	Trash2,
	Workflow,
} from "lucide-react";
import { type ComponentType, useCallback, useState } from "react";
import { AnimatedPinIcon } from "./animated-icons";
import { AnimatedNewProjectIcon } from "./animated-icons/animated-plus";
import { CreateFlowDialog } from "./create-flow-dialog";
import { Avatar, AvatarFallback, AvatarImage } from "./ui/avatar";
import { Button } from "./ui/button";
import {
	SidebarGroup,
	SidebarGroupLabel,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	useSidebar,
} from "./ui/sidebar";

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

function getAppMetadataAppId(appData: unknown): string | undefined {
	if (Array.isArray(appData)) return getAppMetadataAppId(appData[0]);
	if (typeof appData !== "object" || appData === null || !("id" in appData)) {
		return undefined;
	}

	const id = (appData as { id?: unknown }).id;
	return typeof id === "string" ? id : undefined;
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
	onShortcutsChanged?: (
		profileId: string,
		shortcuts: IShortcut[],
	) => Promise<void>;
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
	onShortcutsChanged,
	bits,
}: ShortcutsProps<TBackend, TAppMetadata>) {
	const { state: sidebarState } = useSidebar();
	const [startCodingOpen, setStartCodingOpen] = useState(false);

	const syncShortcuts = useCallback(async () => {
		if (!currentProfileId || !onShortcutsChanged) return;

		try {
			const nextShortcuts = await db.shortcuts
				.where("profileId")
				.equals(currentProfileId)
				.sortBy("order");
			await onShortcutsChanged(currentProfileId, nextShortcuts);
		} catch (error) {
			console.warn("Failed to sync shortcuts:", error);
		}
	}, [currentProfileId, db, onShortcutsChanged]);

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
	const getAppMetadata = useCallback(
		(appId: string) => {
			if (!appMetadata) return null;
			return getAppMetadataById(appId, appMetadata);
		},
		[appMetadata, getAppMetadataById],
	);

	const handleAddCurrentLocation = useCallback(async () => {
		if (!currentProfileId) {
			toast.error("No profile selected");
			return;
		}

		const fullPath =
			typeof window !== "undefined"
				? window.location.pathname + window.location.search
				: pathname;

		// Extract appId from URL patterns. On /flow, id is a board id, not an app id.
		let appId: string | null = null;
		const queryString = fullPath.split("?")[1] ?? "";
		const searchParams = new URLSearchParams(queryString);
		const appParam = searchParams.get("app");
		const idParam = searchParams.get("id");
		appId = appParam || (pathname === "/flow" ? null : idParam);

		// If no app ID found, check if this is a flow page and find the app by board ID
		if (!appId && pathname === "/flow" && appMetadata) {
			const boardId = idParam;

			if (boardId) {
				// Search through all apps to find which one contains this board
				for (const appData of appMetadata) {
					try {
						const candidateAppId = getAppMetadataAppId(appData);
						if (!candidateAppId) continue;

						const boards = await getBoardsByAppId(backend, candidateAppId);
						if (boards?.some((board) => board.id === boardId)) {
							appId = candidateAppId;
							break;
						}
					} catch (error) {
						console.error("Failed to fetch boards for app:", error);
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
			const existingShortcut = shortcuts?.find(
				(shortcut) =>
					shortcut.profileId === currentProfileId && shortcut.path === fullPath,
			);
			if (existingShortcut) {
				await db.shortcuts.update(existingShortcut.id, {
					label,
					appId: appId || undefined,
				});
				await syncShortcuts();
				toast.success("Shortcut updated");
			} else {
				await db.shortcuts.add(shortcut);
				await syncShortcuts();
				toast.success("Shortcut added");
			}
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
		syncShortcuts,
		toast,
	]);

	const predefinedShortcuts: PredefinedShortcut[] = onCreateProject
		? [
				{
					id: "start-coding",
					label: "Create Flow",
					icon: AnimatedNewProjectIcon,
					action: () => {
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
				await syncShortcuts();
				toast.success("Shortcuts reordered");
			} catch (error) {
				console.error("Failed to reorder shortcuts:", error);
				toast.error("Failed to reorder shortcuts");
			}
		},
		[shortcuts, db, syncShortcuts, toast],
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
									onShortcutDeleted={syncShortcuts}
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
				<CreateFlowDialog
					open={startCodingOpen}
					onOpenChange={setStartCodingOpen}
					onCreateProject={onCreateProject}
					isAuthenticated={auth?.isAuthenticated}
					defaultOnline={auth?.isAuthenticated}
					toast={toast}
				/>
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
	onShortcutDeleted?: () => Promise<void>;
}

function SortableShortcutItem({
	shortcut,
	pathname,
	sidebarState,
	db,
	toast,
	getAppMetadata,
	getPageType,
	onShortcutDeleted,
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
					<motion.a
						href={shortcut.path}
						className="flex items-center gap-2"
						initial="initial"
						whileHover="hover"
					>
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
								await onShortcutDeleted?.();
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
