"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	ArrowRight,
	CheckCircle,
	ChevronLeft,
	ChevronRight,
	CircleAlert,
	Eye,
	EyeOff,
	Layers,
	MessageSquare,
	PauseCircle,
	RefreshCw,
	ShieldAlert,
	ShieldCheck,
	Users,
	XCircle,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useFeatures } from "../../../hooks/use-features";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Textarea,
} from "../../ui";

type RequestStatus = "pending" | "on_hold" | "accepted" | "rejected";
type StatusFilter = "all" | RequestStatus;
type ReviewAction = "approve" | "reject" | "hold";
type AiActState = "clear" | "blocked" | "draft" | "missing";

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

interface SuiteMemberItem {
	appId: string;
	name?: string;
	description?: string;
	icon?: string;
	kind: string;
	currentVisibility?: string;
	aiActStatus?: string;
}

interface SuitePublicationRequest {
	id: string;
	groupId: string;
	ownerAppId: string;
	targetVisibility: string;
	status: RequestStatus;
	approverId?: string;
	createdAt: string;
	updatedAt: string;
	suiteName?: string;
	suiteDescription?: string;
	suiteUseCase?: string;
	suiteIcon?: string;
	suiteBanner?: string;
	suiteTags?: string[];
	currentVisibility?: string;
	members: SuiteMemberItem[];
	logs: PublicationLogItem[];
}

interface SuitePublicationListResponse {
	requests: SuitePublicationRequest[];
	total: number;
	page: number;
	limit: number;
	hasMore: boolean;
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

interface RawSuiteMemberItem {
	appId?: string;
	app_id?: string;
	name?: string;
	description?: string;
	icon?: string;
	kind?: string;
	currentVisibility?: string;
	current_visibility?: string;
	aiActStatus?: string;
	ai_act_status?: string;
}

interface RawSuitePublicationRequest {
	id: string;
	groupId?: string;
	group_id?: string;
	ownerAppId?: string;
	owner_app_id?: string;
	targetVisibility?: string;
	target_visibility?: string;
	status: string;
	approverId?: string;
	approver_id?: string;
	createdAt?: string;
	created_at?: string;
	updatedAt?: string;
	updated_at?: string;
	suiteName?: string;
	suite_name?: string;
	suiteDescription?: string;
	suite_description?: string;
	suiteUseCase?: string;
	suite_use_case?: string;
	suiteIcon?: string;
	suite_icon?: string;
	suiteBanner?: string;
	suite_banner?: string;
	suiteTags?: string[];
	suite_tags?: string[];
	currentVisibility?: string;
	current_visibility?: string;
	members?: RawSuiteMemberItem[];
	logs?: RawPublicationLogItem[];
}

interface RawSuitePublicationListResponse {
	requests: RawSuitePublicationRequest[];
	total: number;
	page: number;
	limit: number;
	hasMore?: boolean;
	has_more?: boolean;
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

const ACTION_RESULT_LABEL: Record<ReviewAction, string> = {
	approve: "approved",
	reject: "rejected",
	hold: "put on hold",
};

const PUBLIC_VISIBILITIES = new Set(["public", "public_request_access"]);

function statusVariant(
	status: string,
): "default" | "secondary" | "destructive" {
	return STATUS_BADGE_VARIANT[status] ?? "secondary";
}

function formatStatusLabel(status: string) {
	return status.replaceAll("_", " ");
}

function normalizeRequestStatus(status: string): RequestStatus {
	switch (status.toLowerCase().replaceAll("_", "")) {
		case "onhold":
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

function normalizeMember(raw: RawSuiteMemberItem): SuiteMemberItem {
	return {
		appId: raw.appId ?? raw.app_id ?? "",
		name: raw.name,
		description: raw.description,
		icon: raw.icon,
		kind: (raw.kind ?? "MEMBER").toUpperCase(),
		currentVisibility: (
			raw.currentVisibility ??
			raw.current_visibility ??
			""
		).toLowerCase(),
		aiActStatus: (raw.aiActStatus ?? raw.ai_act_status ?? "").toUpperCase(),
	};
}

function normalizeRequest(
	raw: RawSuitePublicationRequest,
): SuitePublicationRequest {
	return {
		id: raw.id,
		groupId: raw.groupId ?? raw.group_id ?? "",
		ownerAppId: raw.ownerAppId ?? raw.owner_app_id ?? "",
		targetVisibility: (
			raw.targetVisibility ??
			raw.target_visibility ??
			""
		).toLowerCase(),
		status: normalizeRequestStatus(raw.status),
		approverId: raw.approverId ?? raw.approver_id,
		createdAt: raw.createdAt ?? raw.created_at ?? "",
		updatedAt: raw.updatedAt ?? raw.updated_at ?? "",
		suiteName: raw.suiteName ?? raw.suite_name,
		suiteDescription: raw.suiteDescription ?? raw.suite_description,
		suiteUseCase: raw.suiteUseCase ?? raw.suite_use_case,
		suiteIcon: raw.suiteIcon ?? raw.suite_icon,
		suiteBanner: raw.suiteBanner ?? raw.suite_banner,
		suiteTags: raw.suiteTags ?? raw.suite_tags ?? [],
		currentVisibility: (
			raw.currentVisibility ??
			raw.current_visibility ??
			""
		).toLowerCase(),
		members: (raw.members ?? []).map(normalizeMember),
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

function normalizeResponse(
	response: RawSuitePublicationListResponse,
): SuitePublicationListResponse {
	return {
		requests: (response.requests ?? []).map(normalizeRequest),
		total: response.total,
		page: response.page,
		limit: response.limit,
		hasMore: response.hasMore ?? response.has_more ?? false,
	};
}

function aiActState(status?: string): AiActState {
	switch (status) {
		case "SUBMITTED":
		case "APPROVED":
		case "REJECTED":
			return "clear";
		case "BLOCKED":
			return "blocked";
		case "DRAFT":
			return "draft";
		default:
			return "missing";
	}
}

const AI_ACT_LABEL: Record<AiActState, string> = {
	clear: "AI Act submitted",
	blocked: "AI Act blocked",
	draft: "AI Act draft",
	missing: "No AI Act assessment",
};

function memberLabel(member: SuiteMemberItem) {
	return member.name ?? member.appId;
}

function isPublicVisibility(visibility?: string) {
	return PUBLIC_VISIBILITIES.has(visibility ?? "");
}

function errorMessage(error: unknown) {
	return error instanceof Error ? error.message : String(error);
}

function TableSkeleton({ rows = 5 }: { rows?: number }) {
	return (
		<>
			{Array.from({ length: rows }).map((_, i) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: static skeleton rows
				<TableRow key={`skeleton-${i}`}>
					{Array.from({ length: 6 }).map((_, j) => (
						// biome-ignore lint/suspicious/noArrayIndexKey: static skeleton cells
						<TableCell key={`skeleton-${i}-${j}`}>
							<Skeleton className="h-4 w-full" />
						</TableCell>
					))}
				</TableRow>
			))}
		</>
	);
}

function PaginationControls({
	page,
	totalPages,
	onPageChange,
}: {
	page: number;
	totalPages: number;
	onPageChange: (page: number) => void;
}) {
	if (totalPages <= 1) return null;
	return (
		<div className="flex items-center justify-end gap-2 pt-4">
			<Button
				variant="outline"
				size="sm"
				disabled={page <= 1}
				onClick={() => onPageChange(page - 1)}
			>
				<ChevronLeft className="h-4 w-4" />
			</Button>
			<span className="text-sm text-muted-foreground">
				Page {page} of {totalPages}
			</span>
			<Button
				variant="outline"
				size="sm"
				disabled={page >= totalPages}
				onClick={() => onPageChange(page + 1)}
			>
				<ChevronRight className="h-4 w-4" />
			</Button>
		</div>
	);
}

function SuiteAvatar({
	request,
	className,
}: {
	request: SuitePublicationRequest;
	className: string;
}) {
	return (
		<Avatar className={`${className} shrink-0`}>
			<AvatarImage
				src={request.suiteIcon ?? undefined}
				className="rounded-[inherit]"
			/>
			<AvatarFallback className="rounded-[inherit] font-semibold bg-primary/10">
				{(request.suiteName ?? request.groupId).substring(0, 2).toUpperCase()}
			</AvatarFallback>
		</Avatar>
	);
}

function VisibilityTransition({
	request,
	size,
}: {
	request: SuitePublicationRequest;
	size: "sm" | "lg";
}) {
	const badgeClass =
		size === "lg" ? "capitalize text-sm px-3 py-1" : "capitalize text-[10px]";
	return (
		<div className="flex items-center gap-1.5 text-xs">
			{size === "lg" ? (
				<Badge variant="outline" className={badgeClass}>
					{formatStatusLabel(request.currentVisibility || "unknown")}
				</Badge>
			) : (
				<span className="capitalize">
					{formatStatusLabel(request.currentVisibility || "?")}
				</span>
			)}
			<ArrowRight className="h-3 w-3 text-muted-foreground" />
			<Badge
				variant={size === "lg" ? "default" : "outline"}
				className={badgeClass}
			>
				{formatStatusLabel(request.targetVisibility)}
			</Badge>
		</div>
	);
}

function AiActBadge({ state }: { state: AiActState }) {
	if (state === "clear") {
		return (
			<Badge variant="outline" className="gap-1 text-[10px]">
				<ShieldCheck className="h-3 w-3" />
				{AI_ACT_LABEL.clear}
			</Badge>
		);
	}
	return (
		<Badge
			variant={state === "blocked" ? "destructive" : "secondary"}
			className="gap-1 text-[10px]"
		>
			<ShieldAlert className="h-3 w-3" />
			{AI_ACT_LABEL[state]}
		</Badge>
	);
}

function MemberTile({
	member,
	aiActEnabled,
}: {
	member: SuiteMemberItem;
	aiActEnabled: boolean;
}) {
	const state = aiActState(member.aiActStatus);
	const isAnchor = member.kind === "PRIMARY";

	return (
		<div className="flex items-start gap-3 rounded-lg border p-3">
			<Avatar className="h-10 w-10 rounded-lg shrink-0">
				<AvatarImage src={member.icon ?? undefined} className="rounded-lg" />
				<AvatarFallback className="rounded-lg text-xs font-semibold bg-primary/10">
					{memberLabel(member).substring(0, 2).toUpperCase()}
				</AvatarFallback>
			</Avatar>
			<div className="min-w-0 flex-1 space-y-1.5">
				<div className="flex flex-wrap items-center gap-1.5">
					<span className="text-sm font-medium truncate">
						{memberLabel(member)}
					</span>
					<Badge
						variant={isAnchor ? "default" : "secondary"}
						className="text-[10px] px-1.5 py-0"
					>
						{isAnchor ? "Anchor" : "Member"}
					</Badge>
					<Badge variant="outline" className="text-[10px] px-1.5 py-0">
						{formatStatusLabel(member.currentVisibility || "unknown")}
					</Badge>
				</div>
				{member.description && (
					<p className="text-xs text-muted-foreground line-clamp-2">
						{member.description}
					</p>
				)}
				<div className="flex flex-wrap items-center gap-1.5">
					{aiActEnabled && <AiActBadge state={state} />}
					{!isPublicVisibility(member.currentVisibility) && (
						<span className="inline-flex items-center gap-1 text-[10px] text-muted-foreground">
							<EyeOff className="h-3 w-3" />
							Hidden in the store while not public
						</span>
					)}
				</div>
			</div>
		</div>
	);
}

function MembersSection({
	request,
	aiActEnabled,
}: {
	request: SuitePublicationRequest;
	aiActEnabled: boolean;
}) {
	const hiddenMembers = request.members.filter(
		(member) => !isPublicVisibility(member.currentVisibility),
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<Users className="h-4 w-4" />
					Member Apps ({request.members.length})
				</CardTitle>
				<CardDescription>
					A suite is a visual collection. It grants no runtime permissions —
					every member app keeps its own authority and can leave at any time.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{request.members.length === 0 ? (
					<p className="text-sm text-muted-foreground">
						This suite has no member apps.
					</p>
				) : (
					<div className="grid gap-3 sm:grid-cols-2">
						{request.members.map((member) => (
							<MemberTile
								key={member.appId}
								member={member}
								aiActEnabled={aiActEnabled}
							/>
						))}
					</div>
				)}

				{hiddenMembers.length > 0 && (
					<Alert>
						<EyeOff className="h-4 w-4" />
						<AlertTitle>
							{hiddenMembers.length} member app
							{hiddenMembers.length === 1 ? "" : "s"} will not be listed in the
							store
						</AlertTitle>
						<AlertDescription>
							<p>
								{hiddenMembers.map(memberLabel).join(", ")}{" "}
								{hiddenMembers.length === 1 ? "is" : "are"} not publicly
								visible, so the store hides{" "}
								{hiddenMembers.length === 1 ? "it" : "them"} when the suite is
								browsed. This is informational — it does not block publishing.
							</p>
						</AlertDescription>
					</Alert>
				)}
			</CardContent>
		</Card>
	);
}

function AiActReadinessSection({
	request,
}: {
	request: SuitePublicationRequest;
}) {
	const outstanding = request.members.filter(
		(member) => aiActState(member.aiActStatus) !== "clear",
	);
	const blocked = outstanding.filter(
		(member) => aiActState(member.aiActStatus) === "blocked",
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base flex items-center gap-2">
					<ShieldCheck className="h-4 w-4" />
					EU AI Act Readiness
				</CardTitle>
				<CardDescription>
					A suite carries no assessment of its own. Every active member app
					needs a submitted, non-blocked assessment before the suite can go
					public.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{request.members.length === 0 ? (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertTitle>No member apps</AlertTitle>
						<AlertDescription>
							<p>
								A suite needs at least one member app before it can be
								published.
							</p>
						</AlertDescription>
					</Alert>
				) : outstanding.length === 0 ? (
					<Alert>
						<ShieldCheck className="h-4 w-4" />
						<AlertTitle>Every member app clears the gate</AlertTitle>
						<AlertDescription>
							<p>
								All {request.members.length} member app
								{request.members.length === 1 ? " has" : "s have"} a submitted,
								non-blocked assessment.
							</p>
						</AlertDescription>
					</Alert>
				) : (
					<Alert variant={blocked.length > 0 ? "destructive" : "default"}>
						<ShieldAlert className="h-4 w-4" />
						<AlertTitle>
							{outstanding.length} member app
							{outstanding.length === 1 ? "" : "s"} outstanding
						</AlertTitle>
						<AlertDescription>
							<p>
								The server re-checks this gate on approval and will reject the
								decision while these apps are outstanding.
							</p>
							<ul className="mt-1 space-y-1">
								{outstanding.map((member) => (
									<li
										key={member.appId}
										className="flex flex-wrap items-center gap-2"
									>
										<span className="text-foreground">
											{memberLabel(member)}
										</span>
										<AiActBadge state={aiActState(member.aiActStatus)} />
									</li>
								))}
							</ul>
						</AlertDescription>
					</Alert>
				)}
			</CardContent>
		</Card>
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
									{formatStatusLabel(log.visibility.toLowerCase())}
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

function DecisionSection({
	onReview,
	isPending,
	lastError,
}: {
	onReview: (action: ReviewAction, message?: string) => void;
	isPending: boolean;
	lastError?: string | null;
}) {
	const [reviewMessage, setReviewMessage] = useState("");

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Review Decision</CardTitle>
			</CardHeader>
			<CardContent className="space-y-4">
				<Textarea
					placeholder="Add a review message (optional)..."
					className="min-h-20 text-sm resize-none"
					value={reviewMessage}
					onChange={(e) => setReviewMessage(e.target.value)}
				/>
				{lastError && (
					<Alert variant="destructive">
						<CircleAlert className="h-4 w-4" />
						<AlertTitle>The decision was rejected</AlertTitle>
						<AlertDescription>
							<p>{lastError}</p>
						</AlertDescription>
					</Alert>
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
						disabled={isPending}
					>
						<CheckCircle className="h-3 w-3 mr-1" />
						Approve
					</Button>
				</div>
			</CardContent>
		</Card>
	);
}

function SuiteDetail({
	request,
	aiActEnabled,
	onBack,
	onReview,
	isPending,
	lastError,
}: {
	request: SuitePublicationRequest;
	aiActEnabled: boolean;
	onBack: () => void;
	onReview: (action: ReviewAction, message?: string) => void;
	isPending: boolean;
	lastError?: string | null;
}) {
	const isActionable =
		request.status === "pending" || request.status === "on_hold";

	return (
		<div className="mx-auto flex w-full max-w-4xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
			<div className="flex items-center gap-3">
				<Button variant="ghost" size="sm" onClick={onBack}>
					<ArrowLeft className="h-4 w-4 mr-1" />
					Back
				</Button>
				<Separator orientation="vertical" className="h-6" />
				<h1 className="text-lg font-semibold truncate">
					{request.suiteName ?? request.groupId}
				</h1>
				<Badge variant={statusVariant(request.status)}>
					{formatStatusLabel(request.status)}
				</Badge>
			</div>

			<Card className="overflow-hidden">
				{request.suiteBanner && (
					<img
						src={request.suiteBanner}
						alt="Suite banner"
						className="h-40 w-full object-cover"
					/>
				)}
				<CardHeader>
					<CardTitle className="text-base">Suite Details</CardTitle>
				</CardHeader>
				<CardContent className="space-y-6">
					<div className="flex items-start gap-6">
						<SuiteAvatar request={request} className="h-20 w-20 rounded-xl" />
						<div className="flex-1 min-w-0 space-y-2">
							<h2 className="text-xl font-semibold">
								{request.suiteName ?? request.groupId}
							</h2>
							{request.suiteDescription && (
								<p className="text-sm text-muted-foreground leading-relaxed">
									{request.suiteDescription}
								</p>
							)}
							{request.suiteUseCase && (
								<p className="text-sm">
									<span className="text-muted-foreground">Use case: </span>
									{request.suiteUseCase}
								</p>
							)}
							{(request.suiteTags?.length ?? 0) > 0 && (
								<div className="flex flex-wrap gap-1.5">
									{request.suiteTags?.map((tag) => (
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
					</div>

					<div className="space-y-3">
						<VisibilityTransition request={request} size="lg" />
						<div className="flex flex-wrap items-center gap-4 text-xs text-muted-foreground">
							<span>Anchor app: {request.ownerAppId}</span>
							<span>
								Submitted:{" "}
								<RelativeTime
									value={request.createdAt}
									fallback={request.createdAt || "Unknown"}
								/>
							</span>
							{request.updatedAt !== request.createdAt && (
								<span>
									Updated:{" "}
									<RelativeTime
										value={request.updatedAt}
										fallback={request.updatedAt || "Unknown"}
									/>
								</span>
							)}
						</div>
					</div>
				</CardContent>
			</Card>

			<MembersSection request={request} aiActEnabled={aiActEnabled} />

			{aiActEnabled && <AiActReadinessSection request={request} />}

			{request.logs.length > 0 && (
				<Card>
					<CardHeader>
						<CardTitle className="text-base">Review History</CardTitle>
					</CardHeader>
					<CardContent>
						<ReviewTimeline logs={request.logs} />
					</CardContent>
				</Card>
			)}

			{isActionable && (
				<DecisionSection
					onReview={onReview}
					isPending={isPending}
					lastError={lastError}
				/>
			)}
		</div>
	);
}

export function AdminSuitePublicationsPage() {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const features = useFeatures();
	const aiActEnabled = features.data?.ai_act === true;

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
	const [page, setPage] = useState(1);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [lastError, setLastError] = useState<string | null>(null);
	const limit = 20;

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = { page, limit };
		if (statusFilter !== "all") params.status = statusFilter;
		return params;
	}, [page, statusFilter]);

	const requests = useQuery<
		RawSuitePublicationListResponse,
		Error,
		SuitePublicationListResponse
	>({
		queryKey: ["admin", "publication", "suites", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<RawSuitePublicationListResponse>(
				profile.data,
				`admin/publication/suites?${qs}`,
			);
		},
		enabled: !!profile.data,
		select: normalizeResponse,
	});

	const selected = useMemo(
		() =>
			requests.data?.requests.find((request) => request.id === selectedId) ??
			null,
		[requests.data?.requests, selectedId],
	);

	const totalPages = Math.ceil((requests.data?.total ?? 0) / limit);

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "publication", "suites"],
		});
	}, [queryClient]);

	const handleStatusFilter = useCallback((value: string) => {
		setStatusFilter(value as StatusFilter);
		setPage(1);
	}, []);

	const handleSelect = useCallback((requestId: string | null) => {
		setLastError(null);
		setSelectedId(requestId);
	}, []);

	const reviewMutation = useMutation({
		mutationFn: async ({
			requestId,
			action,
			message,
		}: {
			requestId: string;
			action: ReviewAction;
			message?: string;
		}) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.patch(
				profile.data,
				`admin/publication/requests/${requestId}`,
				{ action, message: message?.trim() ? message : undefined },
			);
		},
	});

	const handleReview = useCallback(
		async (action: ReviewAction, message?: string) => {
			if (!selectedId) return;
			setLastError(null);
			try {
				await reviewMutation.mutateAsync({
					requestId: selectedId,
					action,
					message,
				});
				await queryClient.invalidateQueries({
					queryKey: ["admin", "publication"],
				});
				const refreshed = await requests.refetch();
				const updated = refreshed.data?.requests.find(
					(request) => request.id === selectedId,
				);
				toast.success(
					`Suite ${ACTION_RESULT_LABEL[action]}${
						updated ? ` — now ${formatStatusLabel(updated.status)}` : ""
					}`,
				);
			} catch (error) {
				const message = errorMessage(error);
				setLastError(message);
				toast.error("Failed to update suite request", {
					description: message,
				});
			}
		},
		[queryClient, requests, reviewMutation, selectedId],
	);

	if (selected) {
		return (
			<SuiteDetail
				request={selected}
				aiActEnabled={aiActEnabled}
				onBack={() => handleSelect(null)}
				onReview={handleReview}
				isPending={reviewMutation.isPending}
				lastError={lastError}
			/>
		);
	}

	return (
		<div className="mx-auto flex w-full max-w-7xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
			<Card className="border-border/60 shadow-sm">
				<CardHeader className="gap-4 sm:flex-row sm:items-end sm:justify-between">
					<div className="space-y-2">
						<CardTitle className="text-3xl">
							Suite Publication Requests
						</CardTitle>
						<CardDescription className="max-w-3xl text-sm leading-6">
							Suites bundle existing apps into one store listing. They grant no
							runtime permissions, so the review covers the suite's branding,
							its member apps, and whether every active member clears the EU AI
							Act gate.
						</CardDescription>
					</div>
					<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
						<Select value={statusFilter} onValueChange={handleStatusFilter}>
							<SelectTrigger className="w-full sm:w-40">
								<SelectValue placeholder="Suite status" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="all">All statuses</SelectItem>
								<SelectItem value="pending">Pending</SelectItem>
								<SelectItem value="on_hold">On hold</SelectItem>
								<SelectItem value="accepted">Accepted</SelectItem>
								<SelectItem value="rejected">Rejected</SelectItem>
							</SelectContent>
						</Select>
						<Button variant="outline" size="sm" onClick={handleRefresh}>
							<RefreshCw className="mr-2 h-4 w-4" />
							Refresh
						</Button>
					</div>
				</CardHeader>
			</Card>

			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>Suite</TableHead>
								<TableHead>Use case</TableHead>
								<TableHead>Visibility</TableHead>
								<TableHead>Status</TableHead>
								<TableHead>Members</TableHead>
								<TableHead>Submitted</TableHead>
								<TableHead className="text-right">Actions</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{requests.isLoading && <TableSkeleton />}
							{!requests.isLoading &&
								(!requests.data?.requests?.length ? (
									<TableRow>
										<TableCell
											colSpan={7}
											className="text-center text-muted-foreground py-8"
										>
											No suite publication requests found.
										</TableCell>
									</TableRow>
								) : (
									requests.data.requests.map((request) => {
										const outstanding = request.members.filter(
											(member) => aiActState(member.aiActStatus) !== "clear",
										).length;
										return (
											<TableRow key={request.id}>
												<TableCell>
													<div className="flex items-center gap-3">
														<SuiteAvatar
															request={request}
															className="h-8 w-8 rounded-lg"
														/>
														<div className="min-w-0">
															<p className="font-medium text-sm truncate">
																{request.suiteName ?? request.groupId}
															</p>
															{request.suiteDescription && (
																<p className="text-xs text-muted-foreground truncate max-w-xs">
																	{request.suiteDescription}
																</p>
															)}
														</div>
													</div>
												</TableCell>
												<TableCell className="text-xs text-muted-foreground max-w-48 truncate">
													{request.suiteUseCase ?? "—"}
												</TableCell>
												<TableCell>
													<VisibilityTransition request={request} size="sm" />
												</TableCell>
												<TableCell>
													<Badge variant={statusVariant(request.status)}>
														{formatStatusLabel(request.status)}
													</Badge>
												</TableCell>
												<TableCell>
													<div className="flex items-center gap-2 text-xs text-muted-foreground">
														<span className="flex items-center gap-1">
															<Layers className="h-3 w-3" />
															{request.members.length}
														</span>
														{aiActEnabled && outstanding > 0 && (
															<span className="flex items-center gap-1 text-destructive">
																<ShieldAlert className="h-3 w-3" />
																{outstanding}
															</span>
														)}
													</div>
												</TableCell>
												<TableCell className="text-xs text-muted-foreground">
													<RelativeTime
														value={request.createdAt}
														fallback={request.createdAt || "Unknown"}
													/>
												</TableCell>
												<TableCell className="text-right">
													<Button
														size="sm"
														variant="outline"
														onClick={() => handleSelect(request.id)}
													>
														<Eye className="h-3 w-3 mr-1" />
														Review
													</Button>
												</TableCell>
											</TableRow>
										);
									})
								))}
						</TableBody>
					</Table>
				</CardContent>
			</Card>

			<PaginationControls
				page={page}
				totalPages={totalPages}
				onPageChange={setPage}
			/>
		</div>
	);
}
