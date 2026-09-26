import { useTranslation } from "@flow-like/locales";
import {
	BracesIcon,
	CopyIcon,
	CrosshairIcon,
	EyeOffIcon,
	FilterIcon,
	ListCollapseIcon,
	LocateFixedIcon,
	type LucideIcon,
	ScrollTextIcon,
	XIcon,
} from "lucide-react";
import { type ReactNode, memo, useMemo } from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogGroup } from "../../../lib/schema/flow/log-query";
import type { INode } from "../../../lib/schema/flow/node";
import { templateParts } from "./fold-model";
import { LEVEL_TONES, LevelIcon } from "./level-style";
import {
	LEVEL_LABELS,
	embeddedJson,
	formatAbsolute,
	formatDuration,
	formatRelative,
	levelIndex,
	logDuration,
	logStart,
	prettyJson,
	toMicros,
} from "./log-format";
import { LogNodeChip } from "./log-node-chip";

export interface IDetailActions {
	copyText(log: ILog): void;
	copyJson(log: ILog): void;
	scopeNode(nodeId: string): void;
	excludeNode(nodeId: string): void;
	hideSimilar(fingerprint: string): void;
	onlySimilar(fingerprint: string): void;
	showOnBoard(nodeId: string): void;
	close(): void;
}

const PANE_ACTION =
	"inline-flex h-6 items-center gap-1.5 rounded-md border bg-background px-2 text-xs text-foreground transition-colors hover:bg-secondary focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";

function PaneAction({
	icon: Icon,
	label,
	onClick,
}: Readonly<{ icon: LucideIcon; label: string; onClick: () => void }>) {
	return (
		<button type="button" className={PANE_ACTION} onClick={onClick}>
			<Icon aria-hidden className="size-3.5 text-muted-foreground" />
			{label}
		</button>
	);
}

function Field({
	label,
	children,
	note,
}: Readonly<{ label: string; children: ReactNode; note?: ReactNode }>) {
	return (
		<div className="grid grid-cols-[88px_1fr] gap-2 py-1">
			<dt className="text-xs text-muted-foreground">{label}</dt>
			<dd className="min-w-0 select-text font-mono text-xs wrap-anywhere">
				{children}
				{note ? (
					<div className="font-sans text-[11px] text-muted-foreground">
						{note}
					</div>
				) : null}
			</dd>
		</div>
	);
}

function MessageBody({ message }: Readonly<{ message: string }>) {
	const { t } = useTranslation("flow");
	const whole = useMemo(() => prettyJson(message), [message]);
	const embedded = useMemo(
		() => (whole ? undefined : embeddedJson(message)),
		[message, whole],
	);
	const pre =
		"select-text whitespace-pre-wrap rounded-md border bg-background p-2.5 font-mono text-xs leading-relaxed wrap-anywhere";
	return (
		<>
			<pre className={pre}>{whole ?? message}</pre>
			{embedded ? (
				<div className="flex flex-col gap-1">
					<span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
						{t("logViewEmbeddedJson", "Embedded JSON")}
					</span>
					<pre className={pre}>{embedded}</pre>
				</div>
			) : null}
		</>
	);
}

function GroupStats({
	group,
	base,
}: Readonly<{ group: ILogGroup; base: number }>) {
	const { t } = useTranslation("flow");
	const slots = useMemo(
		() =>
			templateParts(group.template, group.slots).filter(
				(p) => p.kind === "slot",
			),
		[group],
	);
	return (
		<div className="flex flex-col gap-1.5 rounded-md border bg-background p-2.5 text-xs">
			<div className="flex items-center gap-1.5 font-medium">
				<ListCollapseIcon
					aria-hidden
					className="size-3.5 text-muted-foreground"
				/>
				{t("logViewOccurrences", "{{count, number}} occurrences", {
					count: group.count,
				})}
			</div>
			<div className="font-mono text-[11.5px] text-muted-foreground">
				{t("logViewGroupSpan", "first {{first}} · last {{last}}", {
					first: formatRelative(group.first_start, base),
					last: formatRelative(group.last_start, base),
				})}
			</div>
			{slots.length > 0 ? (
				<div className="flex flex-wrap gap-1">
					{slots.map((slot, i) => (
						<span
							// biome-ignore lint/suspicious/noArrayIndexKey: slots keep template order
							key={i}
							className="rounded border border-dashed bg-secondary px-1 font-mono text-[11px] text-foreground/80"
						>
							{slot.kind === "slot" ? slot.label : null}
						</span>
					))}
				</div>
			) : null}
			<pre className="select-text whitespace-pre-wrap font-mono text-[11px] text-muted-foreground wrap-anywhere">
				{group.template}
			</pre>
		</div>
	);
}

export const LogDetailPane = memo(function LogDetailPane({
	log,
	base,
	node,
	nodeName,
	path,
	group,
	fingerprinted,
	actions,
}: Readonly<{
	log?: ILog;
	base: number;
	node?: INode;
	nodeName?: string;
	path: readonly string[];
	group?: ILogGroup;
	fingerprinted: boolean;
	actions: IDetailActions;
}>) {
	const { t } = useTranslation("flow");

	if (!log) {
		return (
			<aside
				aria-label={t("logViewLogDetails", "Log details")}
				className="flex h-full flex-col items-center justify-center gap-1.5 bg-card px-6 text-center"
			>
				<ScrollTextIcon
					aria-hidden
					className="size-5 text-muted-foreground/60"
				/>
				<span className="text-sm font-medium">
					{t("logViewNoLogSelected", "No log selected")}
				</span>
				<span className="text-xs text-muted-foreground">
					{t(
						"logViewNoLogSelectedHint",
						"Click a row to read it here. Arrow keys move, Space selects, E jumps to the next error.",
					)}
				</span>
			</aside>
		);
	}

	const level = levelIndex(log.log_level);
	const start = logStart(log);
	const end = toMicros(log.end);
	const duration = formatDuration(logDuration(log));
	const fingerprint = fingerprinted ? log.fingerprint : undefined;
	const tokenIn = log.stats?.token_in;
	const tokenOut = log.stats?.token_out;

	return (
		<aside
			aria-label={t("logViewLogDetails", "Log details")}
			className="flex h-full min-h-0 flex-col bg-card"
		>
			<div className="flex h-9 shrink-0 items-center gap-2 border-b px-3">
				<LevelIcon level={level} />
				<span className={`text-xs font-bold ${LEVEL_TONES[level]}`}>
					{LEVEL_LABELS[level]}
				</span>
				{log.node_id && nodeName ? (
					<LogNodeChip
						node={node}
						label={nodeName}
						title={t("logViewScopeToNode", "Scope to {{name}}", {
							name: nodeName,
						})}
						onClick={() => log.node_id && actions.scopeNode(log.node_id)}
						className="max-w-40"
					/>
				) : null}
				<span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-muted-foreground">
					{formatAbsolute(start)} · {formatRelative(start, base)}
					{duration ? ` · ${duration}` : ""}
				</span>
				<button
					type="button"
					onClick={actions.close}
					aria-label={t("logViewCloseDetails", "Close details")}
					className="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-secondary hover:text-foreground"
				>
					<XIcon className="size-3.5" />
				</button>
			</div>
			<div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
				{group ? <GroupStats group={group} base={base} /> : null}
				<MessageBody message={log.message ?? ""} />
				<div className="flex flex-wrap gap-1.5">
					<PaneAction
						icon={CopyIcon}
						label={t("logViewCopy", "Copy")}
						onClick={() => actions.copyText(log)}
					/>
					<PaneAction
						icon={BracesIcon}
						label={t("logViewCopyJson", "Copy JSON")}
						onClick={() => actions.copyJson(log)}
					/>
					{log.node_id ? (
						<>
							<PaneAction
								icon={CrosshairIcon}
								label={t("logViewScopeNode", "Scope to node")}
								onClick={() => log.node_id && actions.scopeNode(log.node_id)}
							/>
							<PaneAction
								icon={EyeOffIcon}
								label={t("logViewExcludeNode", "Exclude node")}
								onClick={() => log.node_id && actions.excludeNode(log.node_id)}
							/>
						</>
					) : null}
					{fingerprint ? (
						<>
							<PaneAction
								icon={EyeOffIcon}
								label={
									group
										? t(
												"logViewHideSimilarCount",
												"Hide similar ({{count, number}})",
												{
													count: group.count,
												},
											)
										: t("logViewHideSimilar", "Hide similar")
								}
								onClick={() => actions.hideSimilar(fingerprint)}
							/>
							<PaneAction
								icon={FilterIcon}
								label={t("logViewOnlySimilar", "Only similar")}
								onClick={() => actions.onlySimilar(fingerprint)}
							/>
						</>
					) : null}
					{log.node_id ? (
						<PaneAction
							icon={LocateFixedIcon}
							label={t("logViewShowOnBoard", "Show on board")}
							onClick={() => log.node_id && actions.showOnBoard(log.node_id)}
						/>
					) : null}
				</div>
				<dl className="divide-y border-t">
					<Field
						label={t("logViewFieldNode", "Node")}
						note={path.length > 0 ? path.join(" › ") : undefined}
					>
						{nodeName ?? log.node_id ?? "—"}
					</Field>
					<Field label={t("logViewFieldOperation", "Operation")}>
						{log.operation_id || "—"}
					</Field>
					<Field
						label={t("logViewFieldStart", "Start")}
						note={formatRelative(start, base)}
					>
						{formatAbsolute(start)}
					</Field>
					<Field
						label={t("logViewFieldEnd", "End")}
						note={formatRelative(end, base)}
					>
						{formatAbsolute(end)}
					</Field>
					<Field label={t("logViewFieldDuration", "Duration")}>
						{duration || "—"}
					</Field>
					{tokenIn != null || tokenOut != null ? (
						<Field label={t("logViewFieldTokens", "Tokens")}>
							{t("logViewTokensInOut", "{{in}} in · {{out}} out", {
								in: tokenIn ?? 0,
								out: tokenOut ?? 0,
							})}
						</Field>
					) : null}
				</dl>
			</div>
		</aside>
	);
});
