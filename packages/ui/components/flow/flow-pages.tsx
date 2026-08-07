"use client";

import { createId } from "@paralleldrive/cuid2";
import { FileTextIcon, Pencil, PlusIcon, Trash2Icon } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	EmptyState,
	Input,
	Label,
	ScrollArea,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui";

interface FlowPagesProps {
	appId: string;
	boardId: string;
	onOpenPage?: (pageId: string, boardId: string) => void;
}

export function FlowPages({ appId, boardId, onOpenPage }: FlowPagesProps) {
	const backend = useBackend();
	const [createDialogOpen, setCreateDialogOpen] = useState(false);
	const [newPageName, setNewPageName] = useState("");
	const [isCreating, setIsCreating] = useState(false);

	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[appId, boardId],
		!!appId && !!boardId,
		[appId, boardId],
	);

	const handleCreatePage = useCallback(async () => {
		if (!newPageName.trim()) {
			toast.error("Please enter a page name");
			return;
		}

		setIsCreating(true);
		try {
			const pageId = createId();
			const route = `/${newPageName.trim().toLowerCase().replace(/\s+/g, "-")}`;
			await backend.pageState.createPage(
				appId,
				pageId,
				newPageName.trim(),
				route,
				boardId,
			);
			toast.success("Page created successfully");
			setCreateDialogOpen(false);
			setNewPageName("");
			pages.refetch();
		} catch (error) {
			console.error("Failed to create page:", error);
			toast.error("Failed to create page");
		} finally {
			setIsCreating(false);
		}
	}, [appId, boardId, newPageName, backend.pageState, pages]);

	const handleDeletePage = useCallback(
		async (pageId: string) => {
			try {
				await backend.pageState.deletePage(appId, pageId, boardId);
				toast.success("Page deleted");
				pages.refetch();
			} catch (error) {
				console.error("Failed to delete page:", error);
				toast.error("Failed to delete page");
			}
		},
		[appId, boardId, backend.pageState, pages],
	);

	const handleOpenPage = useCallback(
		(pageId: string) => {
			if (onOpenPage) {
				onOpenPage(pageId, boardId);
			} else {
				window.open(
					`/page-builder?id=${pageId}&app=${appId}&board=${boardId}`,
					"_blank",
				);
			}
		},
		[appId, boardId, onOpenPage],
	);

	return (
		<TooltipProvider>
			<div className="flex flex-col h-full">
				<div className="flex items-center justify-between p-4 border-b">
					<div>
						<h3 className="font-semibold">Pages</h3>
						<p className="text-sm text-muted-foreground">UI for this flow</p>
					</div>
					<Button size="sm" onClick={() => setCreateDialogOpen(true)}>
						<PlusIcon className="h-4 w-4 mr-1" />
						New
					</Button>
				</div>

				<ScrollArea className="flex-1">
					<div className="p-3 space-y-2">
						{pages.isLoading ? (
							<div className="space-y-2">
								{[1, 2].map((i) => (
									<div
										key={i}
										className="animate-pulse h-20 bg-muted rounded-lg"
									/>
								))}
							</div>
						) : pages.data && pages.data.length > 0 ? (
							pages.data.map((pageInfo) => (
								<PageCard
									key={pageInfo.pageId}
									pageId={pageInfo.pageId}
									name={pageInfo.name}
									description={pageInfo.description}
									unavailable={pageInfo.unavailable === true}
									onOpen={() => handleOpenPage(pageInfo.pageId)}
									onDelete={() => handleDeletePage(pageInfo.pageId)}
								/>
							))
						) : (
							<EmptyState
								icons={[FileTextIcon]}
								title="No pages yet"
								description="Create a page to build UI for this flow."
								action={{
									label: "Create Page",
									onClick: () => setCreateDialogOpen(true),
								}}
							/>
						)}
					</div>
				</ScrollArea>

				<Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
					<DialogContent>
						<DialogHeader>
							<DialogTitle>Create New Page</DialogTitle>
							<DialogDescription>
								Add a new page to this board. Pages can be connected to routes
								for navigation.
							</DialogDescription>
						</DialogHeader>
						<div className="space-y-4 py-4">
							<div className="space-y-2">
								<Label htmlFor="page-name">Page Name</Label>
								<Input
									id="page-name"
									placeholder="Enter page name..."
									value={newPageName}
									onChange={(e) => setNewPageName(e.target.value)}
									onKeyDown={(e) => {
										if (e.key === "Enter" && !isCreating) {
											handleCreatePage();
										}
									}}
								/>
							</div>
						</div>
						<DialogFooter>
							<Button
								variant="outline"
								onClick={() => setCreateDialogOpen(false)}
							>
								Cancel
							</Button>
							<Button onClick={handleCreatePage} disabled={isCreating}>
								{isCreating ? "Creating..." : "Create Page"}
							</Button>
						</DialogFooter>
					</DialogContent>
				</Dialog>
			</div>
		</TooltipProvider>
	);
}

function PageCard({
	pageId,
	name,
	description,
	unavailable,
	onOpen,
	onDelete,
}: {
	pageId: string;
	name: string;
	description?: string;
	unavailable?: boolean;
	onOpen: () => void;
	onDelete: () => void;
}) {
	const [isDeleting, setIsDeleting] = useState(false);

	const handleDelete = async (e: React.MouseEvent) => {
		e.stopPropagation();
		setIsDeleting(true);
		try {
			await onDelete();
		} finally {
			setIsDeleting(false);
		}
	};

	return (
		<div
			className="group flex items-center gap-3 p-2.5 rounded-lg border border-border/60 hover:border-primary/30 hover:bg-accent/50 cursor-pointer transition-all"
			onClick={onOpen}
		>
			<div className="h-9 w-9 rounded-md bg-gradient-to-br from-primary/15 to-primary/5 flex items-center justify-center shrink-0 group-hover:from-primary/25 group-hover:to-primary/10 transition-colors">
				<FileTextIcon className="h-4 w-4 text-primary/70" />
			</div>
			<div className="min-w-0 flex-1">
				<h4 className="font-medium text-sm truncate">{name}</h4>
				{unavailable ? (
					// The board still lists the page, so it is not deleted — its content just
					// isn't readable here. Naming that beats a card that opens onto nothing.
					<p className="text-xs text-amber-600 dark:text-amber-400 line-clamp-1">
						Content unavailable on this device
					</p>
				) : (
					description && (
						<p className="text-xs text-muted-foreground line-clamp-1">
							{description}
						</p>
					)
				)}
			</div>
			<div className="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7"
							onClick={(e) => {
								e.stopPropagation();
								onOpen();
							}}
						>
							<Pencil className="h-3.5 w-3.5" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom">Edit</TooltipContent>
				</Tooltip>
				<Tooltip>
					<TooltipTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className="h-7 w-7 text-destructive hover:text-destructive hover:bg-destructive/10"
							onClick={handleDelete}
							disabled={isDeleting}
						>
							<Trash2Icon className="h-3.5 w-3.5" />
						</Button>
					</TooltipTrigger>
					<TooltipContent side="bottom">Delete</TooltipContent>
				</Tooltip>
			</div>
		</div>
	);
}
