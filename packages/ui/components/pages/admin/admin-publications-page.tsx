"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowRight,
	ChevronLeft,
	ChevronRight,
	Download,
	Eye,
	LayoutGrid,
	Package,
	RefreshCw,
	Star,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
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
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../ui";

type RequestStatus = "pending" | "on_hold" | "accepted" | "rejected";

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

interface AppPublicationListResponse {
	requests: AppPublicationRequest[];
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

interface RawAppPublicationListResponse {
	requests: RawAppPublicationRequest[];
	total: number;
	page: number;
	limit: number;
	hasMore?: boolean;
	has_more?: boolean;
}

interface PackageRequestItem {
	id: string;
	name: string;
	version: string;
	status: string;
	downloadCount: number;
}

interface PackageRequestListResponse {
	packages: PackageRequestItem[];
	totalCount: number;
	offset: number;
	limit: number;
}

interface RawPackageRequestItem {
	id: string;
	name: string;
	version: string;
	status: string;
	downloadCount?: number;
	download_count?: number;
}

interface RawPackageRequestListResponse {
	packages: RawPackageRequestItem[];
	totalCount?: number;
	total_count?: number;
	offset: number;
	limit: number;
}

export interface AdminPublicationsPageProps {
	onNavigateToPackage?: (packageId: string) => void;
	onSelectRequest?: (requestId: string) => void;
}

type AppStatusFilter = "all" | RequestStatus;
type PackageStatusFilter =
	| "all"
	| "pending_review"
	| "active"
	| "rejected"
	| "deprecated"
	| "disabled";

const STATUS_BADGE_VARIANT: Record<
	string,
	"default" | "secondary" | "destructive"
> = {
	pending: "secondary",
	on_hold: "secondary",
	pending_review: "secondary",
	accepted: "default",
	active: "default",
	rejected: "destructive",
};

function formatDownloadCount(count: number | null | undefined) {
	return (count ?? 0).toLocaleString();
}

function statusVariant(
	status: string,
): "default" | "secondary" | "destructive" {
	return STATUS_BADGE_VARIANT[status] ?? "secondary";
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

function formatStatusLabel(status: string) {
	return status.replaceAll("_", " ");
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

function normalizeAppPublicationResponse(
	response: RawAppPublicationListResponse,
): AppPublicationListResponse {
	return {
		requests: response.requests.map((request) => ({
			id: request.id,
			appId: request.appId ?? request.app_id ?? "",
			targetVisibility: (
				request.targetVisibility ??
				request.target_visibility ??
				""
			).toLowerCase(),
			status: normalizeRequestStatus(request.status),
			approverId: request.approverId ?? request.approver_id,
			createdAt: request.createdAt ?? request.created_at ?? "",
			updatedAt: request.updatedAt ?? request.updated_at ?? "",
			appName: request.appName ?? request.app_name,
			appDescription: request.appDescription ?? request.app_description,
			appIcon: request.appIcon ?? request.app_icon,
			appThumbnail: request.appThumbnail ?? request.app_thumbnail,
			appTags: request.appTags ?? request.app_tags,
			currentVisibility: (
				request.currentVisibility ??
				request.current_visibility ??
				""
			).toLowerCase(),
			downloadCount: request.downloadCount ?? request.download_count ?? 0,
			ratingCount: request.ratingCount ?? request.rating_count ?? 0,
			avgRating: request.avgRating ?? request.avg_rating,
			boardCount: request.boardCount ?? request.board_count ?? 0,
			packageCount: request.packageCount ?? request.package_count ?? 0,
			logs: (request.logs ?? []).map((log) => ({
				id: log.id,
				authorId: log.authorId ?? log.author_id,
				author: normalizeActor(log.author),
				message: log.message,
				visibility: log.visibility,
				createdAt: log.createdAt ?? log.created_at ?? "",
			})),
		})),
		total: response.total,
		page: response.page,
		limit: response.limit,
		hasMore: response.hasMore ?? response.has_more ?? false,
	};
}

function normalizePackageRequestResponse(
	response: RawPackageRequestListResponse,
): PackageRequestListResponse {
	return {
		packages: response.packages.map((pkg) => ({
			id: pkg.id,
			name: pkg.name,
			version: pkg.version,
			status: pkg.status.toLowerCase(),
			downloadCount: pkg.downloadCount ?? pkg.download_count ?? 0,
		})),
		totalCount: response.totalCount ?? response.total_count ?? 0,
		offset: response.offset,
		limit: response.limit,
	};
}

function TableSkeleton({ rows = 5 }: { rows?: number }) {
	return (
		<>
			{Array.from({ length: rows }).map((_, i) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: static skeleton rows
				<TableRow key={`skeleton-${i}`}>
					{Array.from({ length: 5 }).map((_, j) => (
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

function AppRequestsTab({
	statusFilter,
	onSelectRequest,
}: {
	statusFilter: AppStatusFilter;
	onSelectRequest?: (requestId: string) => void;
}) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [page, setPage] = useState(1);
	const limit = 20;

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = { page, limit };
		if (statusFilter !== "all") params.status = statusFilter;
		return params;
	}, [page, statusFilter]);

	const requests = useQuery<
		RawAppPublicationListResponse,
		Error,
		AppPublicationListResponse
	>({
		queryKey: ["admin", "publication", "requests", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<RawAppPublicationListResponse>(
				profile.data,
				`admin/publication/requests?${qs}`,
			);
		},
		enabled: !!profile.data,
		select: normalizeAppPublicationResponse,
	});

	const totalPages = Math.ceil((requests.data?.total ?? 0) / limit);

	return (
		<div className="space-y-4">
			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>App</TableHead>
								<TableHead>Visibility</TableHead>
								<TableHead>Status</TableHead>
								<TableHead>Stats</TableHead>
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
											colSpan={6}
											className="text-center text-muted-foreground py-8"
										>
											No publication requests found.
										</TableCell>
									</TableRow>
								) : (
									requests.data.requests.map((req) => (
										<TableRow key={req.id}>
											<TableCell>
												<div className="flex items-center gap-3">
													<Avatar className="h-8 w-8 rounded-lg shrink-0">
														<AvatarImage
															src={req.appIcon ?? undefined}
															className="rounded-lg"
														/>
														<AvatarFallback className="rounded-lg text-xs font-semibold bg-primary/10">
															{(req.appName ?? req.appId)
																.substring(0, 2)
																.toUpperCase()}
														</AvatarFallback>
													</Avatar>
													<div className="min-w-0">
														<p className="font-medium text-sm truncate">
															{req.appName ?? req.appId}
														</p>
														{req.appDescription && (
															<p className="text-xs text-muted-foreground truncate max-w-xs">
																{req.appDescription}
															</p>
														)}
													</div>
												</div>
											</TableCell>
											<TableCell>
												<div className="flex items-center gap-1.5 text-xs">
													<span className="capitalize">
														{req.currentVisibility ?? "?"}
													</span>
													<ArrowRight className="h-3 w-3 text-muted-foreground" />
													<Badge
														variant="outline"
														className="capitalize text-[10px]"
													>
														{req.targetVisibility}
													</Badge>
												</div>
											</TableCell>
											<TableCell>
												<Badge variant={statusVariant(req.status)}>
													{formatStatusLabel(req.status)}
												</Badge>
											</TableCell>
											<TableCell>
												<div className="flex items-center gap-3 text-xs text-muted-foreground">
													<span className="flex items-center gap-1">
														<Download className="h-3 w-3" />
														{formatDownloadCount(req.downloadCount)}
													</span>
													{(req.ratingCount ?? 0) > 0 && (
														<span className="flex items-center gap-1">
															<Star className="h-3 w-3" />
															{(req.avgRating ?? 0).toFixed(1)}
														</span>
													)}
													<span className="flex items-center gap-1">
														<LayoutGrid className="h-3 w-3" />
														{req.boardCount ?? 0}
													</span>
													<span className="flex items-center gap-1">
														<Package className="h-3 w-3" />
														{req.packageCount ?? 0}
													</span>
												</div>
											</TableCell>
											<TableCell className="text-xs text-muted-foreground">
												<RelativeTime
													value={req.createdAt}
													fallback={req.createdAt || "Unknown"}
												/>
											</TableCell>
											<TableCell className="text-right">
												<Button
													size="sm"
													variant="outline"
													onClick={() => onSelectRequest?.(req.id)}
												>
													<Eye className="h-3 w-3 mr-1" />
													Review
												</Button>
											</TableCell>
										</TableRow>
									))
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

function PackageRequestsTab({
	statusFilter,
	onNavigateToPackage,
}: {
	statusFilter: PackageStatusFilter;
	onNavigateToPackage?: (packageId: string) => void;
}) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [page, setPage] = useState(1);
	const limit = 20;

	const status = statusFilter === "all" ? "pending_review" : statusFilter;

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			offset: (page - 1) * limit,
			limit,
			status,
		};
		return params;
	}, [page, status]);

	const packages = useQuery<
		RawPackageRequestListResponse,
		Error,
		PackageRequestListResponse
	>({
		queryKey: ["admin", "packages", "publications", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<RawPackageRequestListResponse>(
				profile.data,
				`admin/packages?${qs}`,
			);
		},
		enabled: !!profile.data,
		select: normalizePackageRequestResponse,
	});

	const totalPages = Math.ceil((packages.data?.totalCount ?? 0) / limit);

	return (
		<div className="space-y-4">
			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>Name</TableHead>
								<TableHead>Version</TableHead>
								<TableHead>Status</TableHead>
								<TableHead>Downloads</TableHead>
								<TableHead className="text-right">Actions</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{packages.isLoading && <TableSkeleton />}
							{!packages.isLoading &&
								(!packages.data?.packages?.length ? (
									<TableRow>
										<TableCell
											colSpan={5}
											className="text-center text-muted-foreground py-8"
										>
											No packages pending review.
										</TableCell>
									</TableRow>
								) : (
									packages.data.packages.map((pkg) => (
										<TableRow key={pkg.id}>
											<TableCell className="font-medium">
												<div className="flex items-center gap-2">
													<Package className="h-4 w-4 shrink-0" />
													{pkg.name}
												</div>
											</TableCell>
											<TableCell>
												<Badge variant="outline">{pkg.version}</Badge>
											</TableCell>
											<TableCell>
												<Badge variant={statusVariant(pkg.status)}>
													{formatStatusLabel(pkg.status)}
												</Badge>
											</TableCell>
											<TableCell>
												<span className="flex items-center gap-1">
													<Download className="h-3 w-3" />
													{formatDownloadCount(pkg.downloadCount)}
												</span>
											</TableCell>
											<TableCell className="text-right">
												<Button
													size="sm"
													variant="outline"
													onClick={() => onNavigateToPackage?.(pkg.id)}
												>
													<Eye className="h-3 w-3 mr-1" />
													View
												</Button>
											</TableCell>
										</TableRow>
									))
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

export function AdminPublicationsPage({
	onNavigateToPackage,
	onSelectRequest,
}: AdminPublicationsPageProps) {
	const [activeTab, setActiveTab] = useState<"apps" | "packages">("apps");
	const [appStatusFilter, setAppStatusFilter] =
		useState<AppStatusFilter>("all");
	const [packageStatusFilter, setPackageStatusFilter] =
		useState<PackageStatusFilter>("all");
	const queryClient = useQueryClient();

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "publication"],
		});
		queryClient.invalidateQueries({
			queryKey: ["admin", "packages", "publications"],
		});
	}, [queryClient]);

	return (
		<div className="mx-auto flex w-full max-w-7xl flex-col gap-6 px-4 py-6 sm:px-6 lg:px-8">
			<Card className="border-border/60 shadow-sm">
				<CardHeader className="gap-4 sm:flex-row sm:items-end sm:justify-between">
					<div className="space-y-2">
						<CardTitle className="text-3xl">Publication Requests</CardTitle>
						<CardDescription className="max-w-3xl text-sm leading-6">
							Review app publication requests and package submissions from one
							place. Package requests use the package review queue and app
							requests use the publication request workflow.
						</CardDescription>
					</div>
					<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
						{activeTab === "apps" ? (
							<Select
								value={appStatusFilter}
								onValueChange={(value) =>
									setAppStatusFilter(value as AppStatusFilter)
								}
							>
								<SelectTrigger className="w-full sm:w-40">
									<SelectValue placeholder="App status" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="all">All app statuses</SelectItem>
									<SelectItem value="pending">Pending</SelectItem>
									<SelectItem value="on_hold">On hold</SelectItem>
									<SelectItem value="accepted">Accepted</SelectItem>
									<SelectItem value="rejected">Rejected</SelectItem>
								</SelectContent>
							</Select>
						) : (
							<Select
								value={packageStatusFilter}
								onValueChange={(value) =>
									setPackageStatusFilter(value as PackageStatusFilter)
								}
							>
								<SelectTrigger className="w-full sm:w-48">
									<SelectValue placeholder="Package status" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="all">Pending review</SelectItem>
									<SelectItem value="pending_review">Pending review</SelectItem>
									<SelectItem value="active">Approved</SelectItem>
									<SelectItem value="rejected">Rejected</SelectItem>
									<SelectItem value="deprecated">Deprecated</SelectItem>
									<SelectItem value="disabled">Disabled</SelectItem>
								</SelectContent>
							</Select>
						)}
						<Button variant="outline" size="sm" onClick={handleRefresh}>
							<RefreshCw className="mr-2 h-4 w-4" />
							Refresh
						</Button>
					</div>
				</CardHeader>
			</Card>

			<Tabs
				value={activeTab}
				onValueChange={(value) => setActiveTab(value as "apps" | "packages")}
			>
				<TabsList className="w-full justify-start gap-2 rounded-xl border border-border/60 bg-background p-1">
					<TabsTrigger value="apps">App Requests</TabsTrigger>
					<TabsTrigger value="packages">Package Requests</TabsTrigger>
				</TabsList>

				<TabsContent value="apps" className="mt-4">
					<AppRequestsTab
						statusFilter={appStatusFilter}
						onSelectRequest={onSelectRequest}
					/>
				</TabsContent>

				<TabsContent value="packages" className="mt-4">
					<PackageRequestsTab
						statusFilter={packageStatusFilter}
						onNavigateToPackage={onNavigateToPackage}
					/>
				</TabsContent>
			</Tabs>
		</div>
	);
}
