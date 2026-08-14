"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	CheckCircle2Icon,
	CircleXIcon,
	CloudIcon,
	CornerRightUpIcon,
	EllipsisVerticalIcon,
	FlameIcon,
	HardDriveIcon,
	Loader2Icon,
	LogsIcon,
	RefreshCcwIcon,
	ScrollIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { memo, useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import {
	type ILog,
	ILogLevel,
	type ILogMetadata,
	type INode,
	formatDuration,
	formatRelativeTime,
} from "../../lib";
import { logLevelFromNumber, logLevelToNumber } from "../../lib/log-level";
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

function parseVersion(
	versionStr: string,
): [number, number, number] | undefined {
	const normalized = versionStr.trim().replace(/^v/i, "");
	const delimiter = normalized.includes("-") ? "-" : ".";
	const parts = normalized.split(delimiter).map(Number);

	if (parts.length >= 3 && parts.every((part) => Number.isFinite(part))) {
		return [parts[0], parts[1], parts[2]];
	}

	return undefined;
}

function isCurrentBoardVersion(
	runVersion: string,
	version: [number, number, number],
) {
	const normalized = runVersion.trim().replace(/^v/i, "");
	return normalized === version.join("-") || normalized === version.join(".");
}

function toMicros(time: ILog["start"]) {
	return (
		time.secs_since_epoch * 1_000_000 +
		Math.floor(time.nanos_since_epoch / 1_000)
	);
}

function hydrateMetadataFromLogs(
	run: ILogMetadata,
	logs: ILog[],
): ILogMetadata {
	const nodes = new Map<string, number>();
	let earliest = Number.POSITIVE_INFINITY;
	let latest = 0;

	for (const log of logs) {
		const start = toMicros(log.start);
		const end = toMicros(log.end);

		earliest = Math.min(earliest, start);
		latest = Math.max(latest, end);

		if (!log.node_id) {
			continue;
		}

		nodes.set(
			log.node_id,
			Math.max(nodes.get(log.node_id) ?? 0, logLevelToNumber(log.log_level)),
		);
	}

	return {
		...run,
		start: Number.isFinite(earliest) ? earliest : run.start,
		end: latest > 0 ? Math.max(run.end, latest) : run.end,
		nodes: Array.from(nodes.entries()),
	};
}

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
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const {
		currentMetadata,
		setCurrentMetadata,
		currentLogs,
		setFilter,
		refetchLogs,
		isLoading,
		heatmapEnabled,
		setHeatmapEnabled,
	} = useLogAggregation();
	const [localFilter, setLocalFilter] = useState<ILogAggregationFilter>({
		appId,
		boardId,
		limit: 100,
		from: (Date.now() - 5 * 60 * 1000) * 1000,
	});
	const [timeRange, setTimeRange] = useState("last_5_minutes");
	const [selectedRunId, setSelectedRunId] = useState<string>();

	useEffect(() => {
		setFilter(backend, localFilter);
	}, [appId, boardId, backend, localFilter, setFilter]);

	useEffect(() => {
		setSelectedRunId(undefined);
	}, [appId, boardId]);

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

	const handleRunSelection = useCallback(
		async (run: ILogMetadata) => {
			if (currentMetadata?.run_id === run.run_id) {
				setSelectedRunId(undefined);
				setCurrentMetadata(undefined);
				onVersionChange(undefined);
				return;
			}

			setSelectedRunId(run.run_id);
			setCurrentMetadata(run);
			onVersionChange(
				isCurrentBoardVersion(run.version, version)
					? undefined
					: parseVersion(run.version),
			);

			try {
				const logs: ILog[] = [];
				const pageSize = 1000;
				let offset = 0;

				while (true) {
					const batch = await backend.boardState.queryRun(
						run,
						"",
						offset,
						pageSize,
					);
					if (batch.length === 0) {
						break;
					}

					logs.push(...batch);

					if (batch.length < pageSize) {
						break;
					}

					offset += batch.length;
				}

				if (
					useLogAggregation.getState().currentMetadata?.run_id === run.run_id
				) {
					setCurrentMetadata(hydrateMetadataFromLogs(run, logs));
				}
			} catch {
				if (
					useLogAggregation.getState().currentMetadata?.run_id === run.run_id
				) {
					setCurrentMetadata(run);
				}
			} finally {
				setSelectedRunId((current) =>
					current === run.run_id ? undefined : current,
				);
			}
		},
		[
			backend.boardState,
			currentMetadata?.run_id,
			onVersionChange,
			setCurrentMetadata,
			version,
		],
	);

	return (
		<div className="flex flex-col gap-2 p-4 bg-background grow h-full max-h-full overflow-hidden">
			<div className="flex flex-row items-center justify-between">
				<h3>{t('runs', 'Runs')}</h3>
				<div className="flex flex-row items-center gap-1.5">
					<Button
						variant={heatmapEnabled ? "default" : "outline"}
						size={"icon"}
						title={t('activityHeatmapOverlayRunCountsAndErrorsPerNode', 'Activity heatmap — overlay run counts and errors per node')}
						aria-pressed={heatmapEnabled}
						onClick={() => {
							const next = !heatmapEnabled;
							setHeatmapEnabled(next);
							// Remote runs only carry per-node summaries when explicitly
							// requested — refetch once so the heatmap has data.
							if (next) void refetchLogs(backend);
						}}
					>
						<FlameIcon className="w-4 h-4" />
					</Button>
					<Button
						variant={"outline"}
						size={"icon"}
						onClick={() => refetchLogs(backend)}
					>
						<RefreshCcwIcon className="w-4 h-4" />
					</Button>
				</div>
			</div>
			<div className="flex flex-row items-center gap-2">
				<Select
					value={timeRange}
					onValueChange={(value) => {
						setTimeRange(value);
					}}
				>
					<SelectTrigger className="max-w-45">
						<SelectValue placeholder={t('timeRange', 'Time Range')} />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="last_5_minutes">{t('5Minutes', '5 Minutes')}</SelectItem>
						<SelectItem value="last_30_minutes">{t('30Minutes', '30 Minutes')}</SelectItem>
						<SelectItem value="last_1_hour">{t('1Hour', '1 Hour')}</SelectItem>
						<SelectItem value="last_5_hours">{t('5Hours', '5 Hours')}</SelectItem>
						<SelectItem value="last_24_hours">{t('24Hours', '24 Hours')}</SelectItem>
						<SelectItem value="last_30_days">{t('30Days', '30 Days')}</SelectItem>
						<SelectItem value="unlimited">{t('all', 'All')}</SelectItem>
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
					<SelectTrigger className="max-w-45">
						<SelectValue placeholder={t('nodes', 'Nodes')} />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">{t('all', 'All')}</SelectItem>
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
					<SelectTrigger className="max-w-45">
						<SelectValue placeholder="Status" />
					</SelectTrigger>
					<SelectContent>
						<SelectItem value="all">{t('all', 'All')}</SelectItem>
						<SelectItem value="Debug">{t('success', 'Success')}</SelectItem>
						<SelectItem value="Warn">{t('warning', 'Warning')}</SelectItem>
						<SelectItem value="Error">{t('error', 'Error')}</SelectItem>
						<SelectItem value="Fatal">{t('fatal', 'Fatal')}</SelectItem>
					</SelectContent>
				</Select>
			</div>
			{isLoading && (
				<div className="flex flex-col items-center justify-center gap-2 py-8 h-full">
					<Loader2Icon className="w-6 h-6 animate-spin text-muted-foreground" />
					<p className="text-sm text-muted-foreground">{t('loadingRuns', 'Loading runs...')}</p>
				</div>
			)}
			{!isLoading && (!currentLogs || currentLogs.length === 0) && (
				<EmptyState
					className="mt-2 h-full"
					icons={[LogsIcon, ScrollIcon, CheckCircle2Icon]}
					description={t('noRunsFoundYetStartAnEventToSeeYourResultsHere', 'No runs found yet, start an event to see your results here!')}
					title={t('noLogs', 'No Logs')}
				/>
			)}
			<div className="flex flex-col gap-2 max-h-full overflow-y-auto">
				{currentLogs?.map((run) => (
					<button
						key={run.run_id}
						className={`flex flex-row gap-2 items-center justify-between border p-2 rounded-md ${currentMetadata?.run_id === run.run_id ? "bg-muted/50" : "hover:bg-muted/50"}`}
						onClick={() => {
							void handleRunSelection(run);
						}}
					>
						<div className="flex flex-col gap-2 items-start justify-center">
							<div className="flex flex-row gap-2 items-center">
								{run.is_remote ? (
									<span title={t('remoteExecution', 'Remote execution')}>
										<CloudIcon className="w-3 h-3 text-blue-500" />
									</span>
								) : (
									<span title={t('localExecution', 'Local execution')}>
										<HardDriveIcon className="w-3 h-3 text-muted-foreground" />
									</span>
								)}
								<small className="leading-none">
									{nodes[run.node_id]?.friendly_name ?? t('deletedEvent', 'Deleted Event')}
								</small>
								<small className="text-muted-foreground">
									{isCurrentBoardVersion(run.version, version)
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
							{selectedRunId === run.run_id && (
								<Loader2Icon className="w-3 h-3 animate-spin text-muted-foreground" />
							)}
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
									<DropdownMenuLabel>{t('logActions', 'Log Actions')}</DropdownMenuLabel>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										onClick={() => {
											onFocusNode(run.node_id);
										}}
										className="flex flex-row gap-2 items-center"
									>
										<CornerRightUpIcon className="w-4 h-4" />
										{t('goToEvent', 'Go to Event')}
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
										{t('rerun', 'Re-Run')}
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
