"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { formatDistanceToNow } from "date-fns";
import {
	CheckCircle,
	ChevronLeft,
	ChevronRight,
	Clock,
	Download,
	Eye,
	Loader2,
	Package,
	RefreshCw,
	XCircle,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import type { AdminPackageListResponse } from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
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

type RequestStatus = "pending" | "accepted" | "rejected";

interface AppPublicationRequest {
	id: string;
	appId: string;
	targetVisibility: string;
	status: RequestStatus;
	approverId?: string;
	createdAt: string;
	updatedAt: string;
}

interface AppPublicationListResponse {
	requests: AppPublicationRequest[];
	total: number;
	page: number;
	limit: number;
	hasMore: boolean;
}

export interface AdminPublicationsPageProps {
	onNavigateToPackage?: (packageId: string) => void;
}

const STATUS_BADGE_VARIANT: Record<
	string,
	"default" | "secondary" | "destructive"
> = {
	pending: "secondary",
	pending_review: "secondary",
	accepted: "default",
	active: "default",
	rejected: "destructive",
};

function statusVariant(
	status: string,
): "default" | "secondary" | "destructive" {
	return STATUS_BADGE_VARIANT[status] ?? "secondary";
}

function TableSkeleton({ rows = 5 }: { rows?: number }) {
	return (
		<>
			{Array.from({ length: rows }).map((_, i) => (
				<TableRow key={`skeleton-${i}`}>
					{Array.from({ length: 5 }).map((_, j) => (
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
}: {
	statusFilter: string;
}) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [page, setPage] = useState(1);
	const [reviewMessage, setReviewMessage] = useState<Record<string, string>>(
		{},
	);
	const limit = 20;

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = { page, limit };
		if (statusFilter !== "all") params.status = statusFilter;
		return params;
	}, [page, limit, statusFilter]);

	const requests = useQuery<AppPublicationListResponse>({
		queryKey: ["admin", "publication", "requests", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<AppPublicationListResponse>(
				profile.data,
				`admin/publication/requests?${qs}`,
			);
		},
		enabled: !!profile.data,
	});

	const reviewMutation = useMutation({
		mutationFn: async ({
			id,
			action,
			message,
		}: {
			id: string;
			action: "approve" | "reject" | "hold";
			message?: string;
		}) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.patch(
				profile.data,
				`admin/publication/requests/${id}`,
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
		(id: string, action: "approve" | "reject" | "hold") => {
			reviewMutation.mutate({ id, action, message: reviewMessage[id] });
		},
		[reviewMutation, reviewMessage],
	);

	const totalPages = Math.ceil((requests.data?.total ?? 0) / limit);

	return (
		<div className="space-y-4">
			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>App ID</TableHead>
								<TableHead>Target Visibility</TableHead>
								<TableHead>Status</TableHead>
								<TableHead>Created</TableHead>
								<TableHead className="text-right">Actions</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{requests.isLoading && <TableSkeleton />}
							{!requests.isLoading &&
								(!requests.data?.requests?.length ? (
									<TableRow>
										<TableCell
											colSpan={5}
											className="text-center text-muted-foreground py-8"
										>
											No publication requests found.
										</TableCell>
									</TableRow>
								) : (
									requests.data.requests.map((req) => (
										<TableRow key={req.id}>
											<TableCell className="font-medium">{req.appId}</TableCell>
											<TableCell>
												<Badge variant="outline">{req.targetVisibility}</Badge>
											</TableCell>
											<TableCell>
												<Badge variant={statusVariant(req.status)}>
													{req.status}
												</Badge>
											</TableCell>
											<TableCell className="text-sm text-muted-foreground">
												{formatDistanceToNow(new Date(req.createdAt), {
													addSuffix: true,
												})}
											</TableCell>
											<TableCell className="text-right">
												{req.status === "pending" ? (
													<div className="flex items-center justify-end gap-2">
														<Input
															placeholder="Message (optional)"
															className="max-w-[180px] h-8 text-xs"
															value={reviewMessage[req.id] ?? ""}
															onChange={(e) =>
																setReviewMessage((prev) => ({
																	...prev,
																	[req.id]: e.target.value,
																}))
															}
														/>
														<Button
															size="sm"
															variant="outline"
															onClick={() => handleReview(req.id, "approve")}
															disabled={reviewMutation.isPending}
														>
															<CheckCircle className="h-3 w-3 mr-1" />
															Approve
														</Button>
														<Button
															size="sm"
															variant="destructive"
															onClick={() => handleReview(req.id, "reject")}
															disabled={reviewMutation.isPending}
														>
															<XCircle className="h-3 w-3 mr-1" />
															Reject
														</Button>
													</div>
												) : (
													<span className="text-xs text-muted-foreground">
														Reviewed
													</span>
												)}
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
	statusFilter: string;
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

	const status =
		statusFilter === "all" ? "pending_review" : statusFilter;

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			offset: (page - 1) * limit,
			limit,
			status,
		};
		return params;
	}, [page, limit, status]);

	const packages = useQuery<AdminPackageListResponse>({
		queryKey: ["admin", "packages", "publications", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<AdminPackageListResponse>(
				profile.data,
				`admin/packages?${qs}`,
			);
		},
		enabled: !!profile.data,
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
													{pkg.status.replace("_", " ")}
												</Badge>
											</TableCell>
											<TableCell>
												<span className="flex items-center gap-1">
													<Download className="h-3 w-3" />
													{pkg.downloadCount.toLocaleString()}
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
}: AdminPublicationsPageProps) {
	const [statusFilter, setStatusFilter] = useState("all");
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
		<div className="container mx-auto py-6 space-y-6">
			<div className="flex items-center justify-between">
				<div>
					<h1 className="text-3xl font-bold">Publication Requests</h1>
					<p className="text-muted-foreground">
						Review and manage app and package publication requests
					</p>
				</div>
				<div className="flex items-center gap-3">
					<Select value={statusFilter} onValueChange={setStatusFilter}>
						<SelectTrigger className="w-36">
							<SelectValue placeholder="Status" />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="all">All</SelectItem>
							<SelectItem value="pending">Pending</SelectItem>
							<SelectItem value="accepted">Accepted</SelectItem>
							<SelectItem value="rejected">Rejected</SelectItem>
						</SelectContent>
					</Select>
					<Button variant="outline" size="sm" onClick={handleRefresh}>
						<RefreshCw className="h-4 w-4 mr-2" />
						Refresh
					</Button>
				</div>
			</div>

			<Tabs defaultValue="apps">
				<TabsList>
					<TabsTrigger value="apps">App Requests</TabsTrigger>
					<TabsTrigger value="packages">Package Requests</TabsTrigger>
				</TabsList>

				<TabsContent value="apps" className="mt-4">
					<AppRequestsTab statusFilter={statusFilter} />
				</TabsContent>

				<TabsContent value="packages" className="mt-4">
					<PackageRequestsTab
						statusFilter={statusFilter}
						onNavigateToPackage={onNavigateToPackage}
					/>
				</TabsContent>
			</Tabs>
		</div>
	);
}
