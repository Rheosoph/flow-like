"use client";
import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	FileText,
	Globe,
	Pencil,
	Plus,
	Route as RouteIcon,
	Sparkles,
	Trash2,
	Workflow,
} from "lucide-react";
import { useState } from "react";
import type { IMetadata } from "../../../types";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card, CardContent } from "../../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { EmptyState } from "../../ui/empty-state";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

export type RouteTargetType = "page" | "event";

export type Version = [number, number, number];

export interface IAppRoute {
	id: string;
	path: string;
	targetType: RouteTargetType;
	pageId?: string;
	boardId?: string;
	pageVersion?: Version;
	eventId?: string;
	priority: number;
}

export interface CreateAppRoute {
	path: string;
	targetType: RouteTargetType;
	pageId?: string;
	boardId?: string;
	pageVersion?: Version;
	eventId?: string;
	priority: number;
}

export interface UpdateAppRoute {
	path?: string;
	targetType?: RouteTargetType;
	pageId?: string;
	boardId?: string;
	pageVersion?: Version;
	eventId?: string;
	priority?: number;
}

export interface PageData {
	appId: string;
	pageId: string;
	boardId: string | null;
	metadata: IMetadata;
}

export interface EventData {
	id: string;
	name: string;
	event_type: string;
}

export interface RoutesSectionProps {
	appId: string;
	routes: IAppRoute[];
	pages: PageData[];
	events: EventData[];
	onCreate: (route: CreateAppRoute) => Promise<void>;
	onUpdate: (routeId: string, updates: UpdateAppRoute) => Promise<void>;
	onDelete: (routeId: string) => Promise<void>;
	onOpenPage: (pageId: string, boardId?: string) => void;
	onOpenBoard: (boardId: string) => void;
	getBoardVersions?: (boardId: string) => Promise<Version[]>;
}

export function RoutesSection({
	appId,
	routes,
	pages,
	events,
	onCreate,
	onDelete,
	onOpenPage,
	onOpenBoard,
	getBoardVersions,
}: RoutesSectionProps) {
	const { t } = useTranslation("settings");
	const [isCreateOpen, setIsCreateOpen] = useState(false);

	return (
		<div className="space-y-4">
			<div className="flex justify-between items-start">
				<div className="space-y-1">
					<h2 className="text-lg font-semibold">
						{t("urlRoutes", "URL Routes")}
					</h2>
					<p className="text-sm text-muted-foreground">
						{t(
							"mapUrlPathsToPagesOrEventsTheRouteIsTheDefaultEntryPoint",
							'Map URL paths to pages or events. The "/" route is the default entry point.',
						)}
					</p>
				</div>
				<Button onClick={() => setIsCreateOpen(true)} size="sm">
					<Plus className="h-4 w-4 mr-2" />
					{t("addRoute", "Add Route")}
				</Button>
			</div>

			{routes.length > 0 ? (
				<div className="space-y-3">
					{routes.map((route) => (
						<RouteCard
							key={route.id}
							route={route}
							pages={pages}
							events={events}
							onDelete={onDelete}
							onOpenPage={onOpenPage}
							onOpenBoard={onOpenBoard}
						/>
					))}
				</div>
			) : (
				<EmptyState
					icons={[Globe]}
					title={t("noRoutesConfigured", "No routes configured")}
					description={t(
						"routesMapUrlPathsToPagesOrEventsCreateYourFirstRouteToDefineHowUsersNavigateYourApp",
						"Routes map URL paths to pages or events. Create your first route to define how users navigate your app.",
					)}
					action={{
						label: "Create Route",
						onClick: () => setIsCreateOpen(true),
					}}
					className="w-full"
				/>
			)}

			<CreateRouteDialog
				open={isCreateOpen}
				onOpenChange={setIsCreateOpen}
				pages={pages}
				events={events}
				onCreate={onCreate}
				getBoardVersions={getBoardVersions}
			/>
		</div>
	);
}

export interface RouteCardProps {
	route: IAppRoute;
	pages: PageData[];
	events: EventData[];
	onDelete: (routeId: string) => Promise<void>;
	onOpenPage: (pageId: string, boardId?: string) => void;
	onOpenBoard: (boardId: string) => void;
}

export function RouteCard({
	route,
	pages,
	events,
	onDelete,
	onOpenPage,
	onOpenBoard,
}: RouteCardProps) {
	const { t } = useTranslation("settings");
	const pageInfo =
		route.targetType === "page"
			? pages.find((p) => p.pageId === route.pageId)
			: null;
	const eventInfo =
		route.targetType === "event"
			? events.find((e) => e.id === route.eventId)
			: null;

	const targetName = pageInfo?.metadata?.name || eventInfo?.name || "Unknown";
	const isDefaultRoute = route.path === "/";

	return (
		<Card className="group hover:shadow-md transition-all duration-200 border-border/60 hover:border-primary/30">
			<CardContent className="p-0">
				<div className="flex items-stretch">
					{/* Route Path Section */}
					<div className="flex items-center gap-3 p-4 border-r border-border/40 min-w-[180px]">
						<div
							className={`p-2.5 rounded-xl ${isDefaultRoute ? "bg-primary text-primary-foreground" : "bg-primary/10 text-primary"}`}
						>
							<RouteIcon className="h-5 w-5" />
						</div>
						<div>
							<code className="font-mono text-base font-semibold">
								{route.path}
							</code>
							{isDefaultRoute && (
								<p className="text-xs text-muted-foreground mt-0.5">
									{t("homePage", "Home Page")}
								</p>
							)}
						</div>
					</div>

					{/* Arrow */}
					<div className="flex items-center px-4 text-muted-foreground/50">
						<ArrowRight className="h-5 w-5" />
					</div>

					{/* Target Section */}
					<div className="flex-1 flex items-center gap-3 p-4">
						<div
							className={`p-2.5 rounded-xl ${route.targetType === "page" ? "bg-blue-500/10 text-blue-600 dark:text-blue-400" : "bg-amber-500/10 text-amber-600 dark:text-amber-400"}`}
						>
							{route.targetType === "page" ? (
								<FileText className="h-5 w-5" />
							) : (
								<Sparkles className="h-5 w-5" />
							)}
						</div>
						<div className="flex-1 min-w-0">
							<div className="flex items-center gap-2">
								<span className="font-medium truncate">{targetName}</span>
								<Badge
									variant="secondary"
									className="text-xs capitalize shrink-0"
								>
									{route.targetType}
								</Badge>
								{route.targetType === "page" && (
									<Badge
										variant="outline"
										className="text-xs shrink-0 font-mono"
									>
										{route.pageVersion
											? `v${route.pageVersion.join(".")}`
											: "Latest"}
									</Badge>
								)}
							</div>
							{pageInfo?.boardId && (
								<p className="text-xs text-muted-foreground mt-0.5 truncate">
									{t("board", "Board:")} {pageInfo.boardId.substring(0, 12)}
									{`...`}
								</p>
							)}
						</div>
					</div>

					{/* Actions */}
					<div className="flex items-center gap-1 px-3 border-l border-border/40">
						{route.targetType === "page" && pageInfo && (
							<>
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="ghost"
											size="icon"
											className="h-9 w-9"
											onClick={() =>
												onOpenPage(route.pageId!, pageInfo.boardId ?? undefined)
											}
										>
											<Pencil className="h-4 w-4" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>{t("editPage", "Edit Page")}</TooltipContent>
								</Tooltip>
								{pageInfo.boardId && (
									<Tooltip>
										<TooltipTrigger asChild>
											<Button
												variant="ghost"
												size="icon"
												className="h-9 w-9"
												onClick={() => onOpenBoard(pageInfo.boardId!)}
											>
												<Workflow className="h-4 w-4" />
											</Button>
										</TooltipTrigger>
										<TooltipContent>
											{t("openFlow", "Open Flow")}
										</TooltipContent>
									</Tooltip>
								)}
							</>
						)}
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-9 w-9 text-destructive hover:text-destructive hover:bg-destructive/10"
									onClick={() => onDelete(route.id)}
								>
									<Trash2 className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>
								{t("deleteRoute", "Delete Route")}
							</TooltipContent>
						</Tooltip>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

export interface CreateRouteDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	pages: PageData[];
	events: EventData[];
	onCreate: (route: CreateAppRoute) => Promise<void>;
	getBoardVersions?: (boardId: string) => Promise<Version[]>;
}

export function CreateRouteDialog({
	open,
	onOpenChange,
	pages,
	events,
	onCreate,
	getBoardVersions,
}: CreateRouteDialogProps) {
	const { t } = useTranslation("settings");
	const [path, setPath] = useState("/");
	const [targetType, setTargetType] = useState<RouteTargetType>("page");
	const [pageId, setPageId] = useState("");
	const [boardId, setBoardId] = useState<string | undefined>(undefined);
	const [eventId, setEventId] = useState("");
	const [selectedVersion, setSelectedVersion] = useState<string>("latest");
	const [isLoading, setIsLoading] = useState(false);
	const [boardVersions, setBoardVersions] = useState<Version[]>([]);

	const handleCreate = async () => {
		setIsLoading(true);
		try {
			const pageVersion =
				targetType === "page" && selectedVersion !== "latest"
					? (selectedVersion.split(".").map(Number) as Version)
					: undefined;
			await onCreate({
				path,
				targetType,
				pageId: targetType === "page" ? pageId : undefined,
				boardId: targetType === "page" ? boardId : undefined,
				pageVersion,
				eventId: targetType === "event" ? eventId : undefined,
				priority: 0,
			});
			onOpenChange(false);
			resetForm();
		} finally {
			setIsLoading(false);
		}
	};

	const resetForm = () => {
		setPath("/");
		setTargetType("page");
		setPageId("");
		setBoardId(undefined);
		setEventId("");
		setSelectedVersion("latest");
		setBoardVersions([]);
	};

	const handlePageSelect = async (value: string) => {
		setPageId(value);
		setSelectedVersion("latest");
		const page = pages.find((p) => p.pageId === value);
		if (page?.boardId) {
			setBoardId(page.boardId);
			if (getBoardVersions) {
				const versions = await getBoardVersions(page.boardId);
				setBoardVersions(versions);
			}
		} else {
			setBoardId(undefined);
			setBoardVersions([]);
		}
	};

	const isValid =
		path &&
		((targetType === "page" && pageId) || (targetType === "event" && eventId));

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t("createRoute", "Create Route")}</DialogTitle>
					<DialogDescription>
						{t(
							"defineAUrlPathAndWhatItDisplays",
							"Define a URL path and what it displays",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="path">{t("path", "Path")}</Label>
						<Input
							id="path"
							value={path}
							onChange={(e) => setPath(e.target.value)}
							placeholder="/about"
							className="font-mono"
						/>
						<p className="text-xs text-muted-foreground">
							{t("useForTheHomePage", 'Use "/" for the home page')}
						</p>
					</div>

					<div className="space-y-2">
						<Label>{t("targetType", "Target Type")}</Label>
						<Select
							value={targetType}
							onValueChange={(v) => setTargetType(v as RouteTargetType)}
						>
							<SelectTrigger>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="page">
									<div className="flex items-center gap-2">
										<FileText className="h-4 w-4" />
										{t("page", "Page")}
									</div>
								</SelectItem>
								<SelectItem value="event">
									<div className="flex items-center gap-2">
										<Sparkles className="h-4 w-4" />
										{t("event", "Event")}
									</div>
								</SelectItem>
							</SelectContent>
						</Select>
					</div>

					{targetType === "page" && (
						<>
							<div className="space-y-2">
								<Label>{t("page", "Page")}</Label>
								<Select value={pageId} onValueChange={handlePageSelect}>
									<SelectTrigger>
										<SelectValue
											placeholder={t("selectAPage", "Select a page")}
										/>
									</SelectTrigger>
									<SelectContent>
										{pages.length === 0 ? (
											<div className="p-2 text-sm text-muted-foreground text-center">
												{t(
													"noPagesAvailableCreateOneInAFlowFirst",
													"No pages available. Create one in a flow first.",
												)}
											</div>
										) : (
											pages.map((page) => (
												<SelectItem key={page.pageId} value={page.pageId}>
													{page.metadata.name}
												</SelectItem>
											))
										)}
									</SelectContent>
								</Select>
							</div>
							{boardId && (
								<div className="space-y-2">
									<Label>{t("version", "Version")}</Label>
									<Select
										value={selectedVersion}
										onValueChange={setSelectedVersion}
									>
										<SelectTrigger>
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											<SelectItem value="latest">
												{t("latest", "Latest")}
											</SelectItem>
											{boardVersions.map((v) => (
												<SelectItem key={v.join(".")} value={v.join(".")}>
													v{v.join(".")}
												</SelectItem>
											))}
										</SelectContent>
									</Select>
									<p className="text-xs text-muted-foreground">
										{selectedVersion === "latest"
											? t(
													"alwaysUseTheLatestPublishedVersion",
													"Always use the latest published version",
												)
											: t(
													"lockToVersionSelectedversion",
													"Lock to version {{selectedVersion}}",
													{ selectedVersion },
												)}
									</p>
								</div>
							)}
						</>
					)}

					{targetType === "event" && (
						<div className="space-y-2">
							<Label>{t("event", "Event")}</Label>
							<Select value={eventId} onValueChange={setEventId}>
								<SelectTrigger>
									<SelectValue
										placeholder={t("selectAnEvent", "Select an event")}
									/>
								</SelectTrigger>
								<SelectContent>
									{events.length === 0 ? (
										<div className="p-2 text-sm text-muted-foreground text-center">
											{t("noEventsAvailable", "No events available")}
										</div>
									) : (
										events.map((event) => (
											<SelectItem key={event.id} value={event.id}>
												{event.name}
											</SelectItem>
										))
									)}
								</SelectContent>
							</Select>
						</div>
					)}
				</div>
				<DialogFooter>
					<Button variant="outline" onClick={() => onOpenChange(false)}>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={handleCreate} disabled={isLoading || !isValid}>
						{isLoading ? "Creating..." : t("createRoute", "Create Route")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
