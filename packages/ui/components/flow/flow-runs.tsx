"use client";

import {
	BanIcon,
	CheckCircle2Icon,
	CircleXIcon,
	CloudIcon,
	CornerRightUpIcon,
	EllipsisVerticalIcon,
	HardDriveIcon,
	Loader2Icon,
	LogsIcon,
	RefreshCcwIcon,
	ScrollIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { memo, useEffect, useState } from "react";
import { toast } from "sonner";
import {
	ILogLevel,
	type INode,
	formatDuration,
	formatRelativeTime,
} from "../../lib";
import { logLevelFromNumber } from "../../lib/log-level";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { useBackend } from "../../state/backend-state";
import {
	type ILogAggregationFilter,
	useLogAggregation,
} from "../../state/log-aggregation-state";
import {
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
	EmptyState,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui";

const FlowRunsComponent = ({
	appId,
	boardId,
	nodes,
	version,
	executeBoard,
	onVersionChange,
	onFocusNode,
}: {
	appId: string;
	boardId: string;
	nodes: {
		[key: string]: INode;
	};
	version: [number, number, number];
	executeBoard: (node: INode, payload?: object) => Promise<void>;
	onVersionChange: (version?: [number, number, number]) => void;
	onFocusNode: (nodeId: string) => void;
}) => {
	const backend = useBackend();
	const {
		currentMetadata,
		setCurrentMetadata,
		currentLogs,
		setFilter,
		refetchLogs,
		isLoading,
	} = useLogAggregation();
	const [localFilter, setLocalFilter] = useState<ILogAggregationFilter>({
		appId,
		boardId,
		limit: 100,
		from: (Date.now() - 5 * 60 * 1000) * 1000,
	});
	const [timeRange, setTimeRange] = useState("last_5_minutes");

	useEffect(() => {
		setFilter(backend, localFilter);
	}, [appId, boardId, backend, localFilter, setFilter]);

	useEffect(() => {
		const now = Date.now();
		const fiveMinutesAgo = now - 5 * 60 * 1000;
		const thirtyMinutesAgo = now - 30 * 60 * 1000;
		const oneHourAgo = now - 60 * 60 * 1000;
		const fiveHoursAgo = now - 5 * 60 * 60 * 1000;
		const twentyFourHoursAgo = now - 24 * 60 * 60 * 1000;
		const thirtyDaysAgo = now - 30 * 24 * 60 * 60 * 1000;

		let from: number | undefined;
		switch (timeRange) {
			case "last_5_minutes":
				from = fiveMinutesAgo;
				break;
			case "last_30_minutes":
				from = thirtyMinutesAgo;
				break;
			case "last_1_hour":
				from = oneHourAgo;
				break;
			case "last_5_hours":
				from = fiveHoursAgo;
				break;
			case "last_24_hours":
				from = twentyFourHoursAgo;
				break;
			case "last_30_days":
				from = thirtyDaysAgo;
				break;
			default:
				from = undefined;
		}

		setLocalFilter((prev) => ({
			...prev,
			from: from ? from * 1000 : undefined,
		}));
	}, [timeRange]);

	return (
		<div className="flex flex-col gap-2 p-4 bg-background grow h-full max-h-full overflow-hidden">
			<div className="flex flex-row items-center justify-between">
				<h3>Runs</h3>
				<Button
					variant={"outline"}
					size={"icon"}
					onClick={() => refetchLogs(backend)}
				>
					<RefreshCcwIcon className="w-4 h-4" />
				</Button>
			</div>
			<div className="flex flex-row items-center gap-2">
				<Select
					value={timeRange}
					onValueChange={(value) => {
						setTimeRange(value);
					}}
				>
					<SelectTrigger className="max-w-[180px]">
						<SelectValue placeholder="Time Range" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="last_5_minutes">5 Minutes</SelectItem>
						<SelectItem value="last_30_minutes">30 Minutes</SelectItem>
						<SelectItem value="last_1_hour">1 Hour</SelectItem>
						<SelectItem value="last_5_hours">5 Hours</SelectItem>
						<SelectItem value="last_24_hours">24 Hours</SelectItem>
						<SelectItem value="last_30_days">30 Days</SelectItem>
						<SelectItem value="unlimited">All</SelectItem>
					</SelectContent>
				</Select>
				<Select
					value={localFilter.nodeId ?? "all"}
					onValueChange={(value) => {
						setLocalFilter((old) => ({
							...old,
							nodeId: value === "all" ? undefined : value,
						}));
					}}
				>
					<SelectTrigger className="max-w-[180px]">
						<SelectValue placeholder="Nodes" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">All</SelectItem>
						{Object.values(nodes)
							.filter((node) => node.start)
							.map((node) => (
								<SelectItem key={node.id} value={node.id}>
									{node.friendly_name}
								</SelectItem>
							))}
					</SelectContent>
				</Select>
				<Select
					value={localFilter.status ?? "all"}
					onValueChange={(value) => {
						let status: ILogLevel | undefined;
						switch (value) {
							case "Debug":
								status = ILogLevel.Debug;
								break;
							case "Warn":
								status = ILogLevel.Warn;
								break;
							case "Error":
								status = ILogLevel.Error;
								break;
							case "Fatal":
								status = ILogLevel.Fatal;
								break;
							default:
								status = undefined;
						}

						setLocalFilter((old) => ({ ...old, status: status }));
					}}
				>
					<SelectTrigger className="max-w-[180px]">
						<SelectValue placeholder="Status" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">All</SelectItem>
						<SelectItem value="Debug">Success</SelectItem>
						<SelectItem value="Warn">Warning</SelectItem>
						<SelectItem value="Error">Error</SelectItem>
						<SelectItem value="Fatal">Fatal</SelectItem>
					</SelectContent>
				</Select>
			</div>
			{isLoading && (
				<div className="flex flex-col items-center justify-center gap-2 py-8 h-full">
					<Loader2Icon className="w-6 h-6 animate-spin text-muted-foreground" />
					<p className="text-sm text-muted-foreground">Loading runs...</p>
				</div>
			)}
			{!isLoading && (!currentLogs || currentLogs.length === 0) && (
				<EmptyState
					className="mt-2 h-full"
					icons={[LogsIcon, ScrollIcon, CheckCircle2Icon]}
					description="No runs found yet, start an event to see your results here!"
					title="No Logs"
				/>
			)}
			<div className="flex flex-col gap-2 max-h-full overflow-y-auto">
				{currentLogs?.map((run) => (
					<button
						key={run.run_id}
						className={`flex flex-row gap-2 items-center justify-between border p-2 rounded-md ${currentMetadata?.run_id === run.run_id ? "bg-muted/50" : "hover:bg-muted/50"}`}
						onClick={() => {
							if (currentMetadata?.run_id === run.run_id) {
								setCurrentMetadata(undefined);
								onVersionChange(undefined);
								return;
							}

							setCurrentMetadata(run);

							// Parse version string safely - handle cases like "v1", "v1-0", "v1-0-0"
							const parseVersion = (
								versionStr: string,
							): [number, number, number] | undefined => {
								const parts = versionStr
									.replace("v", "")
									.split("-")
									.map(Number);
								if (parts.length >= 3 && parts.every((n) => !isNaN(n))) {
									return [parts[0], parts[1], parts[2]];
								}
								// If version doesn't have 3 parts, return undefined (use latest)
								return undefined;
							};

							onVersionChange(
								run.version === `v${version.join("-")}`
									? undefined
									: parseVersion(run.version),
							);
						}}
					>
						<div className="flex flex-col gap-2 items-start justify-center">
							<div className="flex flex-row gap-2 items-center">
								{run.is_remote ? (
									<span title="Remote execution">
										<CloudIcon className="w-3 h-3 text-blue-500" />
									</span>
								) : (
									<span title="Local execution">
										<HardDriveIcon className="w-3 h-3 text-muted-foreground" />
									</span>
								)}
								<small className="leading-none">
									{nodes[run.node_id]?.friendly_name ?? "Deleted Event"}
								</small>
								<small className="text-muted-foreground">
									{run.version === `v${version.join("-")}`
										? "Latest"
										: `${run.version}`}
								</small>
							</div>

							<small className="text-muted-foreground leading-none">
								{formatRelativeTime(
									{
										nanos_since_epoch: (run.start % 1_000_000) * 1000,
										secs_since_epoch: Math.floor(run.start / 1_000_000),
									},
									"narrow",
								)}
							</small>
						</div>
						<div className="flex flex-row items-center gap-2">
							<div className="flex flex-row gap-2 items-center">
								<small className="text-muted-foreground">
									{formatDuration(Math.abs(run.end - run.start))}
								</small>

								<div>
									{logLevelFromNumber(run.log_level) === ILogLevel.Debug && (
										<CheckCircle2Icon className="w-3 h-3 text-green-500" />
									)}
									{logLevelFromNumber(run.log_level) === ILogLevel.Info && (
										<CheckCircle2Icon className="w-3 h-3 text-green-500" />
									)}
									{logLevelFromNumber(run.log_level) === ILogLevel.Warn && (
										<TriangleAlertIcon className="w-3 h-3 text-yellow-500" />
									)}
									{logLevelFromNumber(run.log_level) === ILogLevel.Error && (
										<CircleXIcon className="w-3 h-3 text-red-500" />
									)}
									{logLevelFromNumber(run.log_level) === ILogLevel.Fatal && (
										<BanIcon className="w-3 h-3 text-red-800" />
									)}
								</div>
							</div>

							<DropdownMenu>
								<DropdownMenuTrigger>
									<Button
										size={"icon"}
										className="px-0 mx-0 w-4"
										variant={"ghost"}
									>
										<EllipsisVerticalIcon className="w-4 h-4" />
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent>
									<DropdownMenuLabel>Log Actions</DropdownMenuLabel>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										onClick={() => {
											onFocusNode(run.node_id);
										}}
										className="flex flex-row gap-2 items-center"
									>
										<CornerRightUpIcon className="w-4 h-4" />
										Go to Event
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() => {
											const node = nodes[run.node_id];
											if (!node) {
												toast.error("Node not found");
												return;
											}
											executeBoard(node, parseUint8ArrayToJson(run.payload));
										}}
										className="flex flex-row gap-2 items-center"
									>
										<RefreshCcwIcon className="w-4 h-4" />
										Re-Run
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
						</div>
					</button>
				))}
			</div>
		</div>
	);
};

export const FlowRuns = memo(
	FlowRunsComponent,
	(prev, next) =>
		prev.appId === next.appId &&
		prev.boardId === next.boardId &&
		prev.executeBoard === next.executeBoard &&
		prev.onFocusNode === next.onFocusNode &&
		// shallow compare nodes object by reference
		prev.nodes === next.nodes,
);
