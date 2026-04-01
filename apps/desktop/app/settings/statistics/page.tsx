"use client";

import { ResponsiveBar } from "@nivo/bar";
import { ResponsivePie } from "@nivo/pie";
import { ResponsiveTreeMap } from "@nivo/treemap";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	ScrollArea,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@tm9657/flow-like-ui";
import type { ISettingsProfile } from "@tm9657/flow-like-ui/types";
import {
	BarChart3,
	Boxes,
	ExternalLink,
	GitBranchIcon,
	HashIcon,
	Layers,
	Loader2,
	type LucideIcon,
	MessageSquare,
	RefreshCw,
	Sparkles,
	TrendingUp,
	Variable,
	Workflow,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTauriInvoke } from "../../../components/useInvoke";

interface NodeUsage {
	name: string;
	friendly_name: string;
	category: string;
	count: number;
	boards: string[];
}

interface BoardRef {
	id: string;
	name: string;
	app_id: string;
}

interface NodePattern {
	nodes: string[];
	edges: [string, string][];
	edge_count: number;
	occurrences: number;
	boards: BoardRef[];
	rarity_score: number;
	frequency_score: number;
}

interface CategoryStats {
	name: string;
	node_count: number;
	unique_nodes: number;
}

interface BoardSummary {
	id: string;
	app_id: string;
	name: string;
	node_count: number;
	connection_count: number;
	variable_count: number;
	layer_count: number;
	comment_count: number;
}

interface BoardStatistics {
	total_boards: number;
	total_nodes: number;
	total_connections: number;
	total_variables: number;
	total_layers: number;
	total_comments: number;
	avg_nodes_per_board: number;
	avg_connections_per_board: number;
	most_used_nodes: NodeUsage[];
	rare_patterns: NodePattern[];
	common_patterns: NodePattern[];
	category_stats: CategoryStats[];
	board_summaries: BoardSummary[];
}

type BoardStatisticsUpdate =
	| {
			type: "Cached";
			statistics: BoardStatistics;
	  }
	| {
			type: "BoardsLoaded";
			total_boards: number;
			total_nodes: number;
			total_connections: number;
			total_variables: number;
			total_layers: number;
			total_comments: number;
			avg_nodes_per_board: number;
			avg_connections_per_board: number;
			board_summaries: BoardSummary[];
	  }
	| {
			type: "NodeUsage";
			most_used_nodes: NodeUsage[];
			category_stats: CategoryStats[];
	  }
	| {
			type: "PatternsReady";
			rare_patterns: NodePattern[];
			common_patterns: NodePattern[];
	  }
	| {
			type: "Complete";
			statistics: BoardStatistics;
	  };

type LoadingPhase =
	| "idle"
	| "loading-boards"
	| "loading-nodes"
	| "mining-patterns"
	| "complete";

function StatCard({
	title,
	value,
	icon: Icon,
	description,
}: Readonly<{
	title: string;
	value: string | number;
	icon: LucideIcon;
	description?: string;
}>) {
	return (
		<Card>
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium">{title}</CardTitle>
				<Icon className="h-4 w-4 text-muted-foreground" />
			</CardHeader>
			<CardContent>
				<div className="text-2xl font-bold">{value}</div>
				{description && (
					<p className="text-xs text-muted-foreground">{description}</p>
				)}
			</CardContent>
		</Card>
	);
}

function BoardLink({ board }: Readonly<{ board: BoardRef }>) {
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Link
					href={`/flow?id=${board.id}&app=${board.app_id}`}
					className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-primary/10 hover:bg-primary/20 text-primary text-xs font-medium transition-colors"
				>
					{board.name}
					<ExternalLink className="h-3 w-3" />
				</Link>
			</TooltipTrigger>
			<TooltipContent>Open board</TooltipContent>
		</Tooltip>
	);
}

function NodeUsageBarChart({ nodes }: Readonly<{ nodes: NodeUsage[] }>) {
	const data = useMemo(
		() =>
			nodes.slice(0, 15).map((node) => ({
				node: node.friendly_name || node.name,
				count: node.count,
				category: node.category,
			})),
		[nodes],
	);

	return (
		<div className="h-100">
			<ResponsiveBar
				data={data}
				keys={["count"]}
				indexBy="node"
				margin={{ top: 20, right: 20, bottom: 80, left: 60 }}
				padding={0.3}
				valueScale={{ type: "linear" }}
				colors={{ scheme: "paired" }}
				borderRadius={4}
				axisBottom={{
					tickSize: 5,
					tickPadding: 5,
					tickRotation: -45,
					legend: "Node",
					legendPosition: "middle",
					legendOffset: 70,
					truncateTickAt: 20,
				}}
				axisLeft={{
					tickSize: 5,
					tickPadding: 5,
					tickRotation: 0,
					legend: "Usage Count",
					legendPosition: "middle",
					legendOffset: -50,
				}}
				labelSkipWidth={12}
				labelSkipHeight={12}
				labelTextColor="hsl(var(--primary-foreground))"
				theme={{
					text: { fill: "hsl(var(--foreground))" },
					axis: {
						ticks: { text: { fill: "hsl(var(--muted-foreground))" } },
						legend: { text: { fill: "hsl(var(--foreground))" } },
					},
					grid: { line: { stroke: "hsl(var(--border))" } },
					labels: { text: { fill: "hsl(var(--foreground))" } },
					tooltip: {
						container: {
							background: "hsl(var(--popover))",
							color: "hsl(var(--popover-foreground))",
							borderRadius: "8px",
							boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
						},
					},
				}}
			/>
		</div>
	);
}

function CategoryPieChart({
	categories,
}: Readonly<{ categories: CategoryStats[] }>) {
	const data = useMemo(
		() =>
			categories
				.filter((cat) => cat.node_count > 0)
				.map((cat) => ({
					id: cat.name || "Uncategorized",
					label: cat.name || "Uncategorized",
					value: cat.node_count,
					unique: cat.unique_nodes,
				})),
		[categories],
	);

	return (
		<div className="h-100">
			<ResponsivePie
				data={data}
				margin={{ top: 40, right: 80, bottom: 40, left: 80 }}
				innerRadius={0.5}
				padAngle={0.7}
				cornerRadius={3}
				activeOuterRadiusOffset={8}
				colors={{ scheme: "nivo" }}
				borderWidth={1}
				borderColor={{ from: "color", modifiers: [["darker", 0.2]] }}
				arcLinkLabelsSkipAngle={10}
				arcLinkLabelsTextColor="hsl(var(--foreground))"
				arcLinkLabelsThickness={2}
				arcLinkLabelsColor={{ from: "color" }}
				arcLabelsSkipAngle={10}
				arcLabelsTextColor={{ from: "color", modifiers: [["darker", 2]] }}
				theme={{
					text: { fill: "hsl(var(--foreground))" },
					labels: { text: { fill: "hsl(var(--foreground))" } },
					tooltip: {
						container: {
							background: "hsl(var(--popover))",
							color: "hsl(var(--popover-foreground))",
							borderRadius: "8px",
							boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
						},
					},
				}}
				tooltip={({ datum }) => (
					<div className="px-3 py-2 bg-popover text-popover-foreground rounded-lg shadow-lg border">
						<strong>{datum.label}</strong>
						<div className="text-sm text-muted-foreground">
							{datum.value} nodes ({datum.data.unique} unique)
						</div>
					</div>
				)}
			/>
		</div>
	);
}

function BoardComplexityTreeMap({
	boards,
}: Readonly<{ boards: BoardSummary[] }>) {
	const data = useMemo(
		() => ({
			name: "Boards",
			children: boards.slice(0, 30).map((board) => ({
				name: board.name,
				value: board.node_count + board.connection_count,
				nodeCount: board.node_count,
				connectionCount: board.connection_count,
				id: board.id,
				appId: board.app_id,
			})),
		}),
		[boards],
	);

	return (
		<div className="h-100">
			<ResponsiveTreeMap
				data={data}
				identity="name"
				value="value"
				margin={{ top: 10, right: 10, bottom: 10, left: 10 }}
				labelSkipSize={24}
				label={(node) => `${node.id}`}
				parentLabelPosition="left"
				parentLabelTextColor={{ from: "color", modifiers: [["darker", 2]] }}
				colors={{ scheme: "blues" }}
				borderColor={{ from: "color", modifiers: [["darker", 0.3]] }}
				labelTextColor={{ from: "color", modifiers: [["darker", 2]] }}
				theme={{
					text: { fill: "hsl(var(--foreground))" },
					labels: { text: { fill: "hsl(var(--foreground))" } },
					tooltip: {
						container: {
							background: "hsl(var(--popover))",
							color: "hsl(var(--popover-foreground))",
							borderRadius: "8px",
							boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
						},
					},
				}}
				tooltip={({ node }) => (
					<div className="px-3 py-2 bg-popover text-popover-foreground rounded-lg shadow-lg border">
						<strong>{node.id}</strong>
						<div className="text-sm text-muted-foreground">
							Complexity: {node.value}
						</div>
					</div>
				)}
			/>
		</div>
	);
}

function PatternSchematic({ pattern }: Readonly<{ pattern: NodePattern }>) {
	const nodePositions = useMemo(() => {
		const nodeCount = pattern.nodes.length;
		const width = 100;
		const height = 100;
		const padding = 12;
		const nodeRadius = 4;

		if (nodeCount === 1) {
			return [{ x: width / 2, y: height / 2 }];
		}

		if (nodeCount === 2) {
			return [
				{ x: padding + nodeRadius, y: height / 2 },
				{ x: width - padding - nodeRadius, y: height / 2 },
			];
		}

		const radius = Math.min(width, height) / 2 - padding - nodeRadius;
		const centerX = width / 2;
		const centerY = height / 2;

		return pattern.nodes.map((_, i) => {
			const angle = (2 * Math.PI * i) / nodeCount - Math.PI / 2;
			return {
				x: centerX + radius * Math.cos(angle),
				y: centerY + radius * Math.sin(angle),
			};
		});
	}, [pattern.nodes]);

	const edgeLines = useMemo(() => {
		const nodeIndexMap: Record<string, number[]> = {};
		pattern.nodes.forEach((name, idx) => {
			if (!nodeIndexMap[name]) nodeIndexMap[name] = [];
			nodeIndexMap[name].push(idx);
		});

		const lines: { x1: number; y1: number; x2: number; y2: number }[] = [];
		const usedEdges = new Set<string>();

		for (const [source, target] of pattern.edges) {
			const sourceIndices = nodeIndexMap[source] || [];
			const targetIndices = nodeIndexMap[target] || [];

			for (const si of sourceIndices) {
				for (const ti of targetIndices) {
					if (si !== ti) {
						const edgeKey = si < ti ? `${si}-${ti}` : `${ti}-${si}`;
						if (!usedEdges.has(edgeKey)) {
							usedEdges.add(edgeKey);
							const from = nodePositions[si];
							const to = nodePositions[ti];
							if (from && to) {
								lines.push({ x1: from.x, y1: from.y, x2: to.x, y2: to.y });
							}
							break;
						}
					}
				}
				if (usedEdges.size >= pattern.edge_count) break;
			}
			if (usedEdges.size >= pattern.edge_count) break;
		}

		return lines;
	}, [pattern, nodePositions]);

	return (
		<div className="flex gap-3 p-3 rounded-lg border bg-muted/20">
			<div className="shrink-0">
				<svg
					viewBox="0 0 100 100"
					className="w-20 h-20 rounded-md bg-background/60 border"
				>
					{edgeLines.map((line, i) => (
						<line
							key={`edge-${i}`}
							x1={line.x1}
							y1={line.y1}
							x2={line.x2}
							y2={line.y2}
							className="stroke-muted-foreground/60"
							strokeWidth="1.5"
						/>
					))}
					{nodePositions.map((pos, i) => (
						<circle
							key={`node-${i}`}
							cx={pos.x}
							cy={pos.y}
							r={4}
							className="fill-primary stroke-primary-foreground"
							strokeWidth="1"
						/>
					))}
				</svg>
			</div>
			<div className="flex-1 min-w-0">
				<div className="flex flex-wrap gap-1.5">
					{pattern.nodes.map((nodeName, i) => (
						<Badge
							key={`${nodeName}-${i}`}
							variant="secondary"
							className="text-xs font-mono truncate max-w-32"
						>
							{nodeName}
						</Badge>
					))}
				</div>
			</div>
		</div>
	);
}

function PatternCard({
	pattern,
	index,
	scoreType,
}: Readonly<{
	pattern: NodePattern;
	index: number;
	scoreType: "rarity" | "frequency";
}>) {
	const score =
		scoreType === "rarity" ? pattern.rarity_score : pattern.frequency_score;
	const scoreLabel = scoreType === "rarity" ? "Rarity" : "Frequency";

	return (
		<Card className="overflow-hidden">
			<CardHeader className="pb-2">
				<div className="flex items-center justify-between">
					<div className="flex items-center gap-2">
						<span className="flex items-center justify-center w-6 h-6 rounded-full bg-primary/10 text-primary text-xs font-bold">
							{index + 1}
						</span>
						<Badge variant="outline">{pattern.nodes.length} nodes</Badge>
						<Badge variant="secondary">{pattern.edge_count} edges</Badge>
					</div>
					<Badge className="bg-linear-to-r from-primary/80 to-primary">
						{scoreLabel}: {score.toFixed(1)}
					</Badge>
				</div>
			</CardHeader>
			<CardContent className="space-y-3">
				<PatternSchematic pattern={pattern} />
				<div className="pt-2 border-t">
					<div className="flex items-center gap-1 flex-wrap">
						<span className="text-xs text-muted-foreground mr-1">
							{pattern.occurrences}× in:
						</span>
						{pattern.boards.slice(0, 5).map((board) => (
							<BoardLink key={board.id} board={board} />
						))}
						{pattern.boards.length > 5 && (
							<span className="text-xs text-muted-foreground">
								+{pattern.boards.length - 5} more
							</span>
						)}
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

function BoardsTable({ boards }: Readonly<{ boards: BoardSummary[] }>) {
	return (
		<div className="rounded-md border">
			<table className="w-full">
				<thead>
					<tr className="border-b bg-muted/50">
						<th className="h-10 px-4 text-left text-sm font-medium">Board</th>
						<th className="h-10 px-4 text-right text-sm font-medium">Nodes</th>
						<th className="h-10 px-4 text-right text-sm font-medium hidden sm:table-cell">
							Connections
						</th>
						<th className="h-10 px-4 text-right text-sm font-medium hidden md:table-cell">
							Variables
						</th>
						<th className="h-10 px-4 text-right text-sm font-medium hidden lg:table-cell">
							Layers
						</th>
						<th className="h-10 px-4 text-right text-sm font-medium hidden lg:table-cell">
							Comments
						</th>
						<th className="h-10 px-4 text-right text-sm font-medium">Action</th>
					</tr>
				</thead>
				<tbody>
					{boards.map((board) => (
						<tr key={board.id} className="border-b hover:bg-muted/30">
							<td className="p-4 text-sm font-medium truncate max-w-50">
								{board.name}
							</td>
							<td className="p-4 text-sm text-right">{board.node_count}</td>
							<td className="p-4 text-sm text-right hidden sm:table-cell">
								{board.connection_count}
							</td>
							<td className="p-4 text-sm text-right hidden md:table-cell">
								{board.variable_count}
							</td>
							<td className="p-4 text-sm text-right hidden lg:table-cell">
								{board.layer_count}
							</td>
							<td className="p-4 text-sm text-right hidden lg:table-cell">
								{board.comment_count}
							</td>
							<td className="p-4 text-sm text-right">
								<Link
									href={`/flow?id=${board.id}&app=${board.app_id}`}
									className="inline-flex items-center gap-1 text-primary hover:underline"
								>
									Open <ExternalLink className="h-3 w-3" />
								</Link>
							</td>
						</tr>
					))}
				</tbody>
			</table>
		</div>
	);
}

function LoadingSkeleton() {
	return (
		<div className="space-y-6">
			<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
				{["stat-1", "stat-2", "stat-3", "stat-4"].map((id) => (
					<Card key={id}>
						<CardHeader className="pb-2">
							<Skeleton className="h-4 w-24" />
						</CardHeader>
						<CardContent>
							<Skeleton className="h-8 w-16" />
						</CardContent>
					</Card>
				))}
			</div>
			<Card>
				<CardHeader>
					<Skeleton className="h-6 w-32" />
				</CardHeader>
				<CardContent className="space-y-3">
					{["row-1", "row-2", "row-3", "row-4", "row-5"].map((id) => (
						<Skeleton key={id} className="h-8 w-full" />
					))}
				</CardContent>
			</Card>
		</div>
	);
}

function EmptyState({ onRefresh }: Readonly<{ onRefresh: () => void }>) {
	return (
		<Card>
			<CardContent className="flex flex-col items-center justify-center py-12 text-center">
				<BarChart3 className="h-12 w-12 text-muted-foreground mb-4" />
				<h3 className="text-lg font-semibold mb-2">No boards found</h3>
				<p className="text-sm text-muted-foreground mb-4 max-w-sm">
					Create some boards in your apps to see statistics about your node
					usage patterns and workflows.
				</p>
				<Button onClick={onRefresh} variant="outline" className="gap-2">
					<RefreshCw className="h-4 w-4" />
					Refresh
				</Button>
			</CardContent>
		</Card>
	);
}

export default function StatisticsPage() {
	const currentProfile = useTauriInvoke<ISettingsProfile>(
		"get_settings_profile",
		{},
	);

	const profileId = currentProfile.data?.hub_profile?.id;

	const [statistics, setStatistics] = useState<BoardStatistics | null>(null);
	const [phase, setPhase] = useState<LoadingPhase>("idle");
	const [isStale, setIsStale] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const fetchingRef = useRef(false);

	const streamStatistics = useCallback(async () => {
		if (fetchingRef.current) return;
		fetchingRef.current = true;
		setError(null);
		setPhase("loading-boards");

		try {
			const channel = new Channel<BoardStatisticsUpdate>();

			channel.onmessage = (update: BoardStatisticsUpdate) => {
				switch (update.type) {
					case "Cached":
						setStatistics(update.statistics);
						setIsStale(true);
						break;
					case "BoardsLoaded":
						setStatistics((prev) => ({
							...(prev ?? {
								most_used_nodes: [],
								rare_patterns: [],
								common_patterns: [],
								category_stats: [],
							}),
							total_boards: update.total_boards,
							total_nodes: update.total_nodes,
							total_connections: update.total_connections,
							total_variables: update.total_variables,
							total_layers: update.total_layers,
							total_comments: update.total_comments,
							avg_nodes_per_board: update.avg_nodes_per_board,
							avg_connections_per_board: update.avg_connections_per_board,
							board_summaries: update.board_summaries,
						}));
						setPhase("loading-nodes");
						break;
					case "NodeUsage":
						setStatistics((prev) =>
							prev
								? {
										...prev,
										most_used_nodes: update.most_used_nodes,
										category_stats: update.category_stats,
									}
								: null,
						);
						setPhase("mining-patterns");
						break;
					case "PatternsReady":
						setStatistics((prev) =>
							prev
								? {
										...prev,
										rare_patterns: update.rare_patterns,
										common_patterns: update.common_patterns,
									}
								: null,
						);
						break;
					case "Complete":
						setStatistics(update.statistics);
						setPhase("complete");
						setIsStale(false);
						break;
				}
			};

			await invoke("get_board_statistics", { channel });
		} catch (e) {
			setError(e instanceof Error ? e.message : String(e));
			setPhase("idle");
		} finally {
			fetchingRef.current = false;
		}
	}, []);

	useEffect(() => {
		if (profileId) {
			streamStatistics();
		}
	}, [profileId, streamStatistics]);

	const isComputing = phase !== "idle" && phase !== "complete";

	if (
		!statistics &&
		(phase === "loading-boards" || currentProfile.isLoading)
	) {
		return (
			<div className="container mx-auto p-6 max-w-7xl">
				<div className="flex items-center justify-between mb-6">
					<div>
						<h1 className="text-2xl font-bold">Board Statistics</h1>
						<p className="text-muted-foreground">
							Analyzing your local boards...
						</p>
					</div>
				</div>
				<LoadingSkeleton />
			</div>
		);
	}

	if (error && !statistics) {
		return (
			<div className="container mx-auto p-6 max-w-7xl">
				<Card>
					<CardContent className="flex flex-col items-center justify-center py-12 text-center">
						<h3 className="text-lg font-semibold mb-2 text-destructive">
							Error loading statistics
						</h3>
						<p className="text-sm text-muted-foreground mb-4">{error}</p>
						<Button
							onClick={streamStatistics}
							variant="outline"
							className="gap-2"
						>
							<RefreshCw className="h-4 w-4" />
							Try Again
						</Button>
					</CardContent>
				</Card>
			</div>
		);
	}

	if (!statistics || statistics.total_boards === 0) {
		return (
			<div className="container mx-auto p-6 max-w-7xl">
				<div className="flex items-center justify-between mb-6">
					<div>
						<h1 className="text-2xl font-bold">Board Statistics</h1>
						<p className="text-muted-foreground">
							Insights from your local boards
						</p>
					</div>
				</div>
				<EmptyState onRefresh={streamStatistics} />
			</div>
		);
	}

	return (
		<ScrollArea className="h-full w-full">
			<div className="container mx-auto p-6 max-w-7xl">
				<div className="flex items-center justify-between mb-6">
					<div>
						<h1 className="text-2xl font-bold flex items-center gap-2">
							<Sparkles className="h-6 w-6 text-primary" />
							Board Statistics
							{isComputing && (
								<Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
							)}
							{isStale && !isComputing && (
								<Badge variant="outline" className="text-xs font-normal ml-1">
									cached
								</Badge>
							)}
						</h1>
						<p className="text-muted-foreground">
							Insights from {statistics.total_boards} local board
							{statistics.total_boards !== 1 ? "s" : ""} in your current profile
							{isComputing && (
								<span className="ml-2 text-xs">
									{phase === "loading-boards" && "— loading boards..."}
									{phase === "loading-nodes" && "— analyzing node usage..."}
									{phase === "mining-patterns" && "— mining patterns..."}
								</span>
							)}
						</p>
					</div>
					<Button
						onClick={streamStatistics}
						variant="outline"
						className="gap-2"
						disabled={isComputing}
					>
						<RefreshCw
							className={`h-4 w-4 ${isComputing ? "animate-spin" : ""}`}
						/>
						Refresh
					</Button>
				</div>

				<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4 mb-6">
					<StatCard
						title="Total Boards"
						value={statistics.total_boards}
						icon={Workflow}
						description="Boards in current profile"
					/>
					<StatCard
						title="Total Nodes"
						value={statistics.total_nodes}
						icon={Boxes}
						description={`~${statistics.avg_nodes_per_board.toFixed(1)} per board`}
					/>
					<StatCard
						title="Total Connections"
						value={statistics.total_connections}
						icon={GitBranchIcon}
						description={`~${statistics.avg_connections_per_board.toFixed(1)} per board`}
					/>
					<StatCard
						title="Total Variables"
						value={statistics.total_variables}
						icon={Variable}
					/>
				</div>

				<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4 mb-6">
					<StatCard
						title="Total Layers"
						value={statistics.total_layers}
						icon={Layers}
					/>
					<StatCard
						title="Total Comments"
						value={statistics.total_comments}
						icon={MessageSquare}
					/>
					<StatCard
						title="Node Categories"
						value={statistics.category_stats.length}
						icon={HashIcon}
					/>
					<StatCard
						title="Unique Nodes"
						value={statistics.most_used_nodes.length}
						icon={TrendingUp}
						description="Different node types used"
					/>
				</div>

				<Tabs defaultValue="overview" className="space-y-4">
					<TabsList className="grid w-full grid-cols-5">
						<TabsTrigger value="overview">Overview</TabsTrigger>
						<TabsTrigger value="nodes">Top Nodes</TabsTrigger>
						<TabsTrigger value="patterns">
							Patterns
							{phase === "mining-patterns" && (
								<Loader2 className="h-3 w-3 animate-spin ml-1" />
							)}
						</TabsTrigger>
						<TabsTrigger value="categories">Categories</TabsTrigger>
						<TabsTrigger value="boards">Boards</TabsTrigger>
					</TabsList>

					<TabsContent value="overview" className="space-y-4">
						<div className="grid gap-4 md:grid-cols-2">
							<Card>
								<CardHeader>
									<CardTitle>Top Nodes by Usage</CardTitle>
									<CardDescription>
										Most frequently used nodes across all boards
									</CardDescription>
								</CardHeader>
								<CardContent>
									{statistics.most_used_nodes.length > 0 ? (
										<NodeUsageBarChart nodes={statistics.most_used_nodes} />
									) : (
										<div className="h-100 flex items-center justify-center">
											<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
										</div>
									)}
								</CardContent>
							</Card>
							<Card>
								<CardHeader>
									<CardTitle>Category Distribution</CardTitle>
									<CardDescription>
										Node usage breakdown by category
									</CardDescription>
								</CardHeader>
								<CardContent>
									{statistics.category_stats.length > 0 ? (
										<CategoryPieChart categories={statistics.category_stats} />
									) : (
										<div className="h-100 flex items-center justify-center">
											<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
										</div>
									)}
								</CardContent>
							</Card>
						</div>
						<Card>
							<CardHeader>
								<CardTitle>Board Complexity Map</CardTitle>
								<CardDescription>
									Board sizes visualized by node and connection count
								</CardDescription>
							</CardHeader>
							<CardContent>
								{statistics.board_summaries.length > 0 ? (
									<BoardComplexityTreeMap boards={statistics.board_summaries} />
								) : (
									<div className="h-100 flex items-center justify-center">
										<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
									</div>
								)}
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="nodes">
						<Card>
							<CardHeader>
								<CardTitle>Most Used Nodes</CardTitle>
								<CardDescription>
									The nodes you use most frequently across all boards
								</CardDescription>
							</CardHeader>
							<CardContent>
								{statistics.most_used_nodes.length > 0 ? (
									<>
										<NodeUsageBarChart nodes={statistics.most_used_nodes} />
										<div className="mt-6 grid gap-2 md:grid-cols-2 lg:grid-cols-3">
											{statistics.most_used_nodes
												.slice(0, 12)
												.map((node, i) => (
													<div
														key={node.name}
														className="flex items-center gap-3 p-3 rounded-lg border bg-muted/30"
													>
														<span className="flex items-center justify-center w-8 h-8 rounded-full bg-primary/10 text-primary text-sm font-bold">
															{i + 1}
														</span>
														<div className="flex-1 min-w-0">
															<p className="font-medium truncate">
																{node.friendly_name || node.name}
															</p>
															<p className="text-xs text-muted-foreground">
																{node.count}× in {node.boards.length} board
																{node.boards.length !== 1 ? "s" : ""}
															</p>
														</div>
														<Badge
															variant="secondary"
															className="text-xs shrink-0"
														>
															{node.category}
														</Badge>
													</div>
												))}
										</div>
									</>
								) : (
									<div className="flex items-center justify-center py-12">
										<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
										<span className="ml-2 text-muted-foreground">
											Analyzing node usage...
										</span>
									</div>
								)}
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="patterns">
						<div className="grid gap-4 md:grid-cols-2">
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<TrendingUp className="h-5 w-5 text-primary" />
										Common Patterns
										{phase === "mining-patterns" && (
											<Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
										)}
									</CardTitle>
									<CardDescription>
										Frequently used node combinations across your boards
									</CardDescription>
								</CardHeader>
								<CardContent>
									{statistics.common_patterns.length > 0 ? (
										<ScrollArea className="h-150 pr-4">
											<div className="space-y-3">
												{statistics.common_patterns.map((pattern, i) => (
													<PatternCard
														key={pattern.nodes.join("-")}
														pattern={pattern}
														index={i}
														scoreType="frequency"
													/>
												))}
											</div>
										</ScrollArea>
									) : phase === "mining-patterns" ? (
										<div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
											<Loader2 className="h-8 w-8 animate-spin mb-3" />
											<p className="text-sm">Mining subgraph patterns...</p>
											<p className="text-xs mt-1">
												This may take a moment for large workspaces
											</p>
										</div>
									) : (
										<div className="text-center py-8 text-muted-foreground">
											No common patterns found.
										</div>
									)}
								</CardContent>
							</Card>
							<Card>
								<CardHeader>
									<CardTitle className="flex items-center gap-2">
										<Sparkles className="h-5 w-5 text-amber-500" />
										Rare Patterns
										{phase === "mining-patterns" && (
											<Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
										)}
									</CardTitle>
									<CardDescription>
										Large node structures that appear in fewer boards
									</CardDescription>
								</CardHeader>
								<CardContent>
									{statistics.rare_patterns.length > 0 ? (
										<ScrollArea className="h-150 pr-4">
											<div className="space-y-3">
												{statistics.rare_patterns.map((pattern, i) => (
													<PatternCard
														key={pattern.nodes.join("-")}
														pattern={pattern}
														index={i}
														scoreType="rarity"
													/>
												))}
											</div>
										</ScrollArea>
									) : phase === "mining-patterns" ? (
										<div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
											<Loader2 className="h-8 w-8 animate-spin mb-3" />
											<p className="text-sm">Mining subgraph patterns...</p>
											<p className="text-xs mt-1">
												This may take a moment for large workspaces
											</p>
										</div>
									) : (
										<div className="text-center py-8 text-muted-foreground">
											No rare patterns found.
										</div>
									)}
								</CardContent>
							</Card>
						</div>
					</TabsContent>

					<TabsContent value="categories">
						<Card>
							<CardHeader>
								<CardTitle>Node Categories</CardTitle>
								<CardDescription>
									Distribution of nodes by category
								</CardDescription>
							</CardHeader>
							<CardContent>
								{statistics.category_stats.length > 0 ? (
									<div className="grid gap-4 md:grid-cols-2">
										<CategoryPieChart categories={statistics.category_stats} />
										<div className="space-y-3">
											{[...statistics.category_stats]
												.sort((a, b) => b.node_count - a.node_count)
												.map((cat) => (
													<div
														key={cat.name}
														className="flex items-center justify-between p-3 rounded-lg border bg-muted/30"
													>
														<div>
															<p className="font-medium">
																{cat.name || "Uncategorized"}
															</p>
															<p className="text-xs text-muted-foreground">
																{cat.unique_nodes} unique node types
															</p>
														</div>
														<Badge variant="secondary">
															{cat.node_count}
														</Badge>
													</div>
												))}
										</div>
									</div>
								) : (
									<div className="flex items-center justify-center py-12">
										<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
										<span className="ml-2 text-muted-foreground">
											Loading categories...
										</span>
									</div>
								)}
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="boards">
						<Card>
							<CardHeader>
								<CardTitle>Board Details</CardTitle>
								<CardDescription>
									Overview of all boards sorted by complexity
								</CardDescription>
							</CardHeader>
							<CardContent>
								{statistics.board_summaries.length > 0 ? (
									<ScrollArea className="h-125">
										<BoardsTable boards={statistics.board_summaries} />
									</ScrollArea>
								) : (
									<div className="flex items-center justify-center py-12">
										<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
										<span className="ml-2 text-muted-foreground">
											Loading boards...
										</span>
									</div>
								)}
							</CardContent>
						</Card>
					</TabsContent>
				</Tabs>
			</div>
		</ScrollArea>
	);
}
