"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	ArrowRight,
	BarChart3,
	CheckCircle,
	Download,
	Eye,
	FileText,
	LayoutGrid,
	MessageSquare,
	Package,
	PauseCircle,
	ShieldAlert,
	Star,
	X,
	XCircle,
	Zap,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useFeatures } from "../../../hooks/use-features";
import { useInvoke } from "../../../hooks/use-invoke";
import { presignCanvasSettings, presignPageAssets } from "../../../lib";
import type { IBoard } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import type { IPage } from "../../../state/backend-state/page-state";
import { A2UIRenderer, type Surface, type SurfaceComponent } from "../../a2ui";
import { AdminBoardPreview } from "../../flow/admin-board-preview";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	Dialog,
	DialogContent,
	DialogTitle,
	RelativeTime,
	Separator,
	Skeleton,
	Textarea,
} from "../../ui";
import { AdminAiActAssessmentCard } from "./admin-ai-act-assessment-card";

type RequestStatus = "pending" | "on_hold" | "accepted" | "rejected";

interface BoardScores {
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
}

const SCORE_CATEGORIES = [
	"security",
	"privacy",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

type ScoreCategory = (typeof SCORE_CATEGORIES)[number];

const SCORE_LABELS: Record<ScoreCategory, string> = {
	security: "Security",
	privacy: "Privacy",
	performance: "Performance",
	governance: "Governance",
	reliability: "Reliability",
	cost: "Cost",
};

interface FlaggedPattern {
	node: string;
	category: string;
	score: number;
	count?: number;
}

interface BoardScoreItem {
	boardId: string;
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
	worstScore: number;
	nodeCount: number;
	scoredNodeCount: number;
	flaggedPatterns: FlaggedPattern[];
	computedAt: string;
	updatedAt: string;
}

interface AppScoreDetailResponse {
	appId: string;
	appName?: string | null;
	boards: BoardScoreItem[];
}

interface PageInfo {
	appId: string;
	pageId: string;
	boardId?: string;
	name: string;
	description?: string;
}

interface BoardSummary {
	id: string;
	name: string;
	description: string;
	stage: string;
	executionMode: string;
	logLevel: number;
	version: [number, number, number];
	nodeCount: number;
	connectionCount: number;
	variableCount: number;
	layerCount: number;
	commentCount: number;
	scores?: BoardScores;
	pages: PageInfo[];
}

interface EventSummary {
	id: string;
	name: string;
	description: string;
	boardId: string;
	eventType: string;
	active: boolean;
	priority: number;
	route?: string;
	isDefault: boolean;
	version: [number, number, number];
	defaultPageId?: string;
}

interface AppContentResponse {
	boards: BoardSummary[];
	events: EventSummary[];
	pages: PageInfo[];
}

function buildPageSurface(page: IPage): Surface | null {
	if (!page.components || page.components.length === 0) return null;

	const components: Record<string, SurfaceComponent> = {};
	for (const component of page.components) {
		components[component.id] = component;
	}

	const rootComponentId = components.root ? "root" : page.components[0]?.id;
	if (!rootComponentId) return null;

	return {
		id: page.id,
		rootComponentId,
		components,
		canvasSettings: page.canvasSettings,
	};
}

function AdminPagePreview({
	page,
	appId,
}: {
	page: IPage;
	appId: string;
}) {
	const surface = useMemo(() => buildPageSurface(page), [page]);

	if (!surface) {
		return (
			<div className="flex h-full items-center justify-center text-sm text-muted-foreground">
				No page content to display.
			</div>
		);
	}

	return (
		<div className="h-full w-full overflow-auto bg-background">
			<A2UIRenderer
				surface={surface}
				widgetRefs={page.widgetRefs}
				className="min-h-full w-full"
				appId={appId}
				boardId={page.boardId}
				isPreviewMode={false}
			/>
		</div>
	);
}

interface PublicationActor {
	userId: string;
	username?: string;
	name?: string;
	avatar?: string;
	email?: string;
}

interface PublicationLogItem {
	id: string;
	authorId?: string;
	author?: PublicationActor;
	message?: string;
	visibility?: string;
	createdAt: string;
}

interface AppPublicationRequest {
	id: string;
	appId: string;
	targetVisibility: string;
	status: RequestStatus;
	approverId?: string;
	createdAt: string;
	updatedAt: string;
	appName?: string;
	appDescription?: string;
	appIcon?: string;
	appThumbnail?: string;
	appTags?: string[];
	currentVisibility?: string;
	downloadCount?: number;
	ratingCount?: number;
	avgRating?: number;
	boardCount?: number;
	packageCount?: number;
	logs: PublicationLogItem[];
}

interface RawPublicationActor {
	userId?: string;
	user_id?: string;
	username?: string;
	name?: string;
	avatar?: string;
	email?: string;
}

interface RawPublicationLogItem {
	id: string;
	authorId?: string;
	author_id?: string;
	author?: RawPublicationActor;
	message?: string;
	visibility?: string;
	createdAt?: string;
	created_at?: string;
}

interface RawAppPublicationRequest {
	id: string;
	appId?: string;
	app_id?: string;
	targetVisibility?: string;
	target_visibility?: string;
	status: string;
	approverId?: string;
	approver_id?: string;
	createdAt?: string;
	created_at?: string;
	updatedAt?: string;
	updated_at?: string;
	appName?: string;
	app_name?: string;
	appDescription?: string;
	app_description?: string;
	appIcon?: string;
	app_icon?: string;
	appThumbnail?: string;
	app_thumbnail?: string;
	appTags?: string[];
	app_tags?: string[];
	currentVisibility?: string;
	current_visibility?: string;
	downloadCount?: number;
	download_count?: number;
	ratingCount?: number;
	rating_count?: number;
	avgRating?: number;
	avg_rating?: number;
	boardCount?: number;
	board_count?: number;
	packageCount?: number;
	package_count?: number;
	logs?: RawPublicationLogItem[];
}

interface RawListResponse {
	requests: RawAppPublicationRequest[];
	total: number;
	page: number;
	limit: number;
	hasMore?: boolean;
	has_more?: boolean;
}

function normalizeRequestStatus(status: string): RequestStatus {
	switch (status.toLowerCase()) {
		case "pending":
			return "pending";
		case "on_hold":
			return "on_hold";
		case "accepted":
			return "accepted";
		case "rejected":
			return "rejected";
		default:
			return "pending";
	}
}

function normalizeActor(
	raw?: RawPublicationActor,
): PublicationActor | undefined {
	if (!raw) return undefined;
	return {
		userId: raw.userId ?? raw.user_id ?? "",
		username: raw.username,
		name: raw.name,
		avatar: raw.avatar,
		email: raw.email,
	};
}

function normalizeRequest(
	raw: RawAppPublicationRequest,
): AppPublicationRequest {
	return {
		id: raw.id,
		appId: raw.appId ?? raw.app_id ?? "",
		targetVisibility: (
			raw.targetVisibility ??
			raw.target_visibility ??
			""
		).toLowerCase(),
		status: normalizeRequestStatus(raw.status),
		approverId: raw.approverId ?? raw.approver_id,
		createdAt: raw.createdAt ?? raw.created_at ?? "",
		updatedAt: raw.updatedAt ?? raw.updated_at ?? "",
		appName: raw.appName ?? raw.app_name,
		appDescription: raw.appDescription ?? raw.app_description,
		appIcon: raw.appIcon ?? raw.app_icon,
		appThumbnail: raw.appThumbnail ?? raw.app_thumbnail,
		appTags: raw.appTags ?? raw.app_tags,
		currentVisibility: (
			raw.currentVisibility ??
			raw.current_visibility ??
			""
		).toLowerCase(),
		downloadCount: raw.downloadCount ?? raw.download_count ?? 0,
		ratingCount: raw.ratingCount ?? raw.rating_count ?? 0,
		avgRating: raw.avgRating ?? raw.avg_rating,
		boardCount: raw.boardCount ?? raw.board_count ?? 0,
		packageCount: raw.packageCount ?? raw.package_count ?? 0,
		logs: (raw.logs ?? []).map((log) => ({
			id: log.id,
			authorId: log.authorId ?? log.author_id,
			author: normalizeActor(log.author),
			message: log.message,
			visibility: log.visibility,
			createdAt: log.createdAt ?? log.created_at ?? "",
		})),
	};
}

const STATUS_BADGE_VARIANT: Record<
	string,
	"default" | "secondary" | "destructive"
> = {
	pending: "secondary",
	on_hold: "secondary",
	accepted: "default",
	rejected: "destructive",
};

function statusVariant(
	status: string,
): "default" | "secondary" | "destructive" {
	return STATUS_BADGE_VARIANT[status] ?? "secondary";
}

function formatStatusLabel(status: string) {
	return status.replaceAll("_", " ");
}

function formatDownloadCount(count: number | null | undefined) {
	return (count ?? 0).toLocaleString();
}

function DetailSkeleton() {
	return (
		<div className="space-y-6">
			<div className="flex items-center gap-4">
				<Skeleton className="h-8 w-8" />
				<Skeleton className="h-6 w-48" />
			</div>
			<Card>
				<CardContent className="p-6 space-y-6">
					<div className="flex items-start gap-6">
						<Skeleton className="h-20 w-20 rounded-xl" />
						<div className="flex-1 space-y-3">
							<Skeleton className="h-6 w-64" />
							<Skeleton className="h-4 w-full max-w-lg" />
							<Skeleton className="h-4 w-32" />
						</div>
					</div>
					<Skeleton className="h-24 w-full" />
					<Skeleton className="h-32 w-full" />
				</CardContent>
			</Card>
		</div>
	);
}

function ReviewTimeline({ logs }: { logs: PublicationLogItem[] }) {
	if (logs.length === 0) return null;

	return (
		<div className="space-y-3">
			<div className="flex items-center gap-2 text-sm font-medium text-muted-foreground">
				<MessageSquare className="h-4 w-4" />
				Review History ({logs.length})
			</div>
			<div className="space-y-2 pl-2 border-l-2 border-border/60">
				{logs.map((log) => (
					<div key={log.id} className="pl-4 py-1.5">
						<div className="flex items-center gap-2 text-sm">
							{log.author ? (
								<>
									<Avatar className="h-5 w-5">
										<AvatarImage src={log.author.avatar ?? undefined} />
										<AvatarFallback className="text-[10px]">
											{(log.author.name ?? log.author.username ?? "?")
												.substring(0, 2)
												.toUpperCase()}
										</AvatarFallback>
									</Avatar>
									<span className="font-medium">
										{log.author.name ?? log.author.username ?? "Unknown"}
									</span>
									{log.author.email && (
										<a
											href={`mailto:${log.author.email}`}
											className="text-xs text-muted-foreground hover:text-foreground transition-colors"
										>
											{log.author.email}
										</a>
									)}
								</>
							) : (
								<span className="text-muted-foreground">System</span>
							)}
							{log.visibility && (
								<Badge variant="outline" className="text-[10px] px-1.5 py-0">
									{log.visibility.toLowerCase()}
								</Badge>
							)}
							<span className="text-xs text-muted-foreground ml-auto">
								<RelativeTime value={log.createdAt} fallback={log.createdAt} />
							</span>
						</div>
						{log.message && (
							<p className="text-sm text-muted-foreground mt-1">
								{log.message}
							</p>
						)}
					</div>
				))}
			</div>
		</div>
	);
}

function ScoreBar({ label, value }: { label: string; value: number }) {
	const color =
		value >= 7 ? "bg-green-500" : value >= 4 ? "bg-yellow-500" : "bg-red-500";
	return (
		<div className="flex items-center gap-2 text-xs">
			<span className="w-20 text-muted-foreground">{label}</span>
			<div className="flex-1 h-1.5 bg-muted rounded-full overflow-hidden">
				<div
					className={`h-full rounded-full ${color}`}
					style={{ width: `${value * 10}%` }}
				/>
			</div>
			<span className="w-4 text-right font-medium">{value}</span>
		</div>
	);
}

function scoreTextColor(value: number): string {
	if (value >= 7) return "text-green-600 dark:text-green-400";
	if (value >= 4) return "text-yellow-600 dark:text-yellow-400";
	return "text-red-600 dark:text-red-400";
}

function scoreBgColor(value: number): string {
	if (value >= 7) return "bg-green-500";
	if (value >= 4) return "bg-yellow-500";
	return "bg-red-500";
}

function scoresFromDetail(detail?: BoardScoreItem): BoardScores | undefined {
	if (!detail || detail.scoredNodeCount <= 0) return undefined;
	return {
		security: detail.security,
		privacy: detail.privacy,
		performance: detail.performance,
		governance: detail.governance,
		reliability: detail.reliability,
		cost: detail.cost,
	};
}

function worstScore(scores: BoardScores): number {
	return Math.min(...SCORE_CATEGORIES.map((category) => scores[category]));
}

function formatLogLevel(logLevel: number): string {
	switch (logLevel) {
		case 0:
			return "Debug";
		case 1:
			return "Info";
		case 2:
			return "Warn";
		case 3:
			return "Error";
		case 4:
			return "Fatal";
		default:
			return `Level ${logLevel}`;
	}
}

function BoardScoreOverview({
	board,
	scoreDetail,
	scoresLoading,
}: {
	board: BoardSummary;
	scoreDetail?: BoardScoreItem;
	scoresLoading?: boolean;
}) {
	const scores = board.scores ?? scoresFromDetail(scoreDetail);
	const scoredNodeCount = scoreDetail?.scoredNodeCount;
	const effectiveNodeCount = scoreDetail?.nodeCount ?? board.nodeCount;

	if (!scores) {
		return (
			<div className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
				<div className="flex items-center gap-2">
					<BarChart3 className="h-3.5 w-3.5" />
					<span>
						{scoresLoading
							? "Loading governance scores..."
							: "No governance scores recorded."}
					</span>
				</div>
				{typeof scoredNodeCount === "number" && (
					<p className="mt-1">
						{scoredNodeCount}/{effectiveNodeCount} nodes scored
					</p>
				)}
			</div>
		);
	}

	const worst = scoreDetail?.worstScore ?? worstScore(scores);
	const flagged = scoreDetail?.flaggedPatterns ?? [];

	return (
		<div className="space-y-3 rounded-md border bg-muted/10 p-3">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<div className="flex items-center gap-2">
					<BarChart3 className="h-3.5 w-3.5 text-muted-foreground" />
					<span className="text-xs font-medium">Governance scores</span>
					{typeof scoredNodeCount === "number" && (
						<span className="text-[10px] text-muted-foreground">
							{scoredNodeCount}/{effectiveNodeCount} nodes scored
						</span>
					)}
				</div>
				<div className="flex items-center gap-1.5">
					<span className="text-[10px] text-muted-foreground">Worst</span>
					<span
						className={`inline-flex h-6 min-w-6 items-center justify-center rounded-md px-1.5 text-xs font-semibold tabular-nums text-white ${scoreBgColor(
							worst,
						)}`}
					>
						{worst}
					</span>
				</div>
			</div>

			<div className="grid grid-cols-1 gap-x-6 gap-y-1.5 sm:grid-cols-2">
				{SCORE_CATEGORIES.map((category) => (
					<ScoreBar
						key={category}
						label={SCORE_LABELS[category]}
						value={scores[category]}
					/>
				))}
			</div>

			{flagged.length > 0 && (
				<div className="flex flex-wrap gap-1.5 border-t pt-2">
					{flagged.slice(0, 8).map((pattern, index) => (
						<Badge
							key={`${pattern.node}-${pattern.category}-${index}`}
							variant="outline"
							className="text-[10px]"
						>
							<span className="max-w-36 truncate">{pattern.node}</span>
							<span className="text-muted-foreground">{pattern.category}</span>
							<span className={scoreTextColor(pattern.score)}>
								{pattern.score}
							</span>
							{(pattern.count ?? 1) > 1 && (
								<span className="text-muted-foreground">x{pattern.count}</span>
							)}
						</Badge>
					))}
					{flagged.length > 8 && (
						<Badge variant="secondary" className="text-[10px]">
							+{flagged.length - 8} more
						</Badge>
					)}
				</div>
			)}
		</div>
	);
}

function BoardsSection({
	boards,
	onPreview,
	onPreviewPage,
	boardScoresById,
	boardScoresLoading,
}: {
	boards: BoardSummary[];
	onPreview: (boardId: string) => void;
	onPreviewPage: (page: PageInfo) => void;
	boardScoresById?: Record<string, BoardScoreItem>;
	boardScoresLoading?: boolean;
}) {
	if (boards.length === 0) {
		return (
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<LayoutGrid className="h-4 w-4" />
						Boards (0)
					</CardTitle>
				</CardHeader>
				<CardContent>
					<p className="text-sm text-muted-foreground">No boards found.</p>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<LayoutGrid className="h-4 w-4" />
					Boards ({boards.length})
				</CardTitle>
			</CardHeader>
			<CardContent className="space-y-4">
				{boards.map((board) => (
					<div key={board.id} className="border rounded-lg p-4 space-y-4">
						<div className="flex items-start justify-between gap-4">
							<div className="min-w-0">
								<div className="flex items-center gap-2">
									<h3 className="font-medium text-sm truncate">
										{board.name || board.id}
									</h3>
									<Badge variant="outline" className="text-[10px] px-1.5 py-0">
										v{board.version.join(".")}
									</Badge>
									<Badge
										variant="secondary"
										className="text-[10px] px-1.5 py-0 capitalize"
									>
										{board.stage?.toLowerCase() ?? "unknown"}
									</Badge>
								</div>
								{board.description && (
									<p className="text-xs text-muted-foreground mt-1 line-clamp-2">
										{board.description}
									</p>
								)}
							</div>
							<div className="flex items-center gap-2 shrink-0">
								<Button
									variant="outline"
									size="sm"
									onClick={() => onPreview(board.id)}
									className="text-xs h-7"
								>
									<Eye className="h-3 w-3 mr-1" />
									Preview
								</Button>
								<Badge
									variant="outline"
									className="text-[10px] px-1.5 py-0 capitalize"
								>
									{board.executionMode?.toLowerCase() ?? "auto"}
								</Badge>
							</div>
						</div>

						<div className="grid grid-cols-2 gap-2 text-xs sm:grid-cols-3 lg:grid-cols-6">
							<div className="rounded-md bg-muted/30 px-2 py-1.5">
								<p className="text-[10px] text-muted-foreground">Nodes</p>
								<p className="font-medium tabular-nums">{board.nodeCount}</p>
							</div>
							<div className="rounded-md bg-muted/30 px-2 py-1.5">
								<p className="text-[10px] text-muted-foreground">Connections</p>
								<p className="font-medium tabular-nums">
									{board.connectionCount}
								</p>
							</div>
							<div className="rounded-md bg-muted/30 px-2 py-1.5">
								<p className="text-[10px] text-muted-foreground">Variables</p>
								<p className="font-medium tabular-nums">
									{board.variableCount}
								</p>
							</div>
							<div className="rounded-md bg-muted/30 px-2 py-1.5">
								<p className="text-[10px] text-muted-foreground">Layers</p>
								<p className="font-medium tabular-nums">{board.layerCount}</p>
							</div>
							<div className="rounded-md bg-muted/30 px-2 py-1.5">
								<p className="text-[10px] text-muted-foreground">Comments</p>
								<p className="font-medium tabular-nums">{board.commentCount}</p>
							</div>
							<div className="rounded-md bg-muted/30 px-2 py-1.5">
								<p className="text-[10px] text-muted-foreground">Log level</p>
								<p className="font-medium">{formatLogLevel(board.logLevel)}</p>
							</div>
						</div>

						<BoardScoreOverview
							board={board}
							scoreDetail={boardScoresById?.[board.id]}
							scoresLoading={boardScoresLoading}
						/>

						{board.pages.length > 0 && (
							<div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
								<FileText className="h-3 w-3" />
								{board.pages.length} page{board.pages.length !== 1 ? "s" : ""}:
								{board.pages.map((pg) => (
									<button
										key={pg.pageId}
										type="button"
										onClick={() => onPreviewPage(pg)}
										className="inline-flex h-5 items-center gap-1 rounded-md border px-1.5 text-[10px] font-medium text-foreground transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
									>
										<Eye className="h-2.5 w-2.5" />
										{pg.name}
									</button>
								))}
							</div>
						)}
					</div>
				))}
			</CardContent>
		</Card>
	);
}

function EventsSection({ events }: { events: EventSummary[] }) {
	if (events.length === 0) {
		return (
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<Zap className="h-4 w-4" />
						Events (0)
					</CardTitle>
				</CardHeader>
				<CardContent>
					<p className="text-sm text-muted-foreground">No events found.</p>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<Zap className="h-4 w-4" />
					Events ({events.length})
				</CardTitle>
			</CardHeader>
			<CardContent>
				<div className="divide-y">
					{events.map((event) => (
						<div
							key={event.id}
							className="flex items-center gap-3 py-2.5 first:pt-0 last:pb-0"
						>
							<div
								className={`h-2 w-2 rounded-full shrink-0 ${
									event.active ? "bg-green-500" : "bg-muted-foreground/40"
								}`}
							/>
							<div className="flex-1 min-w-0">
								<div className="flex items-center gap-2">
									<span className="text-sm font-medium truncate">
										{event.name}
									</span>
									{event.isDefault && (
										<Badge
											variant="default"
											className="text-[10px] px-1.5 py-0"
										>
											Default
										</Badge>
									)}
								</div>
								{event.description && (
									<p className="text-xs text-muted-foreground truncate">
										{event.description}
									</p>
								)}
							</div>
							<div className="flex items-center gap-2 shrink-0">
								<Badge variant="outline" className="text-[10px] px-1.5 py-0">
									{event.eventType}
								</Badge>
								{event.route && (
									<Badge
										variant="secondary"
										className="text-[10px] px-1.5 py-0 font-mono"
									>
										{event.route}
									</Badge>
								)}
								<span className="text-[10px] text-muted-foreground">
									v{event.version.join(".")}
								</span>
							</div>
						</div>
					))}
				</div>
			</CardContent>
		</Card>
	);
}

function PagesSection({
	pages,
	onPreview,
}: {
	pages: PageInfo[];
	onPreview: (page: PageInfo) => void;
}) {
	if (pages.length === 0) return null;

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<FileText className="h-4 w-4" />
					Pages ({pages.length})
				</CardTitle>
			</CardHeader>
			<CardContent>
				<div className="divide-y">
					{pages.map((page) => (
						<div
							key={page.pageId}
							className="flex items-center gap-3 py-2 first:pt-0 last:pb-0"
						>
							<FileText className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
							<div className="flex-1 min-w-0">
								<span className="text-sm font-medium">{page.name}</span>
								{page.description && (
									<p className="text-xs text-muted-foreground truncate">
										{page.description}
									</p>
								)}
							</div>
							{page.boardId && (
								<span className="text-[10px] text-muted-foreground font-mono truncate max-w-32">
									{page.boardId}
								</span>
							)}
							<Button
								variant="outline"
								size="sm"
								onClick={() => onPreview(page)}
								className="h-7 shrink-0 text-xs"
							>
								<Eye className="h-3 w-3 mr-1" />
								Preview
							</Button>
						</div>
					))}
				</div>
			</CardContent>
		</Card>
	);
}

function ContentSkeleton() {
	return (
		<div className="space-y-4">
			<Card>
				<CardContent className="p-6">
					<Skeleton className="h-5 w-32 mb-4" />
					<div className="space-y-3">
						<Skeleton className="h-24 w-full" />
						<Skeleton className="h-24 w-full" />
					</div>
				</CardContent>
			</Card>
		</div>
	);
}

function DetailView({
	req,
	onBack,
	onReview,
	isPending,
	content,
	contentLoading,
	boardScoresById,
	boardScoresLoading,
	onPreviewBoard,
	onPreviewPage,
	previewBoard,
	previewBoardLoading,
	previewPage,
	previewPageInfo,
	previewPageLoading,
	onClosePreview,
	approveBlockedReason,
}: {
	req: AppPublicationRequest;
	onBack: () => void;
	onReview: (action: "approve" | "reject" | "hold", message?: string) => void;
	isPending: boolean;
	content?: AppContentResponse;
	contentLoading: boolean;
	boardScoresById?: Record<string, BoardScoreItem>;
	boardScoresLoading?: boolean;
	onPreviewBoard: (boardId: string) => void;
	onPreviewPage: (page: PageInfo) => void;
	previewBoard?: IBoard;
	previewBoardLoading: boolean;
	previewPage?: IPage;
	previewPageInfo?: PageInfo | null;
	previewPageLoading: boolean;
	onClosePreview: () => void;
	approveBlockedReason?: string | null;
}) {
	const [reviewMessage, setReviewMessage] = useState("");
	const isActionable = req.status === "pending" || req.status === "on_hold";

	return (
		<div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
			{/* Back button + title */}
			<div className="flex items-center gap-3">
				<Button variant="ghost" size="sm" onClick={onBack}>
					<ArrowLeft className="h-4 w-4 mr-1" />
					Back
				</Button>
				<Separator orientation="vertical" className="h-6" />
				<h1 className="text-lg font-semibold truncate">
					{req.appName ?? req.appId}
				</h1>
				<Badge variant={statusVariant(req.status)}>
					{formatStatusLabel(req.status)}
				</Badge>
			</div>

			{/* App identity card */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base">App Details</CardTitle>
				</CardHeader>
				<CardContent className="space-y-6">
					<div className="flex items-start gap-6">
						<Avatar className="h-20 w-20 rounded-xl shrink-0">
							<AvatarImage
								src={req.appIcon ?? undefined}
								className="rounded-xl"
							/>
							<AvatarFallback className="rounded-xl text-lg font-semibold bg-primary/10">
								{(req.appName ?? req.appId).substring(0, 2).toUpperCase()}
							</AvatarFallback>
						</Avatar>
						<div className="flex-1 min-w-0 space-y-2">
							<h2 className="text-xl font-semibold">
								{req.appName ?? req.appId}
							</h2>
							{req.appDescription && (
								<p className="text-sm text-muted-foreground leading-relaxed">
									{req.appDescription}
								</p>
							)}
							{req.appTags && req.appTags.length > 0 && (
								<div className="flex flex-wrap gap-1.5">
									{req.appTags.map((tag) => (
										<Badge
											key={tag}
											variant="outline"
											className="text-xs px-2 py-0.5"
										>
											{tag}
										</Badge>
									))}
								</div>
							)}
						</div>
						{req.appThumbnail && (
							<img
								src={req.appThumbnail}
								alt="Thumbnail"
								className="h-24 w-40 rounded-lg object-cover shrink-0 hidden md:block"
							/>
						)}
					</div>
				</CardContent>
			</Card>

			{/* Stats & visibility */}
			<div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
				<Card>
					<CardContent className="p-4 text-center">
						<div className="flex items-center justify-center gap-1 text-muted-foreground mb-1">
							<Download className="h-3.5 w-3.5" />
							<span className="text-xs">Downloads</span>
						</div>
						<p className="text-xl font-bold">
							{formatDownloadCount(req.downloadCount)}
						</p>
					</CardContent>
				</Card>
				<Card>
					<CardContent className="p-4 text-center">
						<div className="flex items-center justify-center gap-1 text-muted-foreground mb-1">
							<Star className="h-3.5 w-3.5" />
							<span className="text-xs">Rating</span>
						</div>
						<p className="text-xl font-bold">
							{(req.ratingCount ?? 0) > 0
								? `${(req.avgRating ?? 0).toFixed(1)} (${req.ratingCount})`
								: "—"}
						</p>
					</CardContent>
				</Card>
				<Card>
					<CardContent className="p-4 text-center">
						<div className="flex items-center justify-center gap-1 text-muted-foreground mb-1">
							<LayoutGrid className="h-3.5 w-3.5" />
							<span className="text-xs">Boards</span>
						</div>
						<p className="text-xl font-bold">{req.boardCount ?? 0}</p>
					</CardContent>
				</Card>
				<Card>
					<CardContent className="p-4 text-center">
						<div className="flex items-center justify-center gap-1 text-muted-foreground mb-1">
							<Package className="h-3.5 w-3.5" />
							<span className="text-xs">Packages</span>
						</div>
						<p className="text-xl font-bold">{req.packageCount ?? 0}</p>
					</CardContent>
				</Card>
			</div>

			{/* Visibility change */}
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Visibility Change</CardTitle>
				</CardHeader>
				<CardContent>
					<div className="flex items-center gap-3 text-sm">
						<Badge variant="outline" className="capitalize text-sm px-3 py-1">
							{req.currentVisibility ?? "unknown"}
						</Badge>
						<ArrowRight className="h-4 w-4 text-muted-foreground" />
						<Badge variant="default" className="capitalize text-sm px-3 py-1">
							{req.targetVisibility}
						</Badge>
					</div>
					<div className="flex items-center gap-4 mt-3 text-xs text-muted-foreground">
						<span>
							Submitted:{" "}
							<RelativeTime
								value={req.createdAt}
								fallback={req.createdAt || "Unknown"}
							/>
						</span>
						{req.updatedAt !== req.createdAt && (
							<span>
								Updated:{" "}
								<RelativeTime
									value={req.updatedAt}
									fallback={req.updatedAt || "Unknown"}
								/>
							</span>
						)}
					</div>
				</CardContent>
			</Card>

			{/* App content: boards, events, pages */}
			{contentLoading ? (
				<ContentSkeleton />
			) : (
				content && (
					<>
						<BoardsSection
							boards={content.boards}
							onPreview={onPreviewBoard}
							onPreviewPage={onPreviewPage}
							boardScoresById={boardScoresById}
							boardScoresLoading={boardScoresLoading}
						/>
						<EventsSection events={content.events} />
						<PagesSection pages={content.pages} onPreview={onPreviewPage} />
					</>
				)
			)}

			{/* EU AI Act conformity assessment (read-only, shown even when the
			    owner has not submitted one yet) */}
			<AdminAiActAssessmentCard appId={req.appId} />

			{/* Review history */}
			{req.logs.length > 0 && (
				<Card>
					<CardHeader>
						<CardTitle className="text-base">Review History</CardTitle>
					</CardHeader>
					<CardContent>
						<ReviewTimeline logs={req.logs} />
					</CardContent>
				</Card>
			)}

			{/* Action bar */}
			{isActionable && (
				<Card>
					<CardHeader>
						<CardTitle className="text-base">Review Decision</CardTitle>
					</CardHeader>
					<CardContent className="space-y-4">
						<Textarea
							placeholder="Add a review message (optional)..."
							className="min-h-[80px] text-sm resize-none"
							value={reviewMessage}
							onChange={(e) => setReviewMessage(e.target.value)}
						/>
						{approveBlockedReason && (
							<div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm text-amber-700 dark:text-amber-400">
								<ShieldAlert className="h-4 w-4 mt-0.5 shrink-0" />
								<span>{approveBlockedReason}</span>
							</div>
						)}
						<div className="flex items-center gap-2 justify-end">
							<Button
								size="sm"
								variant="outline"
								onClick={() => onReview("hold", reviewMessage)}
								disabled={isPending}
							>
								<PauseCircle className="h-3 w-3 mr-1" />
								Hold
							</Button>
							<Button
								size="sm"
								variant="destructive"
								onClick={() => onReview("reject", reviewMessage)}
								disabled={isPending}
							>
								<XCircle className="h-3 w-3 mr-1" />
								Reject
							</Button>
							<Button
								size="sm"
								onClick={() => onReview("approve", reviewMessage)}
								disabled={isPending || !!approveBlockedReason}
								title={approveBlockedReason ?? undefined}
							>
								<CheckCircle className="h-3 w-3 mr-1" />
								Approve
							</Button>
						</div>
					</CardContent>
				</Card>
			)}

			{/* Board/page preview dialog */}
			<Dialog
				open={
					!!previewBoard ||
					previewBoardLoading ||
					!!previewPageInfo ||
					previewPageLoading
				}
				onOpenChange={(open) => {
					if (!open) onClosePreview();
				}}
			>
				<DialogContent
					showCloseButton={false}
					className="!fixed !inset-0 !left-0 !top-0 !h-[100dvh] !max-h-[100dvh] !w-[100vw] !max-w-none !translate-x-0 !translate-y-0 !gap-0 !rounded-none !border-0 !p-0 shadow-none"
				>
					<div className="flex items-center justify-between px-4 py-2 border-b shrink-0">
						<DialogTitle className="text-sm font-semibold">
							{previewPageInfo
								? `Page Preview: ${previewPageInfo.name}`
								: "Board Preview"}
						</DialogTitle>
						<Button
							variant="ghost"
							size="sm"
							onClick={onClosePreview}
							className="h-7 w-7 p-0"
						>
							<X className="h-4 w-4" />
						</Button>
					</div>
					<div className="flex-1 min-h-0">
						{previewPageInfo ? (
							previewPageLoading ? (
								<div className="flex items-center justify-center h-full">
									<div className="text-sm text-muted-foreground">
										Loading page...
									</div>
								</div>
							) : previewPage ? (
								<AdminPagePreview
									page={previewPage}
									appId={previewPageInfo.appId}
								/>
							) : (
								<div className="flex items-center justify-center h-full">
									<div className="text-sm text-muted-foreground">
										Page not found.
									</div>
								</div>
							)
						) : previewBoardLoading ? (
							<div className="flex items-center justify-center h-full">
								<div className="text-sm text-muted-foreground">
									Loading board...
								</div>
							</div>
						) : previewBoard ? (
							<AdminBoardPreview board={previewBoard} />
						) : (
							<div className="flex items-center justify-center h-full">
								<div className="text-sm text-muted-foreground">
									Board not found.
								</div>
							</div>
						)}
					</div>
				</DialogContent>
			</Dialog>
		</div>
	);
}

export interface AdminAppRequestDetailProps {
	requestId: string;
	onBack: () => void;
}

export function AdminAppRequestDetail({
	requestId,
	onBack,
}: AdminAppRequestDetailProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const requestQuery = useQuery<
		RawListResponse,
		Error,
		AppPublicationRequest | null
	>({
		queryKey: ["admin", "publication", "requests", "detail", requestId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RawListResponse>(
				profile.data,
				`admin/publication/requests?id=${encodeURIComponent(requestId)}`,
			);
		},
		enabled: !!profile.data && !!requestId,
		select: (data) => {
			const raw = data.requests[0];
			return raw ? normalizeRequest(raw) : null;
		},
	});

	const appId = requestQuery.data?.appId;

	const contentQuery = useQuery<AppContentResponse>({
		queryKey: ["admin", "publication", "app-content", appId],
		queryFn: async () => {
			if (!profile.data || !appId) throw new Error("Not ready");
			return backend.apiState.get<AppContentResponse>(
				profile.data,
				`admin/publication/apps/${encodeURIComponent(appId)}/content`,
			);
		},
		enabled: !!profile.data && !!appId,
	});

	const governanceScoresQuery = useQuery<AppScoreDetailResponse>({
		queryKey: ["admin", "governance", "scores", appId],
		queryFn: async () => {
			if (!profile.data || !appId) throw new Error("Not ready");
			return backend.apiState.get<AppScoreDetailResponse>(
				profile.data,
				`admin/governance/scores/${encodeURIComponent(appId)}`,
			);
		},
		enabled: !!profile.data && !!appId,
	});

	const boardScoresById = useMemo(
		() =>
			Object.fromEntries(
				(governanceScoresQuery.data?.boards ?? []).map((board) => [
					board.boardId,
					board,
				]),
			) as Record<string, BoardScoreItem>,
		[governanceScoresQuery.data?.boards],
	);

	const [previewBoardId, setPreviewBoardId] = useState<string | null>(null);
	const [previewPageInfo, setPreviewPageInfo] = useState<PageInfo | null>(null);

	const features = useFeatures();
	const aiActEnabled = features.data?.ai_act === true;

	const aiActGate = useQuery<{
		hasAssessment: boolean;
		assessment?: { status?: string } | null;
	}>({
		queryKey: ["admin", "ai-act", "inventory", appId],
		queryFn: async () => {
			if (!profile.data || !appId) throw new Error("Not ready");
			return backend.apiState.get(
				profile.data,
				`admin/ai-act/inventory/${encodeURIComponent(appId)}`,
			);
		},
		enabled: !!profile.data && !!appId && aiActEnabled,
	});

	const approveBlockedReason = ((): string | null => {
		if (!aiActEnabled) return null;
		if (aiActGate.isLoading) return "Loading EU AI Act assessment status…";
		const data = aiActGate.data;
		if (!data || !data.hasAssessment) {
			return "The app owner has not submitted an EU AI Act conformity assessment yet.";
		}
		const status = (data.assessment?.status ?? "").toUpperCase();
		if (status === "BLOCKED") {
			return "This app declares a prohibited AI practice and cannot be approved.";
		}
		if (status === "DRAFT") {
			return "The EU AI Act assessment is still a draft and must be submitted by the owner.";
		}
		return null;
	})();

	const boardQuery = useQuery<IBoard>({
		queryKey: ["admin", "publication", "board", appId, previewBoardId],
		queryFn: async () => {
			if (!profile.data || !appId || !previewBoardId)
				throw new Error("Not ready");
			return backend.apiState.get<IBoard>(
				profile.data,
				`admin/publication/apps/${encodeURIComponent(appId)}/board/${encodeURIComponent(previewBoardId)}`,
			);
		},
		enabled: !!profile.data && !!appId && !!previewBoardId,
	});

	const pageQuery = useQuery<IPage>({
		queryKey: [
			"admin",
			"publication",
			"page",
			appId,
			previewPageInfo?.pageId,
			previewPageInfo?.boardId,
		],
		queryFn: async () => {
			if (!profile.data || !appId || !previewPageInfo?.pageId)
				throw new Error("Not ready");

			const params = new URLSearchParams();
			if (previewPageInfo.boardId) {
				params.set("boardId", previewPageInfo.boardId);
			}

			const page = await backend.apiState.get<IPage>(
				profile.data,
				`admin/publication/apps/${encodeURIComponent(appId)}/page/${encodeURIComponent(previewPageInfo.pageId)}${
					params.size > 0 ? `?${params.toString()}` : ""
				}`,
			);

			let previewPage = { ...page };

			if (page.components && page.components.length > 0) {
				try {
					previewPage = {
						...previewPage,
						components: await presignPageAssets(
							appId,
							page.components,
							backend.storageState,
						),
					};
				} catch (error) {
					console.warn(
						"[AdminAppRequestDetail] Failed to presign page assets",
						{
							pageId: page.id,
							error,
						},
					);
				}
			}

			if (page.canvasSettings?.backgroundImage) {
				try {
					const canvasSettings = await presignCanvasSettings(
						appId,
						{
							backgroundColor: page.canvasSettings.backgroundColor ?? "",
							backgroundImage: page.canvasSettings.backgroundImage,
							padding: page.canvasSettings.padding ?? "",
							customCss: page.canvasSettings.customCss,
						},
						backend.storageState,
					);

					previewPage = {
						...previewPage,
						canvasSettings: {
							...previewPage.canvasSettings,
							backgroundImage: canvasSettings.backgroundImage,
						},
					};
				} catch (error) {
					console.warn(
						"[AdminAppRequestDetail] Failed to presign page background",
						{
							pageId: page.id,
							error,
						},
					);
				}
			}

			return previewPage;
		},
		enabled: !!profile.data && !!appId && !!previewPageInfo?.pageId,
	});

	const handlePreviewBoard = useCallback((boardId: string) => {
		setPreviewPageInfo(null);
		setPreviewBoardId(boardId);
	}, []);

	const handlePreviewPage = useCallback((page: PageInfo) => {
		setPreviewBoardId(null);
		setPreviewPageInfo(page);
	}, []);

	const handleClosePreview = useCallback(() => {
		setPreviewBoardId(null);
		setPreviewPageInfo(null);
	}, []);

	const reviewMutation = useMutation({
		mutationFn: async ({
			action,
			message,
		}: {
			action: "approve" | "reject" | "hold";
			message?: string;
		}) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.patch(
				profile.data,
				`admin/publication/requests/${requestId}`,
				{ action, message },
			);
		},
		onSuccess: () => {
			toast.success("Request updated");
			queryClient.invalidateQueries({
				queryKey: ["admin", "publication", "requests"],
			});
		},
		onError: () => {
			toast.error("Failed to update request");
		},
	});

	const handleReview = useCallback(
		(action: "approve" | "reject" | "hold", message?: string) => {
			reviewMutation.mutate({ action, message });
		},
		[reviewMutation],
	);

	if (requestQuery.isLoading) {
		return (
			<div className="mx-auto w-full max-w-4xl px-4 py-6 sm:px-6 lg:px-8">
				<DetailSkeleton />
			</div>
		);
	}

	if (!requestQuery.data) {
		return (
			<div className="mx-auto flex w-full max-w-4xl flex-col items-center gap-4 px-4 py-12">
				<p className="text-muted-foreground">Request not found.</p>
				<Button variant="outline" onClick={onBack}>
					<ArrowLeft className="h-4 w-4 mr-1" />
					Back
				</Button>
			</div>
		);
	}

	return (
		<DetailView
			req={requestQuery.data}
			onBack={onBack}
			onReview={handleReview}
			isPending={reviewMutation.isPending}
			content={contentQuery.data}
			contentLoading={contentQuery.isLoading}
			boardScoresById={boardScoresById}
			boardScoresLoading={governanceScoresQuery.isLoading}
			onPreviewBoard={handlePreviewBoard}
			onPreviewPage={handlePreviewPage}
			previewBoard={boardQuery.data}
			previewBoardLoading={boardQuery.isLoading && !!previewBoardId}
			previewPage={pageQuery.data}
			previewPageInfo={previewPageInfo}
			previewPageLoading={pageQuery.isLoading && !!previewPageInfo}
			onClosePreview={handleClosePreview}
			approveBlockedReason={approveBlockedReason}
		/>
	);
}
