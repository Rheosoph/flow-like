"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ChevronLeft,
	ChevronRight,
	RefreshCw,
	Search,
	ShieldAlert,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	Input,
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

const SCORE_CATEGORIES = [
	"security",
	"privacy",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

type ScoreCategory = (typeof SCORE_CATEGORIES)[number];

interface AppScoreItem {
	appId: string;
	appName?: string | null;
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
	worstScore: number;
	boardCount: number;
	nodeCount: number;
	scoredNodeCount: number;
}

interface ListAppScoresResponse {
	apps: AppScoreItem[];
	total: number;
	page: number;
	limit: number;
	hasMore: boolean;
}

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

interface PatternItem {
	node: string;
	category: string;
	appCount: number;
	occurrenceCount: number;
	minScore: number;
}

interface ListPatternsResponse {
	patterns: PatternItem[];
	total: number;
	page: number;
	limit: number;
	hasMore: boolean;
}

interface RecomputeResponse {
	appsProcessed: number;
	boardsProcessed: number;
	failures: number;
}

const PAGE_SIZE = 25;

function scoreColor(value: number): string {
	if (value >= 7) return "bg-green-500";
	if (value >= 4) return "bg-yellow-500";
	return "bg-red-500";
}

function scoreTextColor(value: number): string {
	if (value >= 7) return "text-green-600 dark:text-green-400";
	if (value >= 4) return "text-yellow-600 dark:text-yellow-400";
	return "text-red-600 dark:text-red-400";
}

function ScoreChip({ value }: { value: number }) {
	return (
		<span
			className={`inline-flex h-6 w-6 items-center justify-center rounded-md text-xs font-semibold text-white ${scoreColor(
				value,
			)}`}
		>
			{value}
		</span>
	);
}

function CategoryHeaderCells() {
	return (
		<>
			{SCORE_CATEGORIES.map((category) => (
				<TableHead key={category} className="text-center capitalize">
					{category.slice(0, 4)}
				</TableHead>
			))}
		</>
	);
}

function CategoryScoreCells({
	item,
}: {
	item: Pick<AppScoreItem, ScoreCategory>;
}) {
	return (
		<>
			{SCORE_CATEGORIES.map((category) => (
				<TableCell key={category} className="text-center">
					<ScoreChip value={item[category]} />
				</TableCell>
			))}
		</>
	);
}

export function AdminGovernanceScoresPage() {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [selectedAppId, setSelectedAppId] = useState<string | null>(null);

	const handleRecompute = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<RecomputeResponse>(
				profile.data,
				"admin/governance/scores/recompute",
				selectedAppId ? { appId: selectedAppId } : {},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "governance"],
			});
		},
	});

	if (selectedAppId) {
		return (
			<AppScoreDetail
				appId={selectedAppId}
				onBack={() => setSelectedAppId(null)}
			/>
		);
	}

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between gap-3">
				<div>
					<h2 className="text-lg font-semibold">Governance Scores</h2>
					<p className="text-sm text-muted-foreground">
						Quality and governance scores aggregated per app from board scores.
					</p>
				</div>
				<Button
					variant="outline"
					size="sm"
					disabled={handleRecompute.isPending || !profile.data}
					onClick={() => handleRecompute.mutate()}
				>
					<RefreshCw
						className={`mr-2 h-4 w-4 ${
							handleRecompute.isPending ? "animate-spin" : ""
						}`}
					/>
					Recompute all
				</Button>
			</div>

			<Tabs defaultValue="apps">
				<TabsList>
					<TabsTrigger value="apps">Apps</TabsTrigger>
					<TabsTrigger value="patterns">Bad Patterns</TabsTrigger>
				</TabsList>
				<TabsContent value="apps" className="mt-4">
					<AppsTab onSelectApp={setSelectedAppId} />
				</TabsContent>
				<TabsContent value="patterns" className="mt-4">
					<PatternsTab />
				</TabsContent>
			</Tabs>
		</div>
	);
}

function AppsTab({ onSelectApp }: { onSelectApp: (appId: string) => void }) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [search, setSearch] = useState("");
	const [category, setCategory] = useState<string>("all");
	const [threshold, setThreshold] = useState<string>("all");
	const [page, setPage] = useState(1);

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			page,
			limit: PAGE_SIZE,
		};
		if (search.trim()) params.search = search.trim();
		if (threshold !== "all") {
			params.threshold = Number(threshold);
			if (category !== "all") params.category = category;
		}
		return params;
	}, [page, search, category, threshold]);

	const scores = useQuery<ListAppScoresResponse>({
		queryKey: ["admin", "governance", "scores", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<ListAppScoresResponse>(
				profile.data,
				`admin/governance/scores?${qs}`,
			);
		},
		enabled: !!profile.data,
	});

	const totalPages = Math.max(
		1,
		Math.ceil((scores.data?.total ?? 0) / PAGE_SIZE),
	);

	const resetPage = useCallback(() => setPage(1), []);

	return (
		<div className="space-y-4">
			<div className="flex flex-wrap items-center gap-2">
				<div className="relative flex-1 min-w-[200px]">
					<Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
					<Input
						value={search}
						onChange={(event) => {
							setSearch(event.target.value);
							resetPage();
						}}
						placeholder="Search by app name or id..."
						className="pl-8"
					/>
				</div>
				<Select
					value={category}
					onValueChange={(value) => {
						setCategory(value);
						resetPage();
					}}
				>
					<SelectTrigger className="w-40">
						<SelectValue placeholder="Category" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">Worst score</SelectItem>
						{SCORE_CATEGORIES.map((cat) => (
							<SelectItem key={cat} value={cat} className="capitalize">
								{cat}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<Select
					value={threshold}
					onValueChange={(value) => {
						setThreshold(value);
						resetPage();
					}}
				>
					<SelectTrigger className="w-40">
						<SelectValue placeholder="Threshold" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">Any score</SelectItem>
						<SelectItem value="3">≤ 3 (critical)</SelectItem>
						<SelectItem value="4">≤ 4 (flagged)</SelectItem>
						<SelectItem value="6">≤ 6 (warning)</SelectItem>
					</SelectContent>
				</Select>
			</div>

			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>App</TableHead>
								<TableHead className="text-center">Worst</TableHead>
								<CategoryHeaderCells />
								<TableHead className="text-center">Boards</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{scores.isLoading && (
								<TableRow>
									<TableCell colSpan={10} className="py-8">
										<Skeleton className="h-24 w-full" />
									</TableCell>
								</TableRow>
							)}
							{!scores.isLoading &&
								(!scores.data?.apps?.length ? (
									<TableRow>
										<TableCell
											colSpan={10}
											className="text-center text-muted-foreground py-8"
										>
											No scored apps found.
										</TableCell>
									</TableRow>
								) : (
									scores.data.apps.map((app) => (
										<TableRow
											key={app.appId}
											className="cursor-pointer"
											onClick={() => onSelectApp(app.appId)}
										>
											<TableCell>
												<div className="min-w-0">
													<p className="font-medium text-sm truncate">
														{app.appName ?? app.appId}
													</p>
													<p className="text-xs text-muted-foreground truncate">
														{app.appId}
													</p>
												</div>
											</TableCell>
											<TableCell className="text-center">
												<span
													className={`text-base font-bold ${scoreTextColor(
														app.worstScore,
													)}`}
												>
													{app.worstScore}
												</span>
											</TableCell>
											<CategoryScoreCells item={app} />
											<TableCell className="text-center text-sm text-muted-foreground">
												{app.boardCount}
											</TableCell>
										</TableRow>
									))
								))}
						</TableBody>
					</Table>
				</CardContent>
			</Card>

			<Pagination
				page={page}
				totalPages={totalPages}
				total={scores.data?.total ?? 0}
				onPrev={() => setPage((p) => Math.max(1, p - 1))}
				onNext={() => setPage((p) => p + 1)}
				hasMore={scores.data?.hasMore ?? false}
			/>
		</div>
	);
}

function AppScoreDetail({
	appId,
	onBack,
}: {
	appId: string;
	onBack: () => void;
}) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const detail = useQuery<AppScoreDetailResponse>({
		queryKey: ["admin", "governance", "scores", appId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<AppScoreDetailResponse>(
				profile.data,
				`admin/governance/scores/${encodeURIComponent(appId)}`,
			);
		},
		enabled: !!profile.data,
	});

	return (
		<div className="space-y-4">
			<Button variant="ghost" size="sm" onClick={onBack} className="-ml-2">
				<ChevronLeft className="mr-1 h-4 w-4" />
				Back to scores
			</Button>

			<div>
				<h2 className="text-lg font-semibold">
					{detail.data?.appName ?? appId}
				</h2>
				<p className="text-sm text-muted-foreground">{appId}</p>
			</div>

			{detail.isLoading && <Skeleton className="h-48 w-full" />}

			{!detail.isLoading && !detail.data?.boards?.length && (
				<Card>
					<CardContent className="py-8 text-center text-muted-foreground text-sm">
						No board scores recorded for this app.
					</CardContent>
				</Card>
			)}

			{detail.data?.boards?.map((board) => (
				<Card key={board.boardId}>
					<CardContent className="space-y-3 p-4">
						<div className="flex items-center justify-between gap-3">
							<div className="min-w-0">
								<p className="font-mono text-xs text-muted-foreground truncate">
									{board.boardId}
								</p>
								<p className="text-xs text-muted-foreground">
									{board.scoredNodeCount}/{board.nodeCount} nodes scored ·
									updated{" "}
									<RelativeTime
										value={board.updatedAt}
										fallback={board.updatedAt}
									/>
								</p>
							</div>
							<span
								className={`text-lg font-bold ${scoreTextColor(
									board.worstScore,
								)}`}
							>
								{board.worstScore}
							</span>
						</div>

						<div className="flex flex-wrap gap-3">
							{SCORE_CATEGORIES.map((category) => (
								<div
									key={category}
									className="flex items-center gap-1.5 text-xs"
								>
									<ScoreChip value={board[category]} />
									<span className="capitalize text-muted-foreground">
										{category}
									</span>
								</div>
							))}
						</div>

						{board.flaggedPatterns.length > 0 && (
							<div className="space-y-1.5 border-t pt-3">
								<p className="text-xs font-medium flex items-center gap-1.5">
									<ShieldAlert className="h-3.5 w-3.5 text-red-500" />
									Flagged nodes ({board.flaggedPatterns.length})
								</p>
								<div className="flex flex-wrap gap-1.5">
									{board.flaggedPatterns.map((pattern, index) => (
										<Badge
											key={`${pattern.node}-${pattern.category}-${index}`}
											variant="outline"
											className="text-[11px]"
										>
											<span className="font-medium">{pattern.node}</span>
											<span className="mx-1 text-muted-foreground">
												{pattern.category}
											</span>
											<span className={scoreTextColor(pattern.score)}>
												{pattern.score}
											</span>
											{(pattern.count ?? 1) > 1 && (
												<span className="ml-1 text-muted-foreground">
													×{pattern.count}
												</span>
											)}
										</Badge>
									))}
								</div>
							</div>
						)}
					</CardContent>
				</Card>
			))}
		</div>
	);
}

function PatternsTab() {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [search, setSearch] = useState("");
	const [page, setPage] = useState(1);

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			page,
			limit: PAGE_SIZE,
		};
		if (search.trim()) params.search = search.trim();
		return params;
	}, [page, search]);

	const patterns = useQuery<ListPatternsResponse>({
		queryKey: ["admin", "governance", "patterns", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<ListPatternsResponse>(
				profile.data,
				`admin/governance/patterns?${qs}`,
			);
		},
		enabled: !!profile.data,
	});

	const totalPages = Math.max(
		1,
		Math.ceil((patterns.data?.total ?? 0) / PAGE_SIZE),
	);

	return (
		<div className="space-y-4">
			<div className="relative max-w-md">
				<Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
				<Input
					value={search}
					onChange={(event) => {
						setSearch(event.target.value);
						setPage(1);
					}}
					placeholder="Search by node or category..."
					className="pl-8"
				/>
			</div>

			<Card>
				<CardContent className="p-0">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead>Node</TableHead>
								<TableHead>Category</TableHead>
								<TableHead className="text-center">Min score</TableHead>
								<TableHead className="text-center">Apps</TableHead>
								<TableHead className="text-center">Occurrences</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{patterns.isLoading && (
								<TableRow>
									<TableCell colSpan={5} className="py-8">
										<Skeleton className="h-24 w-full" />
									</TableCell>
								</TableRow>
							)}
							{!patterns.isLoading &&
								(!patterns.data?.patterns?.length ? (
									<TableRow>
										<TableCell
											colSpan={5}
											className="text-center text-muted-foreground py-8"
										>
											No flagged patterns found.
										</TableCell>
									</TableRow>
								) : (
									patterns.data.patterns.map((pattern) => (
										<TableRow key={`${pattern.node}-${pattern.category}`}>
											<TableCell className="font-medium text-sm">
												{pattern.node}
											</TableCell>
											<TableCell>
												<Badge variant="secondary" className="capitalize">
													{pattern.category}
												</Badge>
											</TableCell>
											<TableCell className="text-center">
												<ScoreChip value={pattern.minScore} />
											</TableCell>
											<TableCell className="text-center text-sm">
												{pattern.appCount}
											</TableCell>
											<TableCell className="text-center text-sm text-muted-foreground">
												{pattern.occurrenceCount}
											</TableCell>
										</TableRow>
									))
								))}
						</TableBody>
					</Table>
				</CardContent>
			</Card>

			<Pagination
				page={page}
				totalPages={totalPages}
				total={patterns.data?.total ?? 0}
				onPrev={() => setPage((p) => Math.max(1, p - 1))}
				onNext={() => setPage((p) => p + 1)}
				hasMore={patterns.data?.hasMore ?? false}
			/>
		</div>
	);
}

function Pagination({
	page,
	totalPages,
	total,
	onPrev,
	onNext,
	hasMore,
}: {
	page: number;
	totalPages: number;
	total: number;
	onPrev: () => void;
	onNext: () => void;
	hasMore: boolean;
}) {
	if (total === 0) return null;
	return (
		<div className="flex items-center justify-between">
			<p className="text-xs text-muted-foreground">
				Page {page} of {totalPages} · {total} total
			</p>
			<div className="flex items-center gap-2">
				<Button
					variant="outline"
					size="sm"
					disabled={page <= 1}
					onClick={onPrev}
				>
					<ChevronLeft className="h-4 w-4" />
				</Button>
				<Button
					variant="outline"
					size="sm"
					disabled={!hasMore}
					onClick={onNext}
				>
					<ChevronRight className="h-4 w-4" />
				</Button>
			</div>
		</div>
	);
}
