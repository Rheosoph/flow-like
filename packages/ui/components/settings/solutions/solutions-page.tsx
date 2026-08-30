"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	ArrowLeft,
	Building2,
	Check,
	ChevronLeft,
	ChevronRight,
	ChevronsLeft,
	ChevronsRight,
	Circle,
	Clock,
	Copy,
	CreditCard,
	Download,
	Edit,
	ExternalLink,
	File,
	FileText,
	Filter,
	Loader2,
	Mail,
	MessageSquare,
	RefreshCw,
	Search,
	Send,
	Star,
	User,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
	type ISolutionListResponse,
	type ISolutionLog,
	type ISolutionLogPayload,
	type ISolutionRequest,
	type ISolutionUpdatePayload,
	SolutionStatus,
	SolutionStatusColors,
	SolutionStatusDescriptions,
	SolutionStatusLabels,
	SolutionStatusOrder,
} from "../../../lib/schema/solution/solution";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import {
	DropdownMenu,
	DropdownMenuCheckboxItem,
	DropdownMenuContent,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { ScrollArea } from "../../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Separator } from "../../ui/separator";
import { Skeleton } from "../../ui/skeleton";
import { Switch } from "../../ui/switch";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { Textarea } from "../../ui/textarea";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui/tooltip";

export interface SolutionsPageProps {
	data: ISolutionListResponse | undefined;
	isLoading: boolean;
	error: Error | null;
	page: number;
	limit: number;
	statusFilter: SolutionStatus | undefined;
	searchQuery: string;
	onPageChange: (page: number) => void;
	onLimitChange: (limit: number) => void;
	onStatusFilterChange: (status: SolutionStatus | undefined) => void;
	onSearchChange: (query: string) => void;
	onRefresh: () => void;
	onUpdateSolution: (
		id: string,
		update: ISolutionUpdatePayload,
	) => Promise<void>;
	onAddLog?: (id: string, log: ISolutionLogPayload) => Promise<void>;
	onFetchSolution?: (id: string) => Promise<ISolutionRequest | null>;
	trackingBaseUrl?: string;
}

export function SolutionsPage({
	data,
	isLoading,
	error,
	page,
	limit,
	statusFilter,
	searchQuery,
	onPageChange,
	onLimitChange,
	onStatusFilterChange,
	onSearchChange,
	onRefresh,
	onUpdateSolution,
	onAddLog,
	onFetchSolution,
	trackingBaseUrl = "",
}: Readonly<SolutionsPageProps>) {
	const { t } = useTranslation("settings");
	const [selectedSolutionId, setSelectedSolutionId] = useState<string | null>(
		null,
	);
	const [selectedSolution, setSelectedSolution] =
		useState<ISolutionRequest | null>(null);
	const [isLoadingDetail, setIsLoadingDetail] = useState(false);

	const handleViewDetails = useCallback(
		async (solution: ISolutionRequest) => {
			setSelectedSolutionId(solution.id);
			if (onFetchSolution) {
				setIsLoadingDetail(true);
				try {
					const detail = await onFetchSolution(solution.id);
					setSelectedSolution(detail);
				} catch {
					setSelectedSolution(solution);
				} finally {
					setIsLoadingDetail(false);
				}
			} else {
				setSelectedSolution(solution);
			}
		},
		[onFetchSolution],
	);

	const handleBack = useCallback(() => {
		setSelectedSolutionId(null);
		setSelectedSolution(null);
	}, []);

	const handleUpdateAndRefresh = useCallback(
		async (id: string, update: ISolutionUpdatePayload) => {
			await onUpdateSolution(id, update);
			onRefresh();
			if (onFetchSolution && selectedSolutionId === id) {
				const updated = await onFetchSolution(id);
				setSelectedSolution(updated);
			}
		},
		[onUpdateSolution, onRefresh, onFetchSolution, selectedSolutionId],
	);

	const handleAddLog = useCallback(
		async (id: string, log: ISolutionLogPayload) => {
			if (onAddLog) {
				await onAddLog(id, log);
				if (onFetchSolution) {
					const updated = await onFetchSolution(id);
					setSelectedSolution(updated);
				}
			}
		},
		[onAddLog, onFetchSolution],
	);

	const totalPages = useMemo(() => {
		if (!data) return 1;
		return Math.ceil(data.total / limit);
	}, [data, limit]);

	if (error) {
		return (
			<Card className="w-full">
				<CardContent className="flex flex-col items-center justify-center py-12 gap-4">
					<AlertCircle className="h-12 w-12 text-destructive" />
					<p className="text-lg font-medium">
						{t("failedToLoadSolutions", "Failed to load solutions")}
					</p>
					<p className="text-sm text-muted-foreground">{error.message}</p>
					<Button onClick={onRefresh} variant="outline">
						<RefreshCw className="h-4 w-4 mr-2" />
						{t("retry", "Retry")}
					</Button>
				</CardContent>
			</Card>
		);
	}

	if (selectedSolutionId && selectedSolution) {
		return (
			<SolutionDetailView
				solution={selectedSolution}
				isLoading={isLoadingDetail}
				onBack={handleBack}
				onUpdate={handleUpdateAndRefresh}
				onAddLog={handleAddLog}
				trackingBaseUrl={trackingBaseUrl}
			/>
		);
	}

	return (
		<div className="space-y-4 w-full max-w-7xl mx-auto">
			<div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
				<div>
					<h2 className="text-2xl font-bold tracking-tight">
						{t("solutionRequests", "Solution Requests")}
					</h2>
					<p className="text-muted-foreground">
						{t(
							"manageAndTrackCustomSolutionRequests",
							"Manage and track custom solution requests",
						)}
					</p>
				</div>
				<Button onClick={onRefresh} variant="outline" size="sm">
					<RefreshCw className="h-4 w-4 mr-2" />
					{t("refresh", "Refresh")}
				</Button>
			</div>

			<SolutionFilters
				searchQuery={searchQuery}
				statusFilter={statusFilter}
				onSearchChange={onSearchChange}
				onStatusFilterChange={onStatusFilterChange}
			/>

			{isLoading ? (
				<SolutionsTableSkeleton />
			) : data?.solutions.length === 0 ? (
				<Card>
					<CardContent className="flex flex-col items-center justify-center py-12 gap-4">
						<FileText className="h-12 w-12 text-muted-foreground" />
						<p className="text-lg font-medium">
							{t("noSolutionsFound", "No solutions found")}
						</p>
						<p className="text-sm text-muted-foreground">
							{statusFilter || searchQuery
								? t("tryAdjustingYourFilters", "Try adjusting your filters")
								: t(
										"noSolutionRequestsHaveBeenSubmittedYet",
										"No solution requests have been submitted yet",
									)}
						</p>
					</CardContent>
				</Card>
			) : (
				<>
					<SolutionsTable
						solutions={data?.solutions ?? []}
						onViewDetails={handleViewDetails}
					/>

					<SolutionsPagination
						page={page}
						limit={limit}
						totalPages={totalPages}
						total={data?.total ?? 0}
						hasMore={data?.hasMore ?? false}
						onPageChange={onPageChange}
						onLimitChange={onLimitChange}
					/>
				</>
			)}
		</div>
	);
}

function SolutionFilters({
	searchQuery,
	statusFilter,
	onSearchChange,
	onStatusFilterChange,
}: Readonly<{
	searchQuery: string;
	statusFilter: SolutionStatus | undefined;
	onSearchChange: (query: string) => void;
	onStatusFilterChange: (status: SolutionStatus | undefined) => void;
}>) {
	const { t } = useTranslation("settings");
	const statusOptions = Object.values(SolutionStatus);

	return (
		<Card>
			<CardContent className="p-4">
				<div className="flex flex-col sm:flex-row gap-4">
					<div className="relative flex-1">
						<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
						<Input
							placeholder={t(
								"searchByNameEmailOrCompany",
								"Search by name, email, or company...",
							)}
							value={searchQuery}
							onChange={(e) => onSearchChange(e.target.value)}
							className="pl-9"
						/>
					</div>

					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button variant="outline" className="min-w-[140px]">
								<Filter className="h-4 w-4 mr-2" />
								{statusFilter
									? SolutionStatusLabels[statusFilter]
									: t("allStatus", "All Status")}
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end" className="w-48">
							<DropdownMenuLabel>
								{t("filterByStatus", "Filter by Status")}
							</DropdownMenuLabel>
							<DropdownMenuSeparator />
							<DropdownMenuCheckboxItem
								checked={!statusFilter}
								onCheckedChange={() => onStatusFilterChange(undefined)}
							>
								{t("allStatus", "All Status")}
							</DropdownMenuCheckboxItem>
							{statusOptions.map((status) => (
								<DropdownMenuCheckboxItem
									key={status}
									checked={statusFilter === status}
									onCheckedChange={() => onStatusFilterChange(status)}
								>
									{SolutionStatusLabels[status]}
								</DropdownMenuCheckboxItem>
							))}
						</DropdownMenuContent>
					</DropdownMenu>

					{(statusFilter || searchQuery) && (
						<Button
							variant="ghost"
							size="sm"
							onClick={() => {
								onSearchChange("");
								onStatusFilterChange(undefined);
							}}
						>
							<X className="h-4 w-4 mr-2" />
							{t("clear", "Clear")}
						</Button>
					)}
				</div>
			</CardContent>
		</Card>
	);
}

function SolutionsTable({
	solutions,
	onViewDetails,
}: Readonly<{
	solutions: ISolutionRequest[];
	onViewDetails: (solution: ISolutionRequest) => void;
}>) {
	const { t } = useTranslation("settings");
	const formatCurrency = (cents: number) => {
		return new Intl.NumberFormat("en-US", {
			style: "currency",
			currency: "EUR",
		}).format(cents / 100);
	};

	const formatDate = (dateString: string) => {
		return new Date(dateString).toLocaleDateString("en-US", {
			year: "numeric",
			month: "short",
			day: "numeric",
		});
	};

	return (
		<Card>
			<Table>
				<TableHeader>
					<TableRow>
						<TableHead className="w-[50px]" />
						<TableHead>{t("requester", "Requester")}</TableHead>
						<TableHead>{t("company", "Company")}</TableHead>
						<TableHead>{t("status", "Status")}</TableHead>
						<TableHead>{t("payment", "Payment")}</TableHead>
						<TableHead className="text-right">{t("total", "Total")}</TableHead>
						<TableHead>{t("created", "Created")}</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{solutions.map((solution) => (
						<TableRow
							key={solution.id}
							className="cursor-pointer hover:bg-muted/50"
							onClick={() => onViewDetails(solution)}
						>
							<TableCell>
								{solution.priority && (
									<Star className="h-4 w-4 text-yellow-500 fill-yellow-500" />
								)}
							</TableCell>
							<TableCell>
								<div className="flex flex-col">
									<span className="font-medium">{solution.name}</span>
									<span className="text-sm text-muted-foreground">
										{solution.email}
									</span>
								</div>
							</TableCell>
							<TableCell>{solution.company}</TableCell>
							<TableCell>
								<StatusBadge status={solution.status} />
							</TableCell>
							<TableCell>
								<PaymentBadge
									paidDeposit={solution.paidDeposit}
									status={solution.status}
								/>
							</TableCell>
							<TableCell className="text-right font-medium">
								{formatCurrency(solution.totalCents)}
							</TableCell>
							<TableCell className="text-muted-foreground">
								{formatDate(solution.createdAt)}
							</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</Card>
	);
}

function StatusBadge({ status }: { status: SolutionStatus }) {
	const getStatusIcon = () => {
		switch (status) {
			case SolutionStatus.AWAITING_DEPOSIT:
				return <CreditCard className="h-3 w-3" />;
			case SolutionStatus.PENDING_REVIEW:
				return <Clock className="h-3 w-3" />;
			case SolutionStatus.IN_QUEUE:
				return <Clock className="h-3 w-3" />;
			case SolutionStatus.ONBOARDING_DONE:
				return <Check className="h-3 w-3" />;
			case SolutionStatus.IN_PROGRESS:
				return <Loader2 className="h-3 w-3 animate-spin" />;
			case SolutionStatus.DELIVERED:
				return <Check className="h-3 w-3" />;
			case SolutionStatus.AWAITING_PAYMENT:
				return <CreditCard className="h-3 w-3" />;
			case SolutionStatus.PAID:
				return <Check className="h-3 w-3" />;
			case SolutionStatus.CANCELLED:
				return <X className="h-3 w-3" />;
			case SolutionStatus.REFUNDED:
				return <CreditCard className="h-3 w-3" />;
			default:
				return <Circle className="h-3 w-3" />;
		}
	};

	return (
		<Badge className={`${SolutionStatusColors[status]} gap-1.5`}>
			{getStatusIcon()}
			{SolutionStatusLabels[status]}
		</Badge>
	);
}

function PaymentBadge({
	paidDeposit,
	status,
}: { paidDeposit: boolean; status: SolutionStatus }) {
	const { t } = useTranslation("settings");
	if (status === SolutionStatus.AWAITING_DEPOSIT) {
		return (
			<Badge
				variant="outline"
				className="text-yellow-600 border-yellow-600/50 bg-yellow-500/10"
			>
				<CreditCard className="h-3 w-3 mr-1" />
				{t("awaiting", "Awaiting")}
			</Badge>
		);
	}

	if (paidDeposit) {
		return (
			<Badge
				variant="outline"
				className="text-green-600 border-green-600/50 bg-green-500/10"
			>
				<Check className="h-3 w-3 mr-1" />
				{t("depositPaid", "Deposit Paid")}
			</Badge>
		);
	}

	return (
		<Badge
			variant="outline"
			className="text-blue-600 border-blue-600/50 bg-blue-500/10"
		>
			<Check className="h-3 w-3 mr-1" />
			{t("setupComplete", "Setup Complete")}
		</Badge>
	);
}

function SolutionsPagination({
	page,
	limit,
	totalPages,
	total,
	hasMore,
	onPageChange,
	onLimitChange,
}: Readonly<{
	page: number;
	limit: number;
	totalPages: number;
	total: number;
	hasMore: boolean;
	onPageChange: (page: number) => void;
	onLimitChange: (limit: number) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<Card>
			<CardContent className="p-4">
				<div className="flex flex-col sm:flex-row items-center justify-between gap-4">
					<div className="text-sm text-muted-foreground">
						{t("showing", "Showing")} {(page - 1) * limit + 1} to{" "}
						{Math.min(page * limit, total)}{" "}
						{t("ofTotalResults", "of {{total}} results", { total })}
					</div>

					<div className="flex items-center gap-4">
						<div className="flex items-center gap-2">
							<Label className="text-sm">{t("perPage", "Per page:")}</Label>
							<Select
								value={limit.toString()}
								onValueChange={(value) => onLimitChange(Number(value))}
							>
								<SelectTrigger className="w-20">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="10">10</SelectItem>
									<SelectItem value="25">25</SelectItem>
									<SelectItem value="50">50</SelectItem>
									<SelectItem value="100">100</SelectItem>
								</SelectContent>
							</Select>
						</div>

						<div className="flex items-center gap-1">
							<Button
								variant="outline"
								size="icon"
								onClick={() => onPageChange(1)}
								disabled={page === 1}
							>
								<ChevronsLeft className="h-4 w-4" />
							</Button>
							<Button
								variant="outline"
								size="icon"
								onClick={() => onPageChange(page - 1)}
								disabled={page === 1}
							>
								<ChevronLeft className="h-4 w-4" />
							</Button>
							<span className="px-4 text-sm">
								{t("pagePageOfTotalpages", "Page {{page}} of {{totalPages}}", {
									page,
									totalPages,
								})}
							</span>
							<Button
								variant="outline"
								size="icon"
								onClick={() => onPageChange(page + 1)}
								disabled={!hasMore}
							>
								<ChevronRight className="h-4 w-4" />
							</Button>
							<Button
								variant="outline"
								size="icon"
								onClick={() => onPageChange(totalPages)}
								disabled={!hasMore}
							>
								<ChevronsRight className="h-4 w-4" />
							</Button>
						</div>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

function SolutionsTableSkeleton() {
	const { t } = useTranslation("settings");
	return (
		<Card>
			<Table>
				<TableHeader>
					<TableRow>
						<TableHead className="w-[50px]" />
						<TableHead>{t("requester", "Requester")}</TableHead>
						<TableHead>{t("company", "Company")}</TableHead>
						<TableHead>{t("status", "Status")}</TableHead>
						<TableHead>{t("payment", "Payment")}</TableHead>
						<TableHead className="text-right">{t("total", "Total")}</TableHead>
						<TableHead>{t("created", "Created")}</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{Array.from({ length: 5 }).map((_, i) => (
						<TableRow key={`skeleton-${i + 1}`}>
							<TableCell>
								<Skeleton className="h-4 w-4" />
							</TableCell>
							<TableCell>
								<div className="space-y-2">
									<Skeleton className="h-4 w-32" />
									<Skeleton className="h-3 w-48" />
								</div>
							</TableCell>
							<TableCell>
								<Skeleton className="h-4 w-24" />
							</TableCell>
							<TableCell>
								<Skeleton className="h-6 w-24" />
							</TableCell>
							<TableCell>
								<Skeleton className="h-6 w-20" />
							</TableCell>
							<TableCell className="text-right">
								<Skeleton className="h-4 w-16 ml-auto" />
							</TableCell>
							<TableCell>
								<Skeleton className="h-4 w-20" />
							</TableCell>
						</TableRow>
					))}
				</TableBody>
			</Table>
		</Card>
	);
}

// Solution Detail View Component
function SolutionDetailView({
	solution,
	isLoading,
	onBack,
	onUpdate,
	onAddLog,
	trackingBaseUrl,
}: Readonly<{
	solution: ISolutionRequest;
	isLoading: boolean;
	onBack: () => void;
	onUpdate: (id: string, update: ISolutionUpdatePayload) => Promise<void>;
	onAddLog: (id: string, log: ISolutionLogPayload) => Promise<void>;
	trackingBaseUrl: string;
}>) {
	const { t } = useTranslation("settings");
	const [isEditing, setIsEditing] = useState(false);
	const [isUpdating, setIsUpdating] = useState(false);
	const [newLogAction, setNewLogAction] = useState("");
	const [isAddingLog, setIsAddingLog] = useState(false);

	const formatCurrency = (cents: number) => {
		return new Intl.NumberFormat("en-US", {
			style: "currency",
			currency: "EUR",
		}).format(cents / 100);
	};

	const formatDate = (dateString: string) => {
		return new Date(dateString).toLocaleString("en-US", {
			year: "numeric",
			month: "long",
			day: "numeric",
			hour: "2-digit",
			minute: "2-digit",
		});
	};

	const formatBytes = (bytes: number) => {
		if (bytes === 0) return "0 Bytes";
		const k = 1024;
		const sizes = ["Bytes", "KB", "MB", "GB"];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return `${Number.parseFloat((bytes / k ** i).toFixed(2))} ${sizes[i]}`;
	};

	const handleCopyTrackingUrl = async () => {
		const url = `${trackingBaseUrl}/solution/track/${solution.trackingToken}`;
		await navigator.clipboard.writeText(url);
	};

	const handleAddLog = async () => {
		if (!newLogAction.trim()) return;
		setIsAddingLog(true);
		try {
			await onAddLog(solution.id, { action: newLogAction });
			setNewLogAction("");
		} finally {
			setIsAddingLog(false);
		}
	};

	if (isLoading) {
		return (
			<div className="space-y-4">
				<Button variant="ghost" onClick={onBack}>
					<ArrowLeft className="h-4 w-4 mr-2" />
					{t("backToList", "Back to list")}
				</Button>
				<div className="flex items-center justify-center py-12">
					<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
				</div>
			</div>
		);
	}

	return (
		<div className="space-y-6">
			{/* Header */}
			<div className="flex items-center justify-between">
				<div className="flex items-center gap-4">
					<Button variant="ghost" size="sm" onClick={onBack}>
						<ArrowLeft className="h-4 w-4 mr-2" />
						{t("back", "Back")}
					</Button>
					<div>
						<div className="flex items-center gap-2">
							<h2 className="text-2xl font-bold">{solution.company}</h2>
							{solution.priority && (
								<Star className="h-5 w-5 text-yellow-500 fill-yellow-500" />
							)}
						</div>
						<p className="text-sm text-muted-foreground">{`Request from ${solution.name}`}</p>
					</div>
				</div>
				<Button variant="outline" onClick={() => setIsEditing(true)}>
					<Edit className="h-4 w-4 mr-2" />
					{t("edit", "Edit")}
				</Button>
			</div>

			{/* Status Timeline */}
			<StatusTimeline status={solution.status} priority={solution.priority} />

			{/* Main Grid */}
			<div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
				{/* Left Column - Main Info */}
				<div className="lg:col-span-2 space-y-6">
					{/* Contact Info */}
					<Card>
						<CardHeader>
							<CardTitle className="text-lg">
								{t("contactInformation", "Contact Information")}
							</CardTitle>
						</CardHeader>
						<CardContent className="grid grid-cols-2 gap-4">
							<InfoItem icon={User} label="Name" value={solution.name} />
							<InfoItem icon={Mail} label="Email" value={solution.email} />
							<InfoItem
								icon={Building2}
								label="Company"
								value={solution.company}
							/>
							<InfoItem
								icon={Clock}
								label="Timeline"
								value={solution.timeline ?? t("notSpecified", "Not specified")}
							/>
						</CardContent>
					</Card>

					{/* Project Details */}
					<Card>
						<CardHeader>
							<CardTitle className="text-lg">
								{t("projectDetails", "Project Details")}
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-6">
							<div className="grid grid-cols-2 gap-4">
								<InfoItem
									label={t("applicationType", "Application Type")}
									value={solution.applicationType}
								/>
								<InfoItem
									label={t("dataSecurity", "Data Security")}
									value={solution.dataSecurity}
								/>
								<InfoItem
									label={t("userType", "User Type")}
									value={solution.userType}
								/>
								<InfoItem
									label={t("userCount", "User Count")}
									value={solution.userCount}
								/>
								<InfoItem
									label={t("technicalLevel", "Technical Level")}
									value={solution.technicalLevel}
								/>
							</div>

							<Separator />

							<div className="space-y-4">
								<div>
									<Label className="text-sm font-semibold">
										{t("description", "Description")}
									</Label>
									<p className="mt-1 text-sm text-muted-foreground whitespace-pre-wrap">
										{solution.description}
									</p>
								</div>
								<div>
									<Label className="text-sm font-semibold">
										{t("exampleInput", "Example Input")}
									</Label>
									<p className="mt-1 text-sm text-muted-foreground whitespace-pre-wrap bg-muted/50 p-3 rounded-md">
										{solution.exampleInput}
									</p>
								</div>
								<div>
									<Label className="text-sm font-semibold">
										{t("expectedOutput", "Expected Output")}
									</Label>
									<p className="mt-1 text-sm text-muted-foreground whitespace-pre-wrap bg-muted/50 p-3 rounded-md">
										{solution.expectedOutput}
									</p>
								</div>
								{solution.additionalNotes && (
									<div>
										<Label className="text-sm font-semibold">
											{t("additionalNotes", "Additional Notes")}
										</Label>
										<p className="mt-1 text-sm text-muted-foreground whitespace-pre-wrap">
											{solution.additionalNotes}
										</p>
									</div>
								)}
							</div>
						</CardContent>
					</Card>

					{/* Files */}
					{solution.files && solution.files.length > 0 && (
						<Card>
							<CardHeader>
								<CardTitle className="text-lg flex items-center gap-2">
									<File className="h-5 w-5" />
									{t("attachedFiles", "Attached Files")}
								</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="space-y-2">
									{solution.files.map((file, index) => (
										<div
											key={`file-${file.key || index}`}
											className="flex items-center justify-between p-3 rounded-lg border bg-muted/30"
										>
											<div className="flex items-center gap-3">
												<FileText className="h-5 w-5 text-muted-foreground" />
												<div>
													<p className="font-medium text-sm">{file.name}</p>
													<p className="text-xs text-muted-foreground">
														{formatBytes(file.size)}
													</p>
												</div>
											</div>
											{file.downloadUrl && (
												<Button variant="ghost" size="sm" asChild>
													<a
														href={file.downloadUrl}
														target="_blank"
														rel="noopener noreferrer"
													>
														<Download className="h-4 w-4" />
													</a>
												</Button>
											)}
										</div>
									))}
								</div>
							</CardContent>
						</Card>
					)}

					{/* Admin Notes */}
					{solution.adminNotes && (
						<Card>
							<CardHeader>
								<CardTitle className="text-lg">
									{t("adminNotes", "Admin Notes")}
								</CardTitle>
							</CardHeader>
							<CardContent>
								<p className="text-sm text-muted-foreground whitespace-pre-wrap">
									{solution.adminNotes}
								</p>
							</CardContent>
						</Card>
					)}
				</div>

				{/* Right Column - Sidebar */}
				<div className="space-y-6">
					{/* Payment Info */}
					<Card>
						<CardHeader>
							<CardTitle className="text-lg flex items-center gap-2">
								<CreditCard className="h-5 w-5" />
								{t("payment", "Payment")}
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							<div className="flex items-center justify-between">
								<span className="text-sm text-muted-foreground">
									{t("total", "Total")}
								</span>
								<span className="font-semibold">
									{formatCurrency(solution.totalCents)}
								</span>
							</div>
							<div className="flex items-center justify-between">
								<span className="text-sm text-muted-foreground">
									{t("deposit", "Deposit")}
								</span>
								<div className="flex items-center gap-2">
									<span className="font-medium">
										{formatCurrency(solution.depositCents)}
									</span>
									{solution.paidDeposit ? (
										<Badge className="bg-green-500/10 text-green-500">
											{t("paid", "Paid")}
										</Badge>
									) : (
										<Badge className="bg-yellow-500/10 text-yellow-500">
											{t("pending", "Pending")}
										</Badge>
									)}
								</div>
							</div>
							<div className="flex items-center justify-between">
								<span className="text-sm text-muted-foreground">
									{t("remainder", "Remainder")}
								</span>
								<span className="font-medium">
									{formatCurrency(solution.remainderCents)}
								</span>
							</div>
							<Separator />
							<div className="flex items-center justify-between">
								<span className="text-sm text-muted-foreground">
									{t("tier", "Tier")}
								</span>
								<Badge variant="outline">{solution.pricingTier}</Badge>
							</div>
						</CardContent>
					</Card>

					{/* Tracking URL */}
					{solution.trackingToken && (
						<Card>
							<CardHeader>
								<CardTitle className="text-lg flex items-center gap-2">
									<ExternalLink className="h-5 w-5" />
									{t("customerTracking", "Customer Tracking")}
								</CardTitle>
								<CardDescription>
									{t(
										"shareThisUrlWithTheCustomerToTrackStatus",
										"Share this URL with the customer to track status",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<div className="flex items-center gap-2">
									<Input
										value={`${trackingBaseUrl}/solution/track/${solution.trackingToken}`}
										readOnly
										className="text-xs"
									/>
									<TooltipProvider>
										<Tooltip>
											<TooltipTrigger asChild>
												<Button
													variant="outline"
													size="icon"
													onClick={handleCopyTrackingUrl}
												>
													<Copy className="h-4 w-4" />
												</Button>
											</TooltipTrigger>
											<TooltipContent>
												{t("copyUrl", "Copy URL")}
											</TooltipContent>
										</Tooltip>
									</TooltipProvider>
								</div>
							</CardContent>
						</Card>
					)}

					{/* Activity Log */}
					<Card>
						<CardHeader>
							<CardTitle className="text-lg flex items-center gap-2">
								<MessageSquare className="h-5 w-5" />
								{t("activityLog", "Activity Log")}
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-4">
							{/* Add log input */}
							<div className="flex gap-2">
								<Input
									placeholder={t("addALogEntry", "Add a log entry...")}
									value={newLogAction}
									onChange={(e) => setNewLogAction(e.target.value)}
									onKeyDown={(e) => {
										if (e.key === "Enter" && !e.shiftKey) {
											e.preventDefault();
											handleAddLog();
										}
									}}
								/>
								<Button
									size="icon"
									onClick={handleAddLog}
									disabled={isAddingLog || !newLogAction.trim()}
								>
									{isAddingLog ? (
										<Loader2 className="h-4 w-4 animate-spin" />
									) : (
										<Send className="h-4 w-4" />
									)}
								</Button>
							</div>

							<ScrollArea className="h-[300px]">
								<div className="space-y-3">
									{solution.logs && solution.logs.length > 0 ? (
										solution.logs.map((log) => (
											<LogEntry key={log.id} log={log} />
										))
									) : (
										<p className="text-sm text-muted-foreground text-center py-4">
											{t("noActivityYet", "No activity yet")}
										</p>
									)}
								</div>
							</ScrollArea>
						</CardContent>
					</Card>

					{/* Metadata */}
					<Card>
						<CardHeader>
							<CardTitle className="text-lg">
								{t("details", "Details")}
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-3 text-sm">
							<div className="flex items-center justify-between">
								<span className="text-muted-foreground">
									{t("assignedTo", "Assigned To")}
								</span>
								<span>{solution.assignedTo ?? "Unassigned"}</span>
							</div>
							<div className="flex items-center justify-between">
								<span className="text-muted-foreground">
									{t("created", "Created")}
								</span>
								<span>{formatDate(solution.createdAt)}</span>
							</div>
							<div className="flex items-center justify-between">
								<span className="text-muted-foreground">
									{t("updated", "Updated")}
								</span>
								<span>{formatDate(solution.updatedAt)}</span>
							</div>
							{solution.deliveredAt && (
								<div className="flex items-center justify-between">
									<span className="text-muted-foreground">
										{t("delivered", "Delivered")}
									</span>
									<span>{formatDate(solution.deliveredAt)}</span>
								</div>
							)}
						</CardContent>
					</Card>
				</div>
			</div>

			{/* Edit Dialog */}
			<SolutionEditDialog
				solution={isEditing ? solution : null}
				onClose={() => setIsEditing(false)}
				onSave={async (update) => {
					setIsUpdating(true);
					try {
						await onUpdate(solution.id, update);
						setIsEditing(false);
					} finally {
						setIsUpdating(false);
					}
				}}
				isUpdating={isUpdating}
			/>
		</div>
	);
}

function StatusTimeline({
	status,
	priority,
}: { status: SolutionStatus; priority: boolean }) {
	const isCancelled =
		status === SolutionStatus.CANCELLED || status === SolutionStatus.REFUNDED;

	// For priority orders, show all steps. For standard orders, skip the AwaitingDeposit step.
	const statusOrder = priority
		? SolutionStatusOrder
		: SolutionStatusOrder.filter((s) => s !== SolutionStatus.AWAITING_DEPOSIT);

	const currentIndex = statusOrder.indexOf(status);
	const isPaid = status === SolutionStatus.PAID;

	if (isCancelled) {
		return (
			<Card className="border-destructive/50 bg-destructive/5">
				<CardContent className="py-4">
					<div className="flex items-center justify-center gap-3">
						<div className="h-8 w-8 rounded-full bg-destructive/10 flex items-center justify-center">
							<X className="h-4 w-4 text-destructive" />
						</div>
						<div className="flex flex-col">
							<span className="font-medium text-destructive">
								{SolutionStatusLabels[status]}
							</span>
							<span className="text-sm text-muted-foreground">
								{SolutionStatusDescriptions[status]}
							</span>
						</div>
					</div>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card>
			<CardContent className="py-4 px-3">
				<div className="flex items-start justify-between gap-1">
					{statusOrder.map((s, index) => {
						const isCompleted = currentIndex > index || isPaid;
						const isCurrent = currentIndex === index && !isPaid;
						const isLast = index === statusOrder.length - 1;

						return (
							<div key={s} className="flex items-start flex-1">
								<div className="flex flex-col items-center min-w-10">
									<div
										className={`
											w-6 h-6 rounded-full flex items-center justify-center border-2 transition-all
											${isCompleted ? "bg-green-500 border-green-500 text-white" : ""}
											${isCurrent ? "border-primary bg-primary/10 text-primary" : ""}
											${!isCompleted && !isCurrent ? "border-muted-foreground/30 bg-muted/50 text-muted-foreground" : ""}
										`}
									>
										{isCompleted ? (
											<Check className="h-3 w-3" />
										) : isCurrent ? (
											<div className="h-1.5 w-1.5 rounded-full bg-primary animate-pulse" />
										) : (
											<span className="text-[10px] font-medium">
												{index + 1}
											</span>
										)}
									</div>
									<span
										className={`
											mt-1.5 text-[10px] font-medium text-center leading-tight max-w-[50px]
											${isCompleted ? "text-green-600 dark:text-green-400" : ""}
											${isCurrent ? "text-primary font-semibold" : ""}
											${!isCompleted && !isCurrent ? "text-muted-foreground" : ""}
										`}
									>
										{SolutionStatusLabels[s]}
									</span>
								</div>
								{!isLast && (
									<div className="flex-1 flex items-center pt-3 px-0.5">
										<div
											className={`
												flex-1 h-0.5 rounded-full transition-colors
												${isCompleted ? "bg-green-500" : "bg-muted-foreground/20"}
											`}
										/>
									</div>
								)}
							</div>
						);
					})}
				</div>
			</CardContent>
		</Card>
	);
}

function LogEntry({ log }: { log: ISolutionLog }) {
	const formatDate = (dateString: string) => {
		return new Date(dateString).toLocaleString("en-US", {
			month: "short",
			day: "numeric",
			hour: "2-digit",
			minute: "2-digit",
		});
	};

	return (
		<div className="flex gap-3 p-3 rounded-lg bg-muted/30">
			<div className="h-8 w-8 rounded-full bg-primary/10 flex items-center justify-center shrink-0">
				<MessageSquare className="h-4 w-4 text-primary" />
			</div>
			<div className="flex-1 min-w-0">
				<p className="text-sm font-medium">{log.action}</p>
				{log.details && (
					<p className="text-xs text-muted-foreground mt-1">{log.details}</p>
				)}
				<div className="flex items-center gap-2 mt-1 text-xs text-muted-foreground">
					<span>{formatDate(log.createdAt)}</span>
					{log.actor && (
						<>
							<span>•</span>
							<span>{log.actor}</span>
						</>
					)}
				</div>
			</div>
		</div>
	);
}

function InfoItem({
	icon: Icon,
	label,
	value,
}: Readonly<{
	icon?: React.ComponentType<{ className?: string }>;
	label: string;
	value: string;
}>) {
	return (
		<div className="space-y-1">
			<Label className="text-xs text-muted-foreground flex items-center gap-1">
				{Icon && <Icon className="h-3 w-3" />}
				{label}
			</Label>
			<p className="text-sm font-medium">{value}</p>
		</div>
	);
}

function SolutionEditDialog({
	solution,
	onClose,
	onSave,
	isUpdating,
}: Readonly<{
	solution: ISolutionRequest | null;
	onClose: () => void;
	onSave: (update: ISolutionUpdatePayload) => Promise<void>;
	isUpdating: boolean;
}>) {
	const { t } = useTranslation("settings");
	const [status, setStatus] = useState<SolutionStatus | undefined>();
	const [adminNotes, setAdminNotes] = useState("");
	const [assignedTo, setAssignedTo] = useState("");
	const [priority, setPriority] = useState(false);

	useEffect(() => {
		if (solution) {
			setStatus(solution.status);
			setAdminNotes(solution.adminNotes ?? "");
			setAssignedTo(solution.assignedTo ?? "");
			setPriority(solution.priority);
		}
	}, [solution]);

	if (!solution) return null;

	const handleSave = async () => {
		const update: ISolutionUpdatePayload = {};
		if (status && status !== solution.status) update.status = status;
		if (adminNotes !== (solution.adminNotes ?? ""))
			update.adminNotes = adminNotes;
		if (assignedTo !== (solution.assignedTo ?? ""))
			update.assignedTo = assignedTo;
		if (priority !== solution.priority) update.priority = priority;

		await onSave(update);
	};

	return (
		<Dialog open={!!solution} onOpenChange={onClose}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle>
						{t("editSolutionRequest", "Edit Solution Request")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"updateTheStatusAndAdminDetailsForThisSolutionRequest",
							"Update the status and admin details for this solution request.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label>{t("status", "Status")}</Label>
						<Select
							value={status}
							onValueChange={(value) => setStatus(value as SolutionStatus)}
						>
							<SelectTrigger>
								<SelectValue placeholder={t("selectStatus", "Select status")} />
							</SelectTrigger>
							<SelectContent>
								{Object.values(SolutionStatus).map((s) => (
									<SelectItem key={s} value={s}>
										{SolutionStatusLabels[s]}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
					</div>

					<div className="space-y-2">
						<Label>{t("assignedTo", "Assigned To")}</Label>
						<Input
							value={assignedTo}
							onChange={(e) => setAssignedTo(e.target.value)}
							placeholder={t(
								"enterAssigneeNameOrEmail",
								"Enter assignee name or email",
							)}
						/>
					</div>

					<div className="space-y-2">
						<Label>{t("adminNotes", "Admin Notes")}</Label>
						<Textarea
							value={adminNotes}
							onChange={(e) => setAdminNotes(e.target.value)}
							placeholder={t(
								"internalNotesAboutThisRequest",
								"Internal notes about this request...",
							)}
							rows={4}
						/>
					</div>

					<div className="flex items-center justify-between">
						<div className="space-y-0.5">
							<Label>{t("priority", "Priority")}</Label>
							<p className="text-sm text-muted-foreground">
								{t(
									"markThisRequestAsHighPriority",
									"Mark this request as high priority",
								)}
							</p>
						</div>
						<Switch checked={priority} onCheckedChange={setPriority} />
					</div>
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={onClose} disabled={isUpdating}>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={handleSave} disabled={isUpdating}>
						{isUpdating && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}
						{t("saveChanges", "Save Changes")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
