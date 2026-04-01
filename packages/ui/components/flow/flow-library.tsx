"use client";

import type { UseQueryResult } from "@tanstack/react-query";
import {
	AlertTriangle,
	Calendar,
	Cloud,
	Cpu,
	DollarSign,
	ExternalLink,
	FileText,
	LockKeyhole,
	Monitor,
	PlusCircleIcon,
	Shield,
	ShieldAlert,
	Shuffle,
	SquareMousePointerIcon,
	Trash2,
	VariableIcon,
	WorkflowIcon,
} from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { useMemo } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { cn, formatRelativeTime } from "../../lib";
import { type IBoard, IExecutionMode } from "../../lib/schema/flow/board";
import { useBackend } from "../../state/backend-state";
import type { IApp } from "../../types";
import { Badge } from "../ui/badge";
import { BubbleActions } from "../ui/bubble-actions";
import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "../ui/dialog";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../ui/hover-card";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Skeleton } from "../ui/skeleton";
import { Textarea } from "../ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

export interface FlowLibraryBoardCreationState {
	open: boolean;
	name: string;
	description: string;
}

interface FlowLibraryHeaderProps {
	boardCreation: FlowLibraryBoardCreationState;
	setBoardCreation: React.Dispatch<
		React.SetStateAction<FlowLibraryBoardCreationState>
	>;
	onCreateBoard: () => Promise<void>;
}

interface FlowLibraryBoardsSectionProps {
	boards: UseQueryResult<IBoard[]>;
	app?: IApp;
	boardCreation: FlowLibraryBoardCreationState;
	setBoardCreation: React.Dispatch<
		React.SetStateAction<FlowLibraryBoardCreationState>
	>;
	onOpenBoard: (boardId: string) => Promise<void>;
	onDeleteBoard: (boardId: string) => Promise<void>;
	boardHref?: (boardId: string) => string;
}

interface FlowLibraryBoardCardProps {
	board: IBoard;
	app: IApp;
	onOpenBoard: (boardId: string) => Promise<void>;
	onDeleteBoard: (boardId: string) => Promise<void>;
	href?: string;
}

interface AggregatedScores {
	security: number;
	privacy: number;
	performance: number;
	governance: number;
	reliability: number;
	cost: number;
}

interface ScoreConfig {
	key: keyof AggregatedScores;
	label: string;
	icon: ReactNode;
}

const SCORE_CONFIGS: ScoreConfig[] = [
	{
		key: "security",
		label: "Security",
		icon: <ShieldAlert className="h-3 w-3" />,
	},
	{
		key: "privacy",
		label: "Privacy",
		icon: <LockKeyhole className="h-3 w-3" />,
	},
	{
		key: "governance",
		label: "Governance",
		icon: <Shield className="h-3 w-3" />,
	},
	{
		key: "performance",
		label: "Performance",
		icon: <Cpu className="h-3 w-3" />,
	},
	{
		key: "reliability",
		label: "Reliability",
		icon: <AlertTriangle className="h-3 w-3" />,
	},
	{ key: "cost", label: "Cost", icon: <DollarSign className="h-3 w-3" /> },
];

function getScoreColor(score: number): string {
	if (score >= 8) return "bg-emerald-500";
	if (score >= 6) return "bg-lime-500";
	if (score >= 4) return "bg-amber-500";
	if (score >= 2) return "bg-orange-500";
	return "bg-red-500";
}

function getOverallHealthColor(avgScore: number): string {
	if (avgScore >= 8) return "text-emerald-500";
	if (avgScore >= 6) return "text-lime-500";
	if (avgScore >= 4) return "text-amber-500";
	return "text-red-500";
}

function FlowLibraryScoreBar({ scores }: { scores: AggregatedScores }) {
	const avgScore =
		Object.values(scores).reduce((a, b) => a + b, 0) /
		Object.values(scores).length;

	return (
		<HoverCard openDelay={100} closeDelay={100}>
			<HoverCardTrigger asChild>
				<div className="flex items-center gap-1.5 cursor-pointer rounded-md px-1.5 py-0.5 hover:bg-muted/50 transition-colors">
					<Shield
						className={cn("h-3.5 w-3.5", getOverallHealthColor(avgScore))}
					/>
					<span
						className={cn(
							"text-xs font-semibold tabular-nums",
							getOverallHealthColor(avgScore),
						)}
					>
						{avgScore.toFixed(1)}
					</span>
					<div className="hidden sm:flex gap-0.5">
						{SCORE_CONFIGS.map((config) => (
							<div
								key={config.key}
								className={cn(
									"w-1.5 h-4 rounded-sm transition-all",
									getScoreColor(scores[config.key]),
								)}
							/>
						))}
					</div>
				</div>
			</HoverCardTrigger>
			<HoverCardContent side="left" align="start" className="w-64 p-0">
				<div className="px-3 py-2 bg-muted/50 border-b">
					<p className="text-xs font-semibold">Quality Overview</p>
					<p className="text-xs text-muted-foreground">
						Minimum scores across all nodes
					</p>
				</div>
				<div className="p-3 space-y-2.5">
					{SCORE_CONFIGS.map((config) => {
						const score = scores[config.key];
						return (
							<div key={config.key} className="flex items-center gap-2">
								<span className="text-muted-foreground">{config.icon}</span>
								<span className="text-xs w-20">{config.label}</span>
								<div className="flex-1 h-2 bg-muted rounded-full overflow-hidden">
									<div
										className={cn(
											"h-full rounded-full transition-all",
											getScoreColor(score),
										)}
										style={{ width: `${(score / 10) * 100}%` }}
									/>
								</div>
								<span className="text-xs tabular-nums w-6 text-right font-medium">
									{score.toFixed(0)}
								</span>
							</div>
						);
					})}
				</div>
			</HoverCardContent>
		</HoverCard>
	);
}

export function FlowLibraryHeader({
	boardCreation,
	setBoardCreation,
	onCreateBoard,
}: Readonly<FlowLibraryHeaderProps>) {
	return (
		<div className="flex flex-col space-y-6 shrink-0">
			<div className="flex items-center justify-between">
				<div className="space-y-2">
					<h1 className="text-4xl font-bold tracking-tight bg-linear-to-r from-foreground to-foreground/70 bg-clip-text text-transparent">
						Project Flows
					</h1>
					<p className="text-muted-foreground text-lg">
						Manage and organize your application workflows
					</p>
				</div>
				<FlowLibraryCreateDialog
					boardCreation={boardCreation}
					setBoardCreation={setBoardCreation}
					onCreateBoard={onCreateBoard}
				/>
			</div>
		</div>
	);
}

export function FlowLibraryCreateDialog({
	boardCreation,
	setBoardCreation,
	onCreateBoard,
}: Readonly<FlowLibraryHeaderProps>) {
	return (
		<Dialog
			open={boardCreation.open}
			onOpenChange={(open) => setBoardCreation({ ...boardCreation, open })}
		>
			<DialogTrigger asChild>
				<Button
					size="lg"
					className="gap-2 shadow-lg hover:shadow-xl transition-all duration-200 bg-linear-to-r from-primary to-primary/80"
				>
					<PlusCircleIcon className="h-5 w-5" />
					Create New Flow
				</Button>
			</DialogTrigger>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle className="text-2xl">Create New Flow</DialogTitle>
					<DialogDescription className="text-base">
						Design a new flow for your application
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="name" className="text-sm font-medium">
							Flow Name
						</Label>
						<Input
							value={boardCreation.name}
							id="name"
							placeholder="Enter flow name..."
							className="h-11"
							onChange={(event) => {
								setBoardCreation((old) => ({
									...old,
									name: event.target.value,
								}));
							}}
						/>
					</div>
					<div className="space-y-2">
						<Label htmlFor="description" className="text-sm font-medium">
							Description
						</Label>
						<Textarea
							value={boardCreation.description}
							id="description"
							placeholder="Describe the purpose of this flow..."
							className="min-h-[100px] resize-none"
							onChange={(event) => {
								setBoardCreation((old) => ({
									...old,
									description: event.target.value,
								}));
							}}
						/>
					</div>
				</div>
				<DialogFooter className="gap-3">
					<Button
						variant="outline"
						onClick={() => setBoardCreation({ ...boardCreation, open: false })}
					>
						Cancel
					</Button>
					<Button
						onClick={onCreateBoard}
						className="bg-linear-to-r from-primary to-primary/80"
					>
						Create Board
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function FlowLibraryBoardsSection({
	boards,
	app,
	boardCreation,
	setBoardCreation,
	onOpenBoard,
	onDeleteBoard,
	boardHref,
}: Readonly<FlowLibraryBoardsSectionProps>) {
	if (boards.isLoading) {
		return (
			<div className="space-y-6 flex flex-1 flex-col grow max-h-full h-full overflow-y-auto md:overflow-visible overflow-x-hidden">
				<FlowLibraryBoardsSkeleton />
			</div>
		);
	}

	if (boards.data?.length === 0) {
		return (
			<div className="space-y-6 flex flex-1 flex-col grow max-h-full h-full overflow-y-auto md:overflow-visible overflow-x-hidden">
				<FlowLibraryEmptyBoards
					setBoardCreation={setBoardCreation}
					boardCreation={boardCreation}
				/>
			</div>
		);
	}

	const uniqueBoards = Array.from(
		new Map((boards.data ?? []).map((board) => [board.id, board])).values(),
	);

	return (
		<div className="space-y-6 flex flex-1 flex-col grow max-h-full h-full overflow-y-auto md:overflow-visible overflow-x-hidden">
			<div className="grid grid-cols-1 lg:grid-cols-2 xl:grid-cols-3 gap-6 pb-6">
				{app &&
					uniqueBoards.map((board) => (
						<FlowLibraryBoardCard
							key={board.id}
							board={board}
							app={app}
							onOpenBoard={onOpenBoard}
							onDeleteBoard={onDeleteBoard}
							href={boardHref?.(board.id)}
						/>
					))}
			</div>
		</div>
	);
}

export function FlowLibraryBoardsSkeleton() {
	return (
		<div className="grid grid-cols-1 lg:grid-cols-2 xl:grid-cols-3 gap-6 pb-6">
			{[...Array(6)].map((_, index) => (
				<Card key={`${index}-skeleton`} className="border-0 shadow-md">
					<CardHeader>
						<Skeleton className="h-6 w-3/4" />
						<Skeleton className="h-4 w-1/2" />
					</CardHeader>
					<CardContent>
						<Skeleton className="h-20 w-full" />
					</CardContent>
				</Card>
			))}
		</div>
	);
}

export function FlowLibraryEmptyBoards({
	setBoardCreation,
	boardCreation,
}: Readonly<{
	setBoardCreation: React.Dispatch<
		React.SetStateAction<FlowLibraryBoardCreationState>
	>;
	boardCreation: FlowLibraryBoardCreationState;
}>) {
	return (
		<Card className="border-0 shadow-md bg-linear-to-br from-muted/50 to-muted/20">
			<CardContent className="flex flex-col items-center justify-center py-16">
				<WorkflowIcon className="h-16 w-16 text-muted-foreground/50 mb-4" />
				<h3 className="text-xl font-semibold mb-2">No boards yet</h3>
				<p className="text-muted-foreground text-center mb-6 max-w-md">
					Create your first flow to start building amazing automations
				</p>
				<Button
					onClick={() => setBoardCreation({ ...boardCreation, open: true })}
					className="gap-2"
				>
					<PlusCircleIcon className="h-4 w-4" />
					Create Your First Board
				</Button>
			</CardContent>
		</Card>
	);
}

export function FlowLibraryBoardCard({
	board,
	app,
	onOpenBoard,
	onDeleteBoard,
	href,
}: Readonly<FlowLibraryBoardCardProps>) {
	const backend = useBackend();
	const pages = useInvoke(backend.pageState.getPages, backend.pageState, [
		app.id,
		board.id,
	]);

	const aggregatedScores = useMemo((): AggregatedScores | null => {
		const nodes = Object.values(board.nodes);
		const nodesWithScores = nodes.filter((node) => node.scores);
		if (nodesWithScores.length === 0) return null;

		return {
			security: Math.min(
				...nodesWithScores.map((node) => node.scores?.security ?? 10),
			),
			privacy: Math.min(
				...nodesWithScores.map((node) => node.scores?.privacy ?? 10),
			),
			performance: Math.min(
				...nodesWithScores.map((node) => node.scores?.performance ?? 10),
			),
			governance: Math.min(
				...nodesWithScores.map((node) => node.scores?.governance ?? 10),
			),
			reliability: Math.min(
				...nodesWithScores.map((node) => node.scores?.reliability ?? 10),
			),
			cost: Math.min(...nodesWithScores.map((node) => node.scores?.cost ?? 10)),
		};
	}, [board.nodes]);

	return (
		<BubbleActions
			actions={[
				{
					id: "open",
					label: "Open Board",
					icon: <ExternalLink className="h-4 w-4 text-foreground" />,
					onClick: () => onOpenBoard(board.id),
				},
				{
					id: "delete",
					label: "Delete Board",
					icon: <Trash2 className="h-4 w-4 text-foreground" />,
					variant: "destructive",
					onClick: () => onDeleteBoard(board.id),
				},
			]}
			side="top"
			align="end"
		>
			<Card
				title={board.id}
				className="relative group border shadow-sm hover:shadow-lg transition-all duration-200 cursor-pointer hover:border-primary/30 h-full flex flex-col"
			>
				<CardHeader className="pb-2">
					<div className="flex items-start gap-3">
						<div className="p-2 rounded-lg bg-primary/10 shrink-0">
							<WorkflowIcon className="h-4 w-4 text-primary" />
						</div>
						<div className="flex-1 min-w-0">
							<div className="flex items-center gap-2 justify-between">
								<CardTitle className="text-base font-semibold truncate group-hover:text-primary transition-colors">
									{board.name}
								</CardTitle>
								{aggregatedScores && (
									<div
										className="relative z-10 shrink-0"
										onClick={(event) => event.stopPropagation()}
									>
										<FlowLibraryScoreBar scores={aggregatedScores} />
									</div>
								)}
							</div>
							<div className="flex items-center gap-1.5 mt-1 flex-wrap">
								<Badge variant="secondary" className="text-[10px] px-1.5 py-0">
									{board.stage}
								</Badge>
								<ExecutionModeBadge mode={board.execution_mode} />
								<span className="text-[10px] text-muted-foreground">
									{board.log_level}
								</span>
							</div>
						</div>
					</div>
				</CardHeader>
				<CardContent className="pt-0 space-y-3 flex-1 flex flex-col">
					{board.description && (
						<p className="text-sm text-muted-foreground line-clamp-2">
							{board.description}
						</p>
					)}

					{pages.data && pages.data.length > 0 && (
						<div className="flex flex-wrap gap-1">
							{pages.data.slice(0, 4).map((page) => (
								<Link
									key={page.pageId}
									href={`/page-builder?id=${page.pageId}&app=${app.id}&board=${board.id}`}
									className="relative z-10"
									onClick={(event) => event.stopPropagation()}
								>
									<span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-muted/50 hover:bg-muted text-muted-foreground hover:text-foreground transition-colors">
										<FileText className="h-3 w-3" />
										{page.name}
									</span>
								</Link>
							))}
							{pages.data.length > 4 && (
								<HoverCard openDelay={100} closeDelay={200}>
									<HoverCardTrigger asChild>
										<span
											className="inline-flex items-center text-xs px-2 py-0.5 rounded-full bg-muted/50 hover:bg-muted text-muted-foreground hover:text-foreground transition-colors cursor-pointer relative z-10"
											onClick={(event) => event.stopPropagation()}
										>
											+{pages.data.length - 4} more
										</span>
									</HoverCardTrigger>
									<HoverCardContent
										side="bottom"
										align="start"
										className="w-48 p-2"
									>
										<div className="space-y-1">
											{pages.data.slice(4).map((page) => (
												<Link
													key={page.pageId}
													href={`/page-builder?id=${page.pageId}&app=${app.id}&board=${board.id}`}
													className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-muted text-sm transition-colors"
													onClick={(event) => event.stopPropagation()}
												>
													<FileText className="h-3.5 w-3.5 text-muted-foreground" />
													<span className="truncate">{page.name}</span>
												</Link>
											))}
										</div>
									</HoverCardContent>
								</HoverCard>
							)}
						</div>
					)}

					<div className="flex items-center justify-between pt-2 border-t text-xs text-muted-foreground mt-auto">
						<div className="flex items-center gap-3">
							<Tooltip>
								<TooltipTrigger asChild>
									<span className="flex items-center gap-1">
										<SquareMousePointerIcon className="h-3 w-3" />
										{Object.keys(board.nodes).length}
									</span>
								</TooltipTrigger>
								<TooltipContent>Nodes</TooltipContent>
							</Tooltip>
							<Tooltip>
								<TooltipTrigger asChild>
									<span className="flex items-center gap-1">
										<VariableIcon className="h-3 w-3" />
										{Object.keys(board.variables).length}
									</span>
								</TooltipTrigger>
								<TooltipContent>Variables</TooltipContent>
							</Tooltip>
							<Tooltip>
								<TooltipTrigger asChild>
									<span className="flex items-center gap-1">
										<FileText className="h-3 w-3" />
										{pages.data?.length ?? 0}
									</span>
								</TooltipTrigger>
								<TooltipContent>Pages</TooltipContent>
							</Tooltip>
						</div>
						<span className="flex items-center gap-1">
							<Calendar className="h-3 w-3" />
							{formatRelativeTime(board.updated_at)}
						</span>
					</div>
				</CardContent>
				<button
					type="button"
					className="absolute inset-0 rounded-lg"
					onClick={() => onOpenBoard(board.id)}
					data-href={href}
					data-title={board.name}
				/>
			</Card>
		</BubbleActions>
	);
}

function ExecutionModeBadge({ mode }: Readonly<{ mode?: IExecutionMode }>) {
	const effectiveMode = mode ?? IExecutionMode.Hybrid;
	const config = {
		[IExecutionMode.Hybrid]: {
			icon: <Shuffle className="h-2.5 w-2.5" />,
			label: "Hybrid",
		},
		[IExecutionMode.Remote]: {
			icon: <Cloud className="h-2.5 w-2.5" />,
			label: "Remote",
		},
		[IExecutionMode.Local]: {
			icon: <Monitor className="h-2.5 w-2.5" />,
			label: "Local",
		},
	}[effectiveMode];

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Badge variant="outline" className="text-[10px] px-1.5 py-0 gap-0.5">
					{config.icon}
					{config.label}
				</Badge>
			</TooltipTrigger>
			<TooltipContent>
				{effectiveMode === IExecutionMode.Hybrid &&
					"Runs locally when possible, falls back to remote"}
				{effectiveMode === IExecutionMode.Remote &&
					"Always runs on remote servers"}
				{effectiveMode === IExecutionMode.Local && "Always runs locally"}
			</TooltipContent>
		</Tooltip>
	);
}
