"use client";

import {
	AdminPackageDetail,
	type AdminPackageListResponse,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
	type PackageAdminStatus,
	type PackageDetails,
	type PackageVisibility,
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
	useBackend,
	useInvoke,
	useQuery,
	useQueryClient,
} from "@tm9657/flow-like-ui";
import { useDebounce } from "@uidotdev/usehooks";
import { formatDistanceToNow } from "date-fns";
import {
	CheckCircle,
	Clock,
	Download,
	ExternalLink,
	Package,
	RefreshCw,
	Search,
	Shield,
	XCircle,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useMemo, useState } from "react";
import { toast } from "sonner";

const statusBadgeVariant: Record<
	PackageAdminStatus,
	"default" | "secondary" | "destructive" | "outline"
> = {
	pending_review: "secondary",
	active: "default",
	rejected: "destructive",
	deprecated: "outline",
	disabled: "outline",
};

const statusIcon: Record<PackageAdminStatus, React.ReactNode> = {
	pending_review: <Clock className="h-3 w-3" />,
	active: <CheckCircle className="h-3 w-3" />,
	rejected: <XCircle className="h-3 w-3" />,
	deprecated: <Clock className="h-3 w-3" />,
	disabled: <XCircle className="h-3 w-3" />,
};

const visibilityVariant: Record<
	PackageVisibility,
	"default" | "secondary" | "outline"
> = {
	public: "default",
	private: "secondary",
	public_request_access: "outline",
};

const visibilityLabel: Record<PackageVisibility, string> = {
	public: "Public",
	private: "Private",
	public_request_access: "Request Access",
};

function formatDownloadCount(count: number | null | undefined) {
	return (count ?? 0).toLocaleString();
}

function formatRelativeDate(value: string | null | undefined) {
	if (!value) return "Unknown";
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return "Unknown";
	return formatDistanceToNow(date, { addSuffix: true });
}

function StatsCard({
	title,
	value,
	icon,
	loading,
}: {
	title: string;
	value: number | string;
	icon: React.ReactNode;
	loading: boolean;
}) {
	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium">{title}</CardTitle>
				{icon}
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-8 w-16" />
				) : (
					<div className="text-2xl font-bold">{value}</div>
				)}
			</CardContent>
		</Card>
	);
}

function PackageRow({ pkg }: { pkg: PackageDetails }) {
	return (
		<TableRow>
			<TableCell className="font-medium">
				<Link
					href={`/admin/packages?id=${pkg.id}`}
					className="hover:underline flex items-center gap-2"
				>
					<Package className="h-4 w-4" />
					{pkg.name}
				</Link>
			</TableCell>
			<TableCell className="max-w-xs truncate">{pkg.description}</TableCell>
			<TableCell>{pkg.version}</TableCell>
			<TableCell>
				<Badge variant={statusBadgeVariant[pkg.status]} className="gap-1">
					{statusIcon[pkg.status]}
					{pkg.status.replace("_", " ")}
				</Badge>
			</TableCell>
			<TableCell>
				<Badge variant={visibilityVariant[pkg.visibility]} className="gap-1">
					{visibilityLabel[pkg.visibility]}
				</Badge>
			</TableCell>
			<TableCell>
				{pkg.verified && <Shield className="h-4 w-4 text-blue-500" />}
			</TableCell>
			<TableCell className="text-right">
				<span className="flex items-center gap-1 justify-end">
					<Download className="h-3 w-3" />
					{formatDownloadCount(pkg.downloadCount)}
				</span>
			</TableCell>
			<TableCell>{formatRelativeDate(pkg.createdAt)}</TableCell>
			<TableCell>
				{pkg.repository && (
					<a
						href={pkg.repository}
						target="_blank"
						rel="noopener noreferrer"
						className="text-muted-foreground hover:text-foreground"
					>
						<ExternalLink className="h-4 w-4" />
					</a>
				)}
			</TableCell>
		</TableRow>
	);
}

function AdminPackageListContent() {
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [page, setPage] = useState(1);
	const [limit, setLimit] = useState(25);
	const [statusFilter, setStatusFilter] = useState<PackageAdminStatus | "all">(
		"all",
	);
	const [searchQuery, setSearchQuery] = useState("");
	const debouncedSearch = useDebounce(searchQuery, 300);

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			offset: (page - 1) * limit,
			limit,
		};
		if (statusFilter !== "all") params.status = statusFilter;
		return params;
	}, [page, limit, statusFilter]);

	const packages = useQuery<AdminPackageListResponse>({
		queryKey: ["admin", "packages", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const queryString = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<AdminPackageListResponse>(
				profile.data,
				`admin/packages?${queryString}`,
			);
		},
		enabled: !!profile.data,
	});

	const stats = useQuery<{
		totalPackages: number;
		totalVersions: number;
		totalDownloads: number;
		pendingReview: number;
		activePackages: number;
		rejectedPackages: number;
	}>({
		queryKey: ["admin", "packages", "stats"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get(profile.data, "admin/packages/stats");
		},
		enabled: !!profile.data,
	});

	const handleRefresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["admin", "packages"] });
	}, [queryClient]);

	const filteredPackages = useMemo(() => {
		if (!packages.data?.packages) return [];
		if (!debouncedSearch) return packages.data.packages;
		const search = debouncedSearch.toLowerCase();
		return packages.data.packages.filter(
			(pkg) =>
				pkg.name.toLowerCase().includes(search) ||
				pkg.description.toLowerCase().includes(search) ||
				pkg.id.toLowerCase().includes(search),
		);
	}, [packages.data?.packages, debouncedSearch]);

	const totalPages = Math.ceil((packages.data?.totalCount ?? 0) / limit);

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-6xl space-y-6">
					<div className="flex items-center justify-between">
						<div>
							<h1 className="text-3xl font-bold">Package Registry</h1>
							<p className="text-muted-foreground">
								Review and manage WASM packages
							</p>
						</div>
						<Button onClick={handleRefresh} variant="outline" size="sm">
							<RefreshCw className="h-4 w-4 mr-2" />
							Refresh
						</Button>
					</div>

					<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
						<StatsCard
							title="Pending Review"
							value={stats.data?.pendingReview ?? 0}
							icon={<Clock className="h-4 w-4 text-yellow-500" />}
							loading={stats.isLoading}
						/>
						<StatsCard
							title="Active Packages"
							value={stats.data?.activePackages ?? 0}
							icon={<CheckCircle className="h-4 w-4 text-green-500" />}
							loading={stats.isLoading}
						/>
						<StatsCard
							title="Total Downloads"
							value={(stats.data?.totalDownloads ?? 0).toLocaleString()}
							icon={<Download className="h-4 w-4 text-blue-500" />}
							loading={stats.isLoading}
						/>
						<StatsCard
							title="Total Versions"
							value={stats.data?.totalVersions ?? 0}
							icon={<Package className="h-4 w-4 text-purple-500" />}
							loading={stats.isLoading}
						/>
					</div>

					{(stats.data?.pendingReview ?? 0) > 0 && (
						<Card className="border-yellow-500/50 bg-yellow-500/5">
							<CardHeader className="py-3">
								<CardTitle className="flex items-center gap-2 text-sm font-medium text-yellow-700 dark:text-yellow-400">
									<Clock className="h-4 w-4" />
									{stats.data?.pendingReview} package
									{(stats.data?.pendingReview ?? 0) > 1 ? "s" : ""} waiting for
									review
								</CardTitle>
							</CardHeader>
						</Card>
					)}

					<Card>
						<CardHeader>
							<CardTitle>Packages</CardTitle>
							<CardDescription>
								{packages.data?.totalCount ?? 0} total packages
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="flex items-center gap-4 mb-4">
								<div className="relative flex-1 max-w-sm">
									<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
									<Input
										placeholder="Search packages..."
										value={searchQuery}
										onChange={(e) => setSearchQuery(e.target.value)}
										className="pl-10"
									/>
								</div>
								<Select
									value={statusFilter}
									onValueChange={(v) =>
										setStatusFilter(v as PackageAdminStatus | "all")
									}
								>
									<SelectTrigger className="w-48">
										<SelectValue placeholder="Filter by status" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="all">All statuses</SelectItem>
										<SelectItem value="pending_review">
											Pending Review
										</SelectItem>
										<SelectItem value="active">Active</SelectItem>
										<SelectItem value="rejected">Rejected</SelectItem>
										<SelectItem value="deprecated">Deprecated</SelectItem>
										<SelectItem value="disabled">Disabled</SelectItem>
									</SelectContent>
								</Select>
							</div>

							{packages.isLoading ? (
								<div className="space-y-2">
									{[...Array(5)].map((_, i) => (
										<Skeleton key={i} className="h-12 w-full" />
									))}
								</div>
							) : (
								<Table>
									<TableHeader>
										<TableRow>
											<TableHead>Name</TableHead>
											<TableHead>Description</TableHead>
											<TableHead>Version</TableHead>
											<TableHead>Status</TableHead>
											<TableHead>Visibility</TableHead>
											<TableHead>Verified</TableHead>
											<TableHead className="text-right">Downloads</TableHead>
											<TableHead>Created</TableHead>
											<TableHead>Repo</TableHead>
										</TableRow>
									</TableHeader>
									<TableBody>
										{filteredPackages.map((pkg) => (
											<PackageRow key={pkg.id} pkg={pkg} />
										))}
										{filteredPackages.length === 0 && (
											<TableRow>
												<TableCell colSpan={9} className="text-center py-8">
													No packages found
												</TableCell>
											</TableRow>
										)}
									</TableBody>
								</Table>
							)}

							{totalPages > 1 && (
								<div className="flex items-center justify-between mt-4">
									<div className="text-sm text-muted-foreground">
										Page {page} of {totalPages}
									</div>
									<div className="flex gap-2">
										<Button
											variant="outline"
											size="sm"
											onClick={() => setPage((p) => Math.max(1, p - 1))}
											disabled={page === 1}
										>
											Previous
										</Button>
										<Button
											variant="outline"
											size="sm"
											onClick={() =>
												setPage((p) => Math.min(totalPages, p + 1))
											}
											disabled={page === totalPages}
										>
											Next
										</Button>
									</div>
								</div>
							)}
						</CardContent>
					</Card>
				</div>
			</div>
		</main>
	);
}

function AdminPackageDetailWrapper() {
	const searchParams = useSearchParams();
	const router = useRouter();
	const packageId = searchParams.get("id") ?? "";

	const handleBack = useCallback(() => router.back(), [router]);
	const handleSuccess = useCallback(() => {
		toast.success("Operation completed successfully");
	}, []);

	return (
		<AdminPackageDetail
			packageId={packageId}
			onBack={handleBack}
			onSuccess={handleSuccess}
		/>
	);
}

function PageContent() {
	const searchParams = useSearchParams();
	const packageId = searchParams.get("id");

	if (packageId) {
		return (
			<Suspense fallback={<Skeleton className="h-full w-full" />}>
				<AdminPackageDetailWrapper />
			</Suspense>
		);
	}

	return <AdminPackageListContent />;
}

export default function AdminPackagesPage() {
	return (
		<Suspense fallback={<Skeleton className="h-full w-full" />}>
			<PageContent />
		</Suspense>
	);
}
