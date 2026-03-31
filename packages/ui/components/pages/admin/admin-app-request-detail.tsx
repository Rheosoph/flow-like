"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	ArrowRight,
	CheckCircle,
	Download,
	Eye,
	FileText,
	LayoutGrid,
	MessageSquare,
	Package,
	PauseCircle,
	Star,
	X,
	XCircle,
	Zap,
} from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import type { IBoard } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
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

type RequestStatus = "pending" | "on_hold" | "accepted" | "rejected";

interface BoardScores {
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
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

function BoardsSection({
	boards,
	onPreview,
}: { boards: BoardSummary[]; onPreview: (boardId: string) => void }) {
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
					<div key={board.id} className="border rounded-lg p-4 space-y-3">
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

						<div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
							<span>{board.nodeCount} nodes</span>
							<span>{board.connectionCount} connections</span>
							<span>{board.variableCount} variables</span>
							<span>{board.layerCount} layers</span>
							<span>{board.commentCount} comments</span>
						</div>

						{board.scores && (
							<div className="grid grid-cols-2 gap-x-6 gap-y-1">
								<ScoreBar label="Security" value={board.scores.security} />
								<ScoreBar label="Privacy" value={board.scores.privacy} />
								<ScoreBar
									label="Performance"
									value={board.scores.performance}
								/>
								<ScoreBar label="Governance" value={board.scores.governance} />
								<ScoreBar
									label="Reliability"
									value={board.scores.reliability}
								/>
								<ScoreBar label="Cost" value={board.scores.cost} />
							</div>
						)}

						{board.pages.length > 0 && (
							<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
								<FileText className="h-3 w-3" />
								{board.pages.length} page{board.pages.length !== 1 ? "s" : ""}:
								{board.pages.map((pg) => (
									<Badge
										key={pg.pageId}
										variant="outline"
										className="text-[10px] px-1.5 py-0"
									>
										{pg.name}
									</Badge>
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

function PagesSection({ pages }: { pages: PageInfo[] }) {
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
	onPreviewBoard,
	previewBoard,
	previewBoardLoading,
	onClosePreview,
}: {
	req: AppPublicationRequest;
	onBack: () => void;
	onReview: (action: "approve" | "reject" | "hold", message?: string) => void;
	isPending: boolean;
	content?: AppContentResponse;
	contentLoading: boolean;
	onPreviewBoard: (boardId: string) => void;
	previewBoard?: IBoard;
	previewBoardLoading: boolean;
	onClosePreview: () => void;
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
						<BoardsSection boards={content.boards} onPreview={onPreviewBoard} />
						<EventsSection events={content.events} />
						<PagesSection pages={content.pages} />
					</>
				)
			)}

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
								disabled={isPending}
							>
								<CheckCircle className="h-3 w-3 mr-1" />
								Approve
							</Button>
						</div>
					</CardContent>
				</Card>
			)}

			{/* Board preview dialog */}
			<Dialog
				open={!!previewBoard || previewBoardLoading}
				onOpenChange={(open) => {
					if (!open) onClosePreview();
				}}
			>
				<DialogContent className="max-w-[100vw] w-[100vw] h-[100vh] max-h-[100vh] p-0 flex flex-col rounded-none border-none">
					<div className="flex items-center justify-between px-4 py-2 border-b shrink-0">
						<DialogTitle className="text-sm font-semibold">
							Board Preview
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
						{previewBoardLoading ? (
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

	const [previewBoardId, setPreviewBoardId] = useState<string | null>(null);

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

	const handlePreviewBoard = useCallback((boardId: string) => {
		setPreviewBoardId(boardId);
	}, []);

	const handleClosePreview = useCallback(() => {
		setPreviewBoardId(null);
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
			onPreviewBoard={handlePreviewBoard}
			previewBoard={boardQuery.data}
			previewBoardLoading={boardQuery.isLoading && !!previewBoardId}
			onClosePreview={handleClosePreview}
		/>
	);
}
