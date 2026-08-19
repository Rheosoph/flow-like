import { useTranslation } from "@flow-like/locales";
import { useDebounce } from "@uidotdev/usehooks";
import {
	BombIcon,
	CheckCircle2Icon,
	CircleXIcon,
	CopyIcon,
	CornerRightUpIcon,
	InfoIcon,
	LogsIcon,
	ScrollIcon,
	TriangleAlertIcon,
	XIcon,
} from "lucide-react";
import {
	type RefObject,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { AutoSizer } from "react-virtualized";
import "react-virtualized/styles.css";
import { QuestionMarkCircledIcon } from "@radix-ui/react-icons";
import { VariableSizeList as List, type VariableSizeList } from "react-window";
import { toast } from "sonner";
import { type IBoard, type ILog, useBackend, useInfiniteInvoke } from "../..";
import { parseTimespan } from "../../lib/date";
import { logLevelToNumber } from "../../lib/log-level";
import { ILogLevel, type ILogMessage } from "../../lib/schema/flow/run";
import { useLogAggregation } from "../../state/log-aggregation-state";
import { DynamicImage, EmptyState } from "../ui";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";

interface IEnrichedLogMessage extends ILogMessage {
	node_id: string;
}

export function Traces({
	appId,
	boardId,
	board,
	onFocusNode,
	nodeIdFilter,
	onClearNodeIdFilter,
}: Readonly<{
	appId: string;
	boardId: string;
	board: RefObject<IBoard | undefined>;
	onFocusNode: (nodeId: string) => void;
	nodeIdFilter?: string;
	onClearNodeIdFilter?: () => void;
}>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const { currentMetadata } = useLogAggregation();

	const [queryParts, setQueryParts] = useState<string[]>([]);
	const [query, setQuery] = useState("");

	const [logFilter, setLogFilter] = useState<Set<ILogLevel>>(
		new Set([
			ILogLevel.Debug,
			ILogLevel.Info,
			ILogLevel.Warn,
			ILogLevel.Error,
			ILogLevel.Fatal,
		]),
	);

	const { data, hasNextPage, fetchNextPage, isFetchingNextPage } =
		useInfiniteInvoke(
			backend.boardState.queryRun,
			backend.boardState,
			[currentMetadata!, query],
			100,
			typeof currentMetadata !== "undefined",
		);

	const messages = useMemo(() => {
		return data?.pages.flat() ?? [];
	}, [data]);
	const [search, setSearch] = useState<string>("");
	const debouncedSearch = useDebounce(search, 300);
	const rowHeights = useRef(new Map());
	const listRef = useRef<VariableSizeList>(null);

	const toggleLogFilter = useCallback((level: ILogLevel) => {
		setLogFilter((prev) => {
			const newFilter = new Set(prev);
			if (newFilter.has(level)) {
				newFilter.delete(level);
			} else {
				newFilter.add(level);
			}
			return newFilter;
		});
	}, []);

	const buildQuery = useCallback((parts: string[]) => {
		if (parts.length === 0) return "";
		if (parts.length === 1) return parts[0];
		return parts.map((part) => `(${part})`).join(" AND ");
	}, []);

	useEffect(() => {
		const parts = [];

		if (logFilter.size > 0 && logFilter.size < 5) {
			parts.push(
				`log_level IN (${Array.from(logFilter)
					.map((level) => logLevelToNumber(level))
					.join(", ")})`,
			);
		}

		if (debouncedSearch.length > 0) {
			parts.push(`message LIKE '%${debouncedSearch}%'`);
		}

		if (nodeIdFilter) {
			parts.push(`node_id = '${nodeIdFilter}'`);
		}

		setQueryParts(parts);
	}, [logFilter, debouncedSearch, nodeIdFilter]);

	useEffect(() => {
		if (queryParts.length === 0) {
			setQuery("");
			return;
		}

		if (queryParts.length === 1) {
			setQuery(queryParts[0]);
			return;
		}

		let query = "";
		queryParts.forEach((part, index) => {
			if (index === 0) {
				query += `(${part})`;
			} else {
				query += ` AND (${part})`;
			}
		});
		setQuery(query);
	}, [queryParts]);

	function getRowHeight(index: number) {
		if (hasNextPage && index === (messages?.length ?? 0)) {
			return 50;
		}
		return (rowHeights.current.get(index) ?? 88) + 6;
	}

	const renderItem = useCallback(
		(props: any) => {
			if (!messages) return null;
			const { index, style } = props;

			if (hasNextPage && index === messages.length) {
				return (
					<div style={style} className="p-2">
						<Button
							className="w-full"
							onClick={async () => {
								if (isFetchingNextPage) return;
								await fetchNextPage();
							}}
							disabled={isFetchingNextPage}
						>
							{t('loadMoreLogs', 'Load more logs')}
						</Button>
					</div>
				);
			}

			const log = messages[index];
			if (!log) return null;

			return (
				<LogMessage
					key={`${log.operation_id ?? index}-${log.start?.nanos_since_epoch ?? 0}-${index}`}
					log={log}
					index={index}
					style={style}
					board={board}
					onSetHeight={setRowHeight}
					onSelectNode={onFocusNode}
				/>
			);
		},
		[
			messages,
			hasNextPage,
			isFetchingNextPage,
			fetchNextPage,
			board,
			onFocusNode,
		],
	);

	useEffect(() => {
		setQuery(buildQuery(queryParts));
	}, [queryParts, buildQuery]);

	function setRowHeight(index: number, height: number) {
		listRef.current?.resetAfterIndex(0);
		rowHeights.current = rowHeights.current.set(index, height);
	}

	return (
		<div className="h-full w-full">
			<div className="transition-all h-full z-10 bg-background border rounded-lg flex flex-col w-full overflow-hidden">
				<div className="flex flex-col h-full p-2">
					<div className="w-full flex flex-row items-center justify-between my-1 px-2 flex-wrap gap-2">
						<div className="flex flex-row items-center gap-1 flex-wrap">
							<LogFilterBadge
								level={ILogLevel.Debug}
								label="Debug"
								logFilter={logFilter}
								toggleLogFilter={toggleLogFilter}
							/>
							<LogFilterBadge
								level={ILogLevel.Info}
								label="Info"
								logFilter={logFilter}
								toggleLogFilter={toggleLogFilter}
							/>
							<LogFilterBadge
								level={ILogLevel.Warn}
								label="Warning"
								logFilter={logFilter}
								toggleLogFilter={toggleLogFilter}
							/>
							<LogFilterBadge
								level={ILogLevel.Error}
								label="Error"
								logFilter={logFilter}
								toggleLogFilter={toggleLogFilter}
							/>
							<LogFilterBadge
								level={ILogLevel.Fatal}
								label="Fatal"
								logFilter={logFilter}
								toggleLogFilter={toggleLogFilter}
							/>
							{nodeIdFilter && (
								<Badge
									className="cursor-pointer gap-1"
									variant="secondary"
									onClick={onClearNodeIdFilter}
								>
									{t('node', 'Node:')}{" "}
									{board.current?.nodes[nodeIdFilter]?.friendly_name ??
										nodeIdFilter.slice(0, 8)}
									<XIcon className="w-3 h-3" />
								</Badge>
							)}
						</div>

						<div className="flex flex-row items-center gap-2">
							<Input
								value={search}
								onChange={(e) => setSearch(e.target.value)}
								placeholder="Search..."
								className="w-32 md:w-48"
							/>
						</div>
					</div>
					<div className="flex flex-col w-full gap-1 overflow-x-auto flex-1 min-h-0 px-2">
						{(messages?.length ?? 0) === 0 && (
							<EmptyState
								className="h-full w-full max-w-full"
								icons={[LogsIcon, ScrollIcon, CheckCircle2Icon]}
								description={t('noLogsFoundYetStartAnEventToSeeYourResultsHere', 'No logs found yet, start an event to see your results here!')}
								title={t('noLogs', 'No Logs')}
							/>
						)}
						{(messages?.length ?? 0) > 0 && (
							<AutoSizer
								className="h-full grow flex flex-col min-h-full"
								disableWidth
							>
								{({ height, width }) => (
									<List
										className="log-container h-full grow flex flex-col"
										height={height}
										itemCount={(messages?.length ?? 0) + (hasNextPage ? 1 : 0)}
										itemSize={getRowHeight}
										ref={listRef}
										width={width}
									>
										{renderItem}
									</List>
								)}
							</AutoSizer>
						)}
					</div>
				</div>
			</div>
		</div>
	);
}

/**
 * Attempts to format a message with pretty-printed JSON.
 * If the entire message is valid JSON, returns the parsed JSON and formatted string.
 * Otherwise returns the original message.
 */
function formatLogMessage(message: string): {
	isJson: boolean;
	content: string;
} {
	const trimmed = message.trim();

	// Check if the entire message is JSON (starts with { or [)
	if (
		(trimmed.startsWith("{") && trimmed.endsWith("}")) ||
		(trimmed.startsWith("[") && trimmed.endsWith("]"))
	) {
		try {
			const parsed = JSON.parse(trimmed);
			const pretty = JSON.stringify(parsed, null, 2);
			return { isJson: true, content: pretty };
		} catch {
			// Not valid JSON, return as-is
		}
	}

	return { isJson: false, content: message };
}

/**
 * Lightweight log message renderer - avoids heavy TextEditor for simple messages
 */
const LogMessageContent = memo(function LogMessageContent({
	message,
}: Readonly<{ message: { isJson: boolean; content: string } }>) {
	if (message.isJson) {
		return (
			<pre className="text-xs font-mono whitespace-pre-wrap break-all bg-muted/30 p-2 rounded max-h-48 overflow-y-auto w-full">
				<code className="break-all">{message.content}</code>
			</pre>
		);
	}
	return <span className="text-sm break-all">{message.content}</span>;
});

const LogMessage = memo(function LogMessage({
	log,
	style,
	index,
	board,
	onSetHeight,
	onSelectNode,
}: Readonly<{
	log: ILog;
	style: any;
	index: number;
	board: RefObject<IBoard | undefined>;
	onSetHeight: (index: number, height: number) => void;
	onSelectNode: (nodeId: string) => void;
}>) {
	const { t } = useTranslation("flow");
	const rowRef = useRef<HTMLDivElement>(null);

	// Use useMemo instead of useState + useEffect to avoid re-renders
	const node = useMemo(() => {
		if (log?.node_id && board.current) {
			return board.current.nodes[log.node_id];
		}
		return undefined;
	}, [log?.node_id, board.current?.nodes]);

	// Format the message - memoized to avoid re-computing on every render
	const formattedMessage = useMemo(
		() => formatLogMessage(log?.message ?? ""),
		[log?.message],
	);

	useEffect(() => {
		if (rowRef.current) {
			onSetHeight(index, rowRef.current.clientHeight);
		}
	}, [rowRef, index, onSetHeight, log?.message]);

	const logLevel = log?.log_level ?? ILogLevel.Debug;

	return (
		<button
			style={style}
			className="scrollbar-gutter-stable"
			onClick={(e) => e.preventDefault()}
		>
			<div
				ref={rowRef}
				className={`flex flex-col items-center border rounded-md ${logLevelToColor(logLevel)}`}
			>
				<div className="flex p-1 px-2  flex-row items-center gap-2 w-full">
					<LogIndicator logLevel={logLevel} />
					<div className="text-start text-wrap break-all flex-1 min-w-0">
						<LogMessageContent message={formattedMessage} />
					</div>
				</div>
				<div className="flex flex-row items-center gap-1 w-full px-2 py-1 border-t justify-between">
					{log.start?.nanos_since_epoch !== log.end?.nanos_since_epoch ? (
						<div className="flex flex-row items-center">
							<small className="text-xs">
								{log.start && log.end ? parseTimespan(log.start, log.end) : "—"}
							</small>
							{log?.stats?.token_out && (
								<small className="text-xs">
									{t('tokenOut', 'Token Out:')} {log.stats?.token_out}
								</small>
							)}
							{log?.stats?.token_in && (
								<small className="text-xs">
									{t('tokenIn', 'Token In:')} {log.stats?.token_in}
								</small>
							)}
						</div>
					) : (
						<div />
					)}
					<div className="flex flex-row items-center gap-1">
						<div className="m-0! mr-2 p-0!">
							{!!node ? (
								<span className={`flex flex-row items-center gap-2`}>
									<DynamicImage
										url={node.icon ?? ""}
										className={`w-4 h-4 size-4 ${logLevelToColor(logLevel, true)}`}
									/>
									{node.friendly_name || node.name}
								</span>
							) : (
								<span className="flex flex-row items-center gap-2">
									<QuestionMarkCircledIcon className="w-4 h-4 size-4" />
									{t('unknownNode', 'Unknown Node')}
								</span>
							)}
						</div>
						<Button
							variant={"outline"}
							size={"icon"}
							className="p-1! h-6 w-6"
							onClick={() => {
								navigator.clipboard.writeText(log?.message ?? "");
								toast.success("Log message copied to clipboard");
							}}
						>
							<CopyIcon className="w-4 h-4" />
						</Button>
						{log?.node_id && (
							<Button
								variant={"outline"}
								size={"icon"}
								className="p-1! h-6 w-6"
								onClick={() => onSelectNode(log.node_id!)}
							>
								<CornerRightUpIcon className="w-4 h-4" />
							</Button>
						)}
					</div>
				</div>
			</div>
		</button>
	);
});

function logLevelToColor(logLevel: ILogLevel, icon = false) {
	const colors: Record<ILogLevel, { base: string; icon: string }> = {
		[ILogLevel.Debug]: {
			base: "bg-muted/20 text-muted-foreground",
			icon: "bg-muted-foreground",
		},
		[ILogLevel.Info]: { base: "bg-background/20", icon: "bg-foreground" },
		[ILogLevel.Warn]: { base: "bg-yellow-400/20", icon: "bg-yellow-400" },
		[ILogLevel.Error]: { base: "bg-rose-400", icon: "bg-rose-400/20" },
		[ILogLevel.Fatal]: { base: "bg-pink-400/30", icon: "bg-pink-400" },
	};

	const entry = colors[logLevel];
	if (!entry) {
		return icon ? "bg-foreground" : "bg-background";
	}
	return icon ? entry.icon : entry.base;
}

function LogIndicator({ logLevel }: Readonly<{ logLevel: ILogLevel }>) {
	switch (logLevel) {
		case ILogLevel.Debug:
			return <ScrollIcon className="w-4 h-4 min-w-4" />;
		case ILogLevel.Info:
			return <InfoIcon className="w-4 h-4 min-w-4" />;
		case ILogLevel.Warn:
			return <TriangleAlertIcon className="w-4 h-4 min-w-4" />;
		case ILogLevel.Error:
			return <CircleXIcon className="w-4 h-4 min-w-4" />;
		case ILogLevel.Fatal:
			return <BombIcon className="w-4 h-4 min-w-4" />;
	}
}

function LogFilterBadge({
	level,
	label,
	logFilter,
	toggleLogFilter,
}: Readonly<{
	level: ILogLevel;
	label: string;
	logFilter: Set<ILogLevel>;
	toggleLogFilter: (level: ILogLevel) => void;
}>) {
	return (
		<Badge
			className="cursor-pointer"
			variant={logFilter.has(level) ? "default" : "outline"}
			onClick={() => toggleLogFilter(level)}
		>
			{label}
		</Badge>
	);
}
