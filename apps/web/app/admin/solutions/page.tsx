"use client";

import {
	Card,
	CardContent,
	CardHeader,
	CardTitle,
	type ISolutionListResponse,
	type ISolutionLogPayload,
	type ISolutionRequest,
	type ISolutionUpdatePayload,
	Skeleton,
	type SolutionStatus,
	SolutionsPage,
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useDebounce } from "@uidotdev/usehooks";
import { AlertCircle, CheckCircle, Clock, Inbox } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

export default function AdminSolutionsPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [page, setPage] = useState(1);
	const [limit, setLimit] = useState(25);
	const [statusFilter, setStatusFilter] = useState<SolutionStatus | undefined>(
		undefined,
	);
	const [searchQuery, setSearchQuery] = useState("");
	const debouncedSearch = useDebounce(searchQuery, 300);

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			page,
			limit,
		};
		if (statusFilter) params.status = statusFilter;
		if (debouncedSearch) params.search = debouncedSearch;
		return params;
	}, [page, limit, statusFilter, debouncedSearch]);

	const solutions = useQuery<ISolutionListResponse, Error>({
		queryKey: ["admin", "solutions", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const queryString = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<ISolutionListResponse>(
				profile.data,
				`admin/solutions?${queryString}`,
			);
		},
		enabled: !!profile.data,
	});

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({
			queryKey: ["admin", "solutions"],
		});
	}, [queryClient]);

	const handlePageChange = useCallback((newPage: number) => {
		setPage(newPage);
	}, []);

	const handleLimitChange = useCallback((newLimit: number) => {
		setLimit(newLimit);
		setPage(1);
	}, []);

	const handleStatusFilterChange = useCallback(
		(status: SolutionStatus | undefined) => {
			setStatusFilter(status);
			setPage(1);
		},
		[],
	);

	const handleSearchChange = useCallback((query: string) => {
		setSearchQuery(query);
		setPage(1);
	}, []);

	const handleUpdateSolution = useCallback(
		async (id: string, update: ISolutionUpdatePayload) => {
			if (!profile.data) throw new Error("Profile not loaded");

			try {
				await backend.apiState.patch(
					profile.data,
					`admin/solutions/${id}`,
					update,
				);
				toast.success("Solution updated successfully");
			} catch (error) {
				toast.error(
					t("failedToUpdateSolutionVal", "Failed to update solution: {{val}}", {
						val: error instanceof Error ? error.message : "Unknown error",
					}),
				);
				throw error;
			}
		},
		[profile.data, backend.apiState],
	);

	const handleFetchSolution = useCallback(
		async (id: string): Promise<ISolutionRequest | null> => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ISolutionRequest>(
				profile.data,
				`admin/solutions/${id}`,
			);
		},
		[profile.data, backend.apiState],
	);

	const handleAddLog = useCallback(
		async (id: string, log: ISolutionLogPayload) => {
			if (!profile.data) throw new Error("Profile not loaded");

			try {
				await backend.apiState.post(
					profile.data,
					`admin/solutions/${id}/logs`,
					log,
				);
				toast.success("Log added successfully");
			} catch (error) {
				toast.error(
					t("failedToAddLogVal", "Failed to add log: {{val}}", {
						val: error instanceof Error ? error.message : "Unknown error",
					}),
				);
				throw error;
			}
		},
		[profile.data, backend.apiState],
	);

	const openCount = useQuery<ISolutionListResponse, Error>({
		queryKey: ["admin", "solutions", "open-count"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ISolutionListResponse>(
				profile.data,
				"admin/solutions?page=1&limit=1&status=PENDING_REVIEW",
			);
		},
		enabled: !!profile.data,
	});

	const inProgressCount = useQuery<ISolutionListResponse, Error>({
		queryKey: ["admin", "solutions", "in-progress-count"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ISolutionListResponse>(
				profile.data,
				"admin/solutions?page=1&limit=1&status=IN_PROGRESS",
			);
		},
		enabled: !!profile.data,
	});

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-6xl space-y-6">
					<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
						<Card>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">
									{t("totalRequests", "Total Requests")}
								</CardTitle>
								<Inbox className="h-4 w-4 text-muted-foreground" />
							</CardHeader>
							<CardContent>
								{solutions.isLoading ? (
									<Skeleton className="h-8 w-16" />
								) : (
									<div className="text-2xl font-bold">
										{solutions.data?.total ?? 0}
									</div>
								)}
							</CardContent>
						</Card>
						<Card
							className={
								(openCount.data?.total ?? 0) > 0
									? "border-yellow-500/50 bg-yellow-500/5"
									: ""
							}
						>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">
									{t("pendingReview", "Pending Review")}
								</CardTitle>
								<Clock className="h-4 w-4 text-yellow-500" />
							</CardHeader>
							<CardContent>
								{openCount.isLoading ? (
									<Skeleton className="h-8 w-16" />
								) : (
									<div className="text-2xl font-bold">
										{openCount.data?.total ?? 0}
									</div>
								)}
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">
									{t("inProgress", "In Progress")}
								</CardTitle>
								<AlertCircle className="h-4 w-4 text-blue-500" />
							</CardHeader>
							<CardContent>
								{inProgressCount.isLoading ? (
									<Skeleton className="h-8 w-16" />
								) : (
									<div className="text-2xl font-bold">
										{inProgressCount.data?.total ?? 0}
									</div>
								)}
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
								<CardTitle className="text-sm font-medium">
									{t("thisPage", "This Page")}
								</CardTitle>
								<CheckCircle className="h-4 w-4 text-muted-foreground" />
							</CardHeader>
							<CardContent>
								{solutions.isLoading ? (
									<Skeleton className="h-8 w-16" />
								) : (
									<div className="text-2xl font-bold">
										{solutions.data?.solutions.length ?? 0}
									</div>
								)}
							</CardContent>
						</Card>
					</div>

					<SolutionsPage
						data={solutions.data}
						isLoading={solutions.isLoading}
						error={solutions.error}
						page={page}
						limit={limit}
						statusFilter={statusFilter}
						searchQuery={searchQuery}
						onPageChange={handlePageChange}
						onLimitChange={handleLimitChange}
						onStatusFilterChange={handleStatusFilterChange}
						onSearchChange={handleSearchChange}
						onRefresh={handleRefresh}
						onUpdateSolution={handleUpdateSolution}
						onFetchSolution={handleFetchSolution}
						onAddLog={handleAddLog}
						trackingBaseUrl="https://www.flow-like.com"
					/>
				</div>
			</div>
		</main>
	);
}
