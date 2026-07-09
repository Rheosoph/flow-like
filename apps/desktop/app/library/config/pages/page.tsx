"use client";
import {
	Button,
	Card,
	CardContent,
	EmptyState,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import type { IOAuthProvider, IStoredOAuthToken } from "@flow-like/flow-like-ui";
import EventsPage from "@flow-like/flow-like-ui/components/settings/events/events-page";
import type { PageListItem } from "@flow-like/flow-like-ui/state/backend-state/page-state";
import {
	FileText,
	LayoutGrid,
	Pencil,
	Sparkles,
	Trash2,
	Workflow,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import { oauthConsentStore, oauthTokenStore } from "../../../../lib/oauth-db";
import { oauthService } from "../../../../lib/oauth-service";

export default function Page() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const backend = useBackend();
	const id = searchParams.get("id");

	const pages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		[id ?? ""],
		!!id,
		[id],
	);

	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id ?? ""],
		!!id,
		[id],
	);

	const handleDeletePage = useCallback(
		async (pageId: string, boardId: string | null) => {
			if (!id || !boardId) return;
			try {
				await backend.pageState.deletePage(id, pageId, boardId);
				pages.refetch();
			} catch (error) {
				console.error("Failed to delete page:", error);
			}
		},
		[id, backend.pageState, pages],
	);

	const openPageEditor = useCallback(
		(pageId: string, boardId?: string) => {
			if (!id) return;
			const url = boardId
				? `/page-builder?id=${pageId}&app=${id}&board=${boardId}`
				: `/page-builder?id=${pageId}&app=${id}`;
			router.push(url);
		},
		[id, router],
	);

	const openBoard = useCallback(
		(boardId: string) => {
			router.push(`/flow?id=${boardId}&app=${id}`);
		},
		[id, router],
	);

	return (
		<TooltipProvider>
			<main className="h-full flex flex-col max-h-full overflow-auto md:overflow-visible min-h-0">
				<div className="container mx-auto px-6 pb-4 flex flex-col h-full gap-6">
					{/* Header */}
					<div className="flex flex-col gap-2 pt-2">
						<h1 className="text-3xl font-bold tracking-tight">Events</h1>
						<p className="text-muted-foreground">
							Configure UI events, pages, and navigation paths
						</p>
					</div>

					<Tabs defaultValue="events" className="flex-1 flex flex-col">
						<TabsList className="w-fit">
							<TabsTrigger value="events" className="gap-2">
								<Sparkles className="h-4 w-4" />
								Events
							</TabsTrigger>
							<TabsTrigger value="pages" className="gap-2">
								<LayoutGrid className="h-4 w-4" />
								Pages
							</TabsTrigger>
						</TabsList>

						<TabsContent value="events" className="mt-6 flex-1">
							<EventsSection />
						</TabsContent>

						<TabsContent value="pages" className="mt-6 flex-1">
							<PagesSection
								pages={pages.data || []}
								onOpenPage={openPageEditor}
								onOpenBoard={openBoard}
								onDelete={handleDeletePage}
							/>
						</TabsContent>
					</Tabs>
				</div>
			</main>
		</TooltipProvider>
	);
}

// ============================================================================
// PAGES SECTION
// ============================================================================

function PagesSection({
	pages,
	onOpenPage,
	onOpenBoard,
	onDelete,
}: Readonly<{
	pages: PageListItem[];
	onOpenPage: (pageId: string, boardId?: string) => void;
	onOpenBoard: (boardId: string) => void;
	onDelete: (pageId: string, boardId: string | null) => Promise<void>;
}>) {
	if (pages.length === 0) {
		return (
			<EmptyState
				icons={[LayoutGrid]}
				title="No pages yet"
				description="Pages are created from within a flow. Open a flow and use the Pages panel to create your first page."
			/>
		);
	}

	return (
		<div className="space-y-4">
			<div className="space-y-1">
				<h2 className="text-lg font-semibold">All Pages</h2>
				<p className="text-sm text-muted-foreground">
					Manage your app's visual interfaces
				</p>
			</div>

			<div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
				{pages.map((pageInfo) => (
					<PageCard
						key={pageInfo.pageId}
						pageInfo={pageInfo}
						onOpen={() =>
							onOpenPage(pageInfo.pageId, pageInfo.boardId ?? undefined)
						}
						onOpenBoard={
							pageInfo.boardId
								? () => onOpenBoard(pageInfo.boardId!)
								: undefined
						}
						onDelete={() => onDelete(pageInfo.pageId, pageInfo.boardId ?? null)}
					/>
				))}
			</div>
		</div>
	);
}

function PageCard({
	pageInfo,
	onOpen,
	onOpenBoard,
	onDelete,
}: Readonly<{
	pageInfo: PageListItem;
	onOpen: () => void;
	onOpenBoard?: () => void;
	onDelete: () => void;
}>) {
	return (
		<Card className="group hover:shadow-lg transition-all duration-200 border-border/60 hover:border-primary/30 overflow-hidden">
			{/* Preview Area */}
			<div
				className="h-32 bg-linear-to-br from-muted/50 to-muted flex items-center justify-center cursor-pointer relative"
				onClick={onOpen}
			>
				<div className="absolute inset-0 bg-grid-pattern opacity-5" />
				<FileText className="h-12 w-12 text-muted-foreground/30" />
				<div className="absolute inset-0 bg-primary/5 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
					<Button variant="secondary" size="sm" className="gap-2">
						<Pencil className="h-4 w-4" />
						Edit Page
					</Button>
				</div>
			</div>

			<CardContent className="p-4">
				<div className="flex items-start justify-between gap-2">
					<div className="min-w-0 flex-1">
						<h3 className="font-semibold truncate">{pageInfo.name}</h3>
						{pageInfo.description && (
							<p className="text-sm text-muted-foreground line-clamp-1 mt-0.5">
								{pageInfo.description}
							</p>
						)}
					</div>
				</div>

				<div className="flex items-center justify-between mt-4 pt-3 border-t border-border/40">
					<div className="flex items-center gap-2">
						{onOpenBoard && (
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										variant="outline"
										size="sm"
										className="h-8 gap-1.5"
										onClick={onOpenBoard}
									>
										<Workflow className="h-3.5 w-3.5" />
										Flow
									</Button>
								</TooltipTrigger>
								<TooltipContent>Open connected flow</TooltipContent>
							</Tooltip>
						)}
					</div>
					<div className="flex items-center gap-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8"
									onClick={onOpen}
								>
									<Pencil className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>Edit Page</TooltipContent>
						</Tooltip>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 text-destructive hover:text-destructive hover:bg-destructive/10"
									onClick={onDelete}
								>
									<Trash2 className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>Delete Page</TooltipContent>
						</Tooltip>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

// ============================================================================
// EVENTS SECTION
// ============================================================================

function EventsSection() {
	const handleStartOAuth = useCallback(async (provider: IOAuthProvider) => {
		await oauthService.startAuthorization(provider);
	}, []);

	const handleRefreshToken = useCallback(
		async (provider: IOAuthProvider, token: IStoredOAuthToken) => {
			return oauthService.refreshToken(provider, token);
		},
		[],
	);

	const uiEventTypes = useMemo(() => {
		const set = new Set<string>();
		Object.values(EVENT_CONFIG).forEach((cfg: any) => {
			Object.keys(cfg?.useInterfaces ?? {}).forEach((t) => set.add(t));
		});
		return Array.from(set);
	}, []);

	return (
		<EventsPage
			eventMapping={EVENT_CONFIG}
			uiEventTypes={uiEventTypes}
			tokenStore={oauthTokenStore}
			consentStore={oauthConsentStore}
			onStartOAuth={handleStartOAuth}
			onRefreshToken={handleRefreshToken}
			basePath="/library/config/pages"
		/>
	);
}
