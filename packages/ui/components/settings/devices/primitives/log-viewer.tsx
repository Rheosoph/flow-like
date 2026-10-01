"use client";

import { useTranslation } from "@flow-like/locales";
import { useVirtualizer } from "@tanstack/react-virtual";
import { CircleSlash, Download, Pause, Play } from "lucide-react";
import {
	type ReactNode,
	type RefObject,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { enumLabel } from "../copy/enum-labels";
import { useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import { Segmented } from "./segmented";
import { cx } from "./tone";

export type LogStream = "stdout" | "stderr";

export interface LogLine {
	kind: "line";
	id: string;
	/** Unix seconds. */
	at: number;
	stream: LogStream;
	message: string;
	/** The device cut the line at 2 048 bytes. */
	truncated?: boolean;
}

/** SPEC §4.30 markers: dropped lines, evicted history, or a caller's own sentence. */
export interface LogGap {
	kind: "gap";
	id: string;
	reason: "dropped" | "evicted" | "other";
	/** `dropped`: how many lines. */
	count?: number;
	/** `evicted`: the oldest sequence number still on the device. */
	before?: number;
	text?: ReactNode;
}

export type LogRecord = LogLine | LogGap;

export interface LogScope {
	value: string;
	label: string;
}

export interface LogViewerProps {
	/** Oldest first. */
	records: readonly LogRecord[];
	/** Accessible name of the log body ("invoice-extractor output"). */
	label: string;
	scopes?: readonly LogScope[];
	scope?: string;
	onScopeChange?: (scope: string) => void;
	/** Controlled when `onErrorsOnlyChange` is passed. */
	errorsOnly?: boolean;
	onErrorsOnlyChange?: (errorsOnly: boolean) => void;
	/** Controlled when `onFollowChange` is passed. */
	follow?: boolean;
	onFollowChange?: (follow: boolean) => void;
	onLoadOlder?: () => void;
	loadingOlder?: boolean;
	onDownload?: () => void;
	/** "following · 12 behind". */
	stamp?: ReactNode;
	/** Inline result under the body (a download's outcome). */
	result?: ReactNode;
	note?: ReactNode;
	emptyText?: ReactNode;
	/** Windowing threshold (default 500 records). */
	windowAt?: number;
	className?: string;
}

/** Above this many records the body is windowed. */
export const LOG_WINDOW_AT = 500;
const LINE_H = 20;
const GAP_H = 32;

function GapText({ gap }: Readonly<{ gap: LogGap }>) {
	const { t } = useTranslation("devices");
	if (gap.text) return <>{gap.text}</>;
	if (gap.reason === "dropped")
		return (
			<>
				{t("view.logs.dropped", {
					count: gap.count ?? 0,
					defaultValue_one:
						"{{count, number}} line dropped here: more than 100 lines per second.",
					defaultValue_other:
						"{{count, number}} lines dropped here: more than 100 lines per second.",
				})}
			</>
		);
	if (gap.reason === "evicted")
		return (
			<>
				{gap.before === undefined
					? t("view.logs.evicted", "Older lines were deleted on the device.")
					: t(
							"view.logs.evictedBefore",
							"Older lines were deleted on the device (before #{{before, number}}).",
							{ before: gap.before },
						)}
			</>
		);
	return null;
}

function GapRow({ gap }: Readonly<{ gap: LogGap }>) {
	return (
		<div data-gap={gap.reason} className="flex h-8 items-center">
			<div className="flex h-6 w-full items-center gap-1.5 border-y border-dashed border-unknown-line px-3 font-sans whitespace-nowrap text-muted-foreground">
				<CircleSlash aria-hidden className="size-3.25 shrink-0" />
				<GapText gap={gap} />
			</div>
		</div>
	);
}

function LineRow({ line }: Readonly<{ line: LogLine }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const err = line.stream === "stderr";
	return (
		<div
			data-stream={line.stream}
			className="grid h-5 grid-cols-[64px_52px_max-content] gap-3 px-3 whitespace-pre"
		>
			<span className="text-muted-foreground tabular-nums">
				{time.clock(line.at)}
			</span>
			<span className={err ? "text-warning" : "text-muted-foreground"}>
				{enumLabel(t, "logStream", line.stream)}
			</span>
			<span className={err ? "text-warning" : undefined}>
				{line.message}
				{line.truncated ? (
					<span className="text-muted-foreground">
						{t("view.logs.truncated", " [line cut at 2 048 bytes]")}
					</span>
				) : null}
			</span>
		</div>
	);
}

function Row({ record }: Readonly<{ record: LogRecord }>) {
	return record.kind === "gap" ? (
		<GapRow gap={record} />
	) : (
		<LineRow line={record} />
	);
}

interface LogScrollerProps {
	scrollRef: RefObject<HTMLDivElement | null>;
	label: string;
	windowed?: boolean;
	children: ReactNode;
}

function LogScroller({
	scrollRef,
	label,
	windowed,
	children,
}: Readonly<LogScrollerProps>) {
	return (
		<div
			ref={scrollRef}
			role="log"
			aria-live="off"
			aria-label={label}
			// biome-ignore lint/a11y/noNoninteractiveTabindex: a scrollable log must be reachable by keyboard
			tabIndex={0}
			data-windowed={windowed ? "" : undefined}
			className="max-h-80 overflow-auto rounded-lg border border-border bg-surface-sunken py-1.5 font-mono text-xs leading-5 [font-variant-ligatures:none] focus-visible:outline-2 focus-visible:outline-ring"
		>
			{children}
		</div>
	);
}

interface WindowedBodyProps {
	records: readonly LogRecord[];
	follow: boolean;
	label: string;
}

function WindowedBody({ records, follow, label }: Readonly<WindowedBodyProps>) {
	const scrollRef = useRef<HTMLDivElement>(null);
	const virtualizer = useVirtualizer({
		count: records.length,
		getScrollElement: () => scrollRef.current,
		estimateSize: (index) => (records[index]?.kind === "gap" ? GAP_H : LINE_H),
		overscan: 20,
	});
	/* Follow the newest record, not the count: a capped buffer keeps its length while lines keep arriving. */
	const count = records.length;
	const lastId = records.at(-1)?.id;
	useEffect(() => {
		if (follow && count > 0 && lastId !== undefined)
			virtualizer.scrollToIndex(count - 1, { align: "end" });
	}, [follow, count, lastId, virtualizer]);
	return (
		<LogScroller scrollRef={scrollRef} label={label} windowed>
			<div
				className="relative min-w-full"
				style={{ height: virtualizer.getTotalSize() }}
			>
				{virtualizer.getVirtualItems().map((item) => {
					const record = records[item.index];
					return (
						<div
							key={record.id}
							data-index={item.index}
							className="absolute left-0 min-w-full"
							style={{ top: item.start }}
						>
							<Row record={record} />
						</div>
					);
				})}
			</div>
		</LogScroller>
	);
}

interface LogLinesProps {
	records: readonly LogRecord[];
	empty: ReactNode;
}

function LogLines({ records, empty }: Readonly<LogLinesProps>) {
	if (!records.length)
		return (
			<GapRow
				gap={{ kind: "gap", id: "empty", reason: "other", text: empty }}
			/>
		);
	return (
		<div className="w-max min-w-full">
			{records.map((record) => (
				<Row key={record.id} record={record} />
			))}
		</div>
	);
}

interface PlainBodyProps extends LogLinesProps {
	follow: boolean;
	label: string;
}

function PlainBody({
	records,
	follow,
	label,
	empty,
}: Readonly<PlainBodyProps>) {
	const scrollRef = useRef<HTMLDivElement>(null);
	const count = records.length;
	const lastId = records.at(-1)?.id;
	useEffect(() => {
		const el = scrollRef.current;
		if (follow && el && count > 0 && lastId !== undefined)
			el.scrollTop = el.scrollHeight;
	}, [follow, count, lastId]);
	return (
		<LogScroller scrollRef={scrollRef} label={label}>
			<LogLines records={records} empty={empty} />
		</LogScroller>
	);
}

/** A value that is controlled when `onChange` is passed, local state otherwise. */
function useControlled<T>(
	value: T | undefined,
	onChange: ((next: T) => void) | undefined,
	initial: T,
): [T, (next: T) => void] {
	const [own, setOwn] = useState(initial);
	return onChange ? [value ?? initial, onChange] : [own, setOwn];
}

interface ScopeSelectProps {
	scopes: readonly LogScope[];
	scope?: string;
	onScopeChange?: (scope: string) => void;
}

function ScopeSelect({
	scopes,
	scope,
	onScopeChange,
}: Readonly<ScopeSelectProps>) {
	const { t } = useTranslation("devices");
	return (
		<Select value={scope} onValueChange={onScopeChange}>
			<SelectTrigger
				size="sm"
				aria-label={t("view.logs.source", "Log source")}
				className="h-7 rounded-lg border-border bg-card text-ui shadow-none"
			>
				<SelectValue />
			</SelectTrigger>
			<SelectContent className="border-border-strong bg-popover shadow-none">
				{scopes.map((option) => (
					<SelectItem
						key={option.value}
						value={option.value}
						className="focus:bg-row-hover focus:text-foreground"
					>
						{option.label}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

interface LogToolbarProps {
	props: Readonly<LogViewerProps>;
	errors: boolean;
	setErrors: (errors: boolean) => void;
	following: boolean;
	setFollowing: (follow: boolean) => void;
}

function LogToolbar({
	props,
	errors,
	setErrors,
	following,
	setFollowing,
}: Readonly<LogToolbarProps>) {
	const { t } = useTranslation("devices");
	const scopes = props.scopes?.length ? (
		<ScopeSelect
			scopes={props.scopes}
			scope={props.scope}
			onScopeChange={props.onScopeChange}
		/>
	) : null;
	const older = props.onLoadOlder ? (
		<DvButton
			size="sm"
			variant="ghost"
			busy={props.loadingOlder}
			onClick={props.onLoadOlder}
		>
			{t("view.logs.older", "Load older")}
		</DvButton>
	) : null;
	const download = props.onDownload ? (
		<DvButton
			size="sm"
			variant="ghost"
			icon={Download}
			onClick={props.onDownload}
		>
			{t("view.logs.download", "Download")}
		</DvButton>
	) : null;
	return (
		<div className="flex flex-wrap items-center gap-2">
			{scopes}
			<Segmented
				label={t("view.logs.streams", "Streams")}
				size="sm"
				value={errors ? "errors" : "all"}
				onChange={(value) => setErrors(value === "errors")}
				options={[
					{ value: "all", label: t("view.logs.all", "All") },
					{ value: "errors", label: t("view.logs.errorsOnly", "Errors only") },
				]}
			/>
			<DvButton
				size="sm"
				icon={following ? Pause : Play}
				onClick={() => setFollowing(!following)}
			>
				{following
					? t("view.logs.pause", "Pause")
					: t("view.logs.follow", "Follow")}
			</DvButton>
			{older}
			{download}
			<span className="flex-1" />
			{props.stamp}
		</div>
	);
}

/** SPEC §4.30: streams, follow, older lines and download over a mono body; windowed for long buffers. */
export function LogViewer(props: Readonly<LogViewerProps>) {
	const { records, label, result, note, className } = props;
	const { t } = useTranslation("devices");
	const [errors, setErrors] = useControlled(
		props.errorsOnly,
		props.onErrorsOnlyChange,
		false,
	);
	const [following, setFollowing] = useControlled(
		props.follow,
		props.onFollowChange,
		true,
	);
	const shown = errors
		? records.filter(
				(record) => record.kind === "gap" || record.stream === "stderr",
			)
		: records;
	const windowed = shown.length >= (props.windowAt ?? LOG_WINDOW_AT);
	const empty =
		errors && records.length
			? t("view.logs.noErrors", "No error lines.")
			: (props.emptyText ?? t("view.logs.empty", "No lines yet."));
	const body = windowed ? (
		<WindowedBody records={shown} follow={following} label={label} />
	) : (
		<PlainBody records={shown} follow={following} label={label} empty={empty} />
	);

	return (
		<div
			data-log-viewer=""
			data-follow={following}
			data-errors={errors}
			className={cx("flex min-w-0 flex-col gap-2", className)}
		>
			<LogToolbar
				props={props}
				errors={errors}
				setErrors={setErrors}
				following={following}
				setFollowing={setFollowing}
			/>
			{body}
			{result}
			{note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
		</div>
	);
}
