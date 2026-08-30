"use client";

import { useTranslation } from "@flow-like/locales";
import ChevronDownIcon from "lucide-react/dist/esm/icons/chevron-down.js";
import DownloadIcon from "lucide-react/dist/esm/icons/download.js";
import ExternalLinkIcon from "lucide-react/dist/esm/icons/external-link.js";
import FileIcon from "lucide-react/dist/esm/icons/file.js";
import GlobeIcon from "lucide-react/dist/esm/icons/globe.js";
import ImageIcon from "lucide-react/dist/esm/icons/image.js";
import MaximizeIcon from "lucide-react/dist/esm/icons/maximize.js";
import PauseIcon from "lucide-react/dist/esm/icons/pause.js";
import PlayIcon from "lucide-react/dist/esm/icons/play.js";
import VideoIcon from "lucide-react/dist/esm/icons/video.js";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn, humanFileSize } from "../../../lib/utils";
import {
	type ProcessedAttachment,
	getAttachmentHost,
	splitFileName,
} from "./attachment";
import { canPreviewFile } from "./attachment-dialog";

/** Visual files shown inline before the rest collapse behind the +N tile. */
const MAX_TILES = 4;
/** Chips shown inline before the rest collapse behind "+N more". */
const MAX_CHIPS = 5;
/** Past this, the whole strip starts folded so the answer stays on screen. */
const COLLAPSE_ABOVE = 6;

const isVisual = (file: ProcessedAttachment) =>
	file.type === "image" || file.type === "video";

const formatClock = (seconds: number) => {
	if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
	const total = Math.floor(seconds);
	return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
};

function TruncatedName({
	displayName,
	className,
}: Readonly<{ displayName: string; className?: string }>) {
	const { stem, suffix } = splitFileName(displayName);
	return (
		<span className={cn("flex min-w-0 items-baseline", className)}>
			<span className="min-w-0 truncate">{stem}</span>
			{suffix && <span className="shrink-0">{suffix}</span>}
		</span>
	);
}

function TypeMark({
	file,
	className,
}: Readonly<{ file: ProcessedAttachment; className?: string }>) {
	return (
		<span
			className={cn(
				"grid size-7 shrink-0 place-items-center rounded-md bg-muted text-[9px] font-semibold uppercase tracking-tight text-muted-foreground transition-colors group-hover:bg-background",
				className,
			)}
		>
			{file.type === "website" ? (
				<GlobeIcon className="size-3.5" />
			) : file.ext ? (
				file.ext.slice(0, 4)
			) : (
				<FileIcon className="size-3.5" />
			)}
		</span>
	);
}

function MediaTile({
	file,
	onActivate,
	overflowCount,
	className,
}: Readonly<{
	file: ProcessedAttachment;
	onActivate: (file: ProcessedAttachment) => void;
	overflowCount?: number;
	className?: string;
}>) {
	const { t } = useTranslation("chat");
	const [source, setSource] = useState(file.thumbnailUrl || file.url);
	const [failed, setFailed] = useState(false);
	const isVideo = file.type === "video";

	// A video without a thumbnail still paints its first frame from the element
	// itself, which beats a grey box for the common screen-capture case.
	const usePosterElement = isVideo && !file.thumbnailUrl;

	const handleError = useCallback(() => {
		if (source !== file.url) {
			setSource(file.url);
			return;
		}
		setFailed(true);
	}, [source, file.url]);

	return (
		<button
			type="button"
			onClick={() => onActivate(file)}
			title={file.displayName}
			aria-label={
				overflowCount
					? t(
							"showOverflowcountMoreAttachments",
							"Show {{overflowCount}} more attachments",
							{ overflowCount },
						)
					: t("openDisplayname", "Open {{displayName}}", {
							displayName: file.displayName,
						})
			}
			className={cn(
				"group relative overflow-hidden rounded-lg border bg-muted/40 outline-none focus-visible:ring-2 focus-visible:ring-primary/50",
				className,
			)}
		>
			{failed ? (
				<span className="flex size-full flex-col items-center justify-center gap-1 px-2 text-muted-foreground">
					{isVideo ? (
						<VideoIcon className="size-4" />
					) : (
						<ImageIcon className="size-4" />
					)}
					<span className="max-w-full truncate text-[11px]">
						{file.displayName}
					</span>
				</span>
			) : usePosterElement ? (
				<video
					src={file.url}
					preload="metadata"
					muted
					playsInline
					className="size-full object-cover"
					onError={handleError}
				/>
			) : (
				<img
					src={source}
					alt={file.displayName}
					loading="lazy"
					className="size-full object-cover"
					onError={handleError}
				/>
			)}

			{isVideo && !overflowCount && (
				<span className="pointer-events-none absolute inset-0 grid place-items-center">
					<span className="grid size-9 place-items-center rounded-full border border-white/30 bg-black/45 text-white">
						<PlayIcon className="size-4 translate-x-px fill-current" />
					</span>
				</span>
			)}

			{overflowCount ? (
				<span className="absolute inset-0 grid place-content-center justify-items-center bg-black/65 text-white">
					<span className="text-lg font-semibold leading-none">{`+${overflowCount}`}</span>
					<span className="mt-0.5 text-[9px] font-medium uppercase tracking-widest opacity-85">
						more
					</span>
				</span>
			) : (
				<span className="pointer-events-none absolute inset-x-0 bottom-0 flex items-baseline gap-1.5 bg-linear-to-t from-black/70 to-transparent px-1.5 pb-1 pt-4 text-[11px] text-white opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100">
					<span className="min-w-0 truncate">{file.displayName}</span>
					{typeof file.size === "number" && (
						<span className="shrink-0 text-[10px] opacity-80">
							{humanFileSize(file.size, true)}
						</span>
					)}
				</span>
			)}
		</button>
	);
}

function MediaMosaic({
	files,
	overflowCount,
	onActivate,
}: Readonly<{
	files: ProcessedAttachment[];
	overflowCount: number;
	onActivate: (file: ProcessedAttachment) => void;
}>) {
	if (files.length === 0) return null;

	if (files.length === 1) {
		const [only] = files;
		return (
			<MediaTile
				file={only}
				onActivate={onActivate}
				className={cn(
					"w-60 max-w-full",
					only.type === "video" ? "aspect-video" : "aspect-4/3",
				)}
			/>
		);
	}

	const lastIndex = files.length - 1;

	return (
		<div
			className={cn(
				"grid w-[22rem] max-w-full gap-1",
				files.length === 3
					? "grid-cols-[1.6fr_1fr] grid-rows-2"
					: "grid-cols-2",
			)}
		>
			{files.map((file, index) => (
				<MediaTile
					key={file.url}
					file={file}
					onActivate={onActivate}
					overflowCount={
						index === lastIndex && overflowCount > 0 ? overflowCount : undefined
					}
					className={cn(
						files.length === 3 && index === 0
							? "row-span-2 h-full"
							: "aspect-4/3",
					)}
				/>
			))}
		</div>
	);
}

function AudioAttachment({
	file,
	onOpen,
}: Readonly<{
	file: ProcessedAttachment;
	onOpen: (file: ProcessedAttachment) => void;
}>) {
	const { t } = useTranslation("chat");
	const audioRef = useRef<HTMLAudioElement>(null);
	const [playing, setPlaying] = useState(false);
	const [current, setCurrent] = useState(0);
	const [duration, setDuration] = useState(0);

	useEffect(() => {
		const element = audioRef.current;
		if (!element) return;
		const onTime = () => setCurrent(element.currentTime);
		const onMeta = () =>
			setDuration(Number.isFinite(element.duration) ? element.duration : 0);
		const onEnd = () => setPlaying(false);

		element.addEventListener("timeupdate", onTime);
		element.addEventListener("loadedmetadata", onMeta);
		element.addEventListener("ended", onEnd);
		return () => {
			element.removeEventListener("timeupdate", onTime);
			element.removeEventListener("loadedmetadata", onMeta);
			element.removeEventListener("ended", onEnd);
		};
	}, []);

	const toggle = useCallback(() => {
		const element = audioRef.current;
		if (!element) return;
		if (element.paused) {
			element.play().then(
				() => setPlaying(true),
				() => onOpen(file),
			);
			return;
		}
		element.pause();
		setPlaying(false);
	}, [file, onOpen]);

	return (
		<div className="flex w-80 max-w-full items-center gap-2 rounded-[10px] border bg-card px-1.5 py-1.5">
			{/* biome-ignore lint/a11y/useMediaCaption: user-supplied recordings carry no track */}
			<audio
				ref={audioRef}
				src={file.url}
				preload="metadata"
				className="hidden"
			/>
			<button
				type="button"
				onClick={toggle}
				aria-label={`${playing ? "Pause" : "Play"} ${file.displayName}`}
				className="grid size-7 shrink-0 place-items-center rounded-full bg-foreground text-background outline-none transition-opacity hover:opacity-90 focus-visible:ring-2 focus-visible:ring-primary/50"
			>
				{playing ? (
					<PauseIcon className="size-3 fill-current" />
				) : (
					<PlayIcon className="size-3 translate-x-px fill-current" />
				)}
			</button>
			<div className="flex min-w-0 flex-1 flex-col gap-0.5">
				<TruncatedName
					displayName={file.displayName}
					className="text-xs leading-tight"
				/>
				<div className="flex items-center gap-2">
					<input
						type="range"
						min={0}
						max={duration || 0}
						step={0.1}
						value={Math.min(current, duration || 0)}
						disabled={!duration}
						aria-label={t("seekDisplayname", "Seek {{displayName}}", {
							displayName: file.displayName,
						})}
						onChange={(event) => {
							const next = Number(event.target.value);
							setCurrent(next);
							if (audioRef.current) audioRef.current.currentTime = next;
						}}
						className="h-1 min-w-0 flex-1 cursor-pointer accent-primary"
					/>
					<span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground">
						{formatClock(current)}
						{duration > 0 ? ` / ${formatClock(duration)}` : ""}
					</span>
				</div>
			</div>
		</div>
	);
}

function FileChip({
	file,
	onOpen,
}: Readonly<{
	file: ProcessedAttachment;
	onOpen: (file: ProcessedAttachment) => void;
}>) {
	const host = file.type === "website" ? getAttachmentHost(file.url) : "";
	const ActionIcon =
		file.type === "website"
			? ExternalLinkIcon
			: canPreviewFile(file)
				? MaximizeIcon
				: DownloadIcon;

	return (
		<button
			type="button"
			onClick={() => onOpen(file)}
			title={file.previewText || file.displayName}
			className="group flex h-9 max-w-full items-center gap-2 rounded-[10px] border bg-card pl-1.5 pr-2 text-left text-[13px] outline-none transition-colors hover:border-muted-foreground/50 hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-primary/50"
		>
			<TypeMark file={file} />
			<TruncatedName displayName={file.displayName} />
			{host ? (
				<span className="shrink-0 font-mono text-[10px] text-muted-foreground">
					{host}
				</span>
			) : (
				typeof file.size === "number" && (
					<span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground">
						{humanFileSize(file.size, true)}
					</span>
				)
			)}
			{file.pageNumber !== undefined && (
				<span className="shrink-0 font-mono text-[10px] text-muted-foreground">
					p.{file.pageNumber}
				</span>
			)}
			<ActionIcon className="size-3.5 shrink-0 text-muted-foreground opacity-40 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
		</button>
	);
}

const KIND_LABEL: Record<ProcessedAttachment["type"], string> = {
	image: "image",
	video: "video",
	audio: "recording",
	pdf: "file",
	document: "file",
	website: "link",
	other: "file",
};

function summariseKinds(files: ProcessedAttachment[]) {
	const counts = new Map<string, number>();
	for (const file of files) {
		const label = KIND_LABEL[file.type];
		counts.set(label, (counts.get(label) ?? 0) + 1);
	}
	return [...counts.entries()]
		.map(([label, count]) => `${count} ${label}${count > 1 ? "s" : ""}`)
		.join(" · ");
}

function AttachmentManifest({
	files,
	totalSize,
	onExpand,
}: Readonly<{
	files: ProcessedAttachment[];
	totalSize: number;
	onExpand: () => void;
}>) {
	const { t } = useTranslation("chat");
	return (
		<button
			type="button"
			onClick={onExpand}
			className="group flex w-96 max-w-full items-center gap-2.5 rounded-[10px] border bg-card py-1.5 pl-2 pr-2.5 text-left outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-primary/50"
		>
			<span className="flex shrink-0">
				{files.slice(0, 3).map((file, index) => (
					<span
						key={file.url}
						className={cn(
							"grid size-7 place-items-center overflow-hidden rounded-md border-2 border-card bg-muted text-[8px] font-semibold uppercase text-muted-foreground",
							index > 0 && "-ml-2.5",
						)}
					>
						{isVisual(file) && (file.thumbnailUrl || file.type === "image") ? (
							<img
								src={file.thumbnailUrl || file.url}
								alt=""
								className="size-full object-cover"
							/>
						) : (
							(file.ext || KIND_LABEL[file.type]).slice(0, 3)
						)}
					</span>
				))}
			</span>
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="text-[13px] font-medium">
					{t("lengthAttachments", "{{length}} attachments", {
						length: files.length,
					})}
				</span>
				<span className="truncate font-mono text-[10px] text-muted-foreground">
					{summariseKinds(files)}
					{totalSize > 0 ? ` · ${humanFileSize(totalSize, true)}` : ""}
				</span>
			</span>
			<ChevronDownIcon className="size-4 shrink-0 text-muted-foreground" />
		</button>
	);
}

function StripHeader({
	count,
	totalSize,
	onShowAll,
	onCollapse,
}: Readonly<{
	count: number;
	totalSize: number;
	onShowAll?: () => void;
	onCollapse?: () => void;
}>) {
	const { t } = useTranslation("chat");
	const trailing = onCollapse
		? { label: t("showLess", "Show less"), action: onCollapse }
		: onShowAll
			? { label: t("showAll", "Show all"), action: onShowAll }
			: undefined;

	return (
		<div className="flex items-center gap-1.5 font-mono text-[10px] tracking-wide text-muted-foreground">
			<span>{t("countFiles", "{{count}} files", { count })}</span>
			{totalSize > 0 && (
				<>
					<span className="opacity-60">·</span>
					<span className="tabular-nums">{humanFileSize(totalSize, true)}</span>
				</>
			)}
			{trailing && (
				<button
					type="button"
					onClick={trailing.action}
					className="ml-auto rounded-sm text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-primary/50"
				>
					{trailing.label}
				</button>
			)}
		</div>
	);
}

/**
 * One strip of attachments under a message: visual files as a mosaic sized to
 * their count, audio as a compact player, everything else as a chip that is
 * only as wide as its name. Overflow stays reachable while the answer streams.
 */
export function AttachmentStrip({
	files,
	onFileClick,
	onFullscreen,
	onShowAll,
}: Readonly<{
	files: ProcessedAttachment[];
	onFileClick: (file: ProcessedAttachment) => void;
	onFullscreen?: (file: ProcessedAttachment) => void;
	onShowAll?: () => void;
}>) {
	const { t } = useTranslation("chat");
	const { visuals, audio, chips, totalSize } = useMemo(() => {
		const grouped = {
			visuals: [] as ProcessedAttachment[],
			audio: [] as ProcessedAttachment[],
			chips: [] as ProcessedAttachment[],
			totalSize: 0,
		};
		for (const file of files) {
			grouped.totalSize += file.size ?? 0;
			if (isVisual(file)) grouped.visuals.push(file);
			else if (file.type === "audio") grouped.audio.push(file);
			else grouped.chips.push(file);
		}
		return grouped;
	}, [files]);

	const openMedia = useCallback(
		(file: ProcessedAttachment) => {
			if (onFullscreen) onFullscreen(file);
			else onFileClick(file);
		},
		[onFullscreen, onFileClick],
	);

	const [expanded, setExpanded] = useState(false);
	const expand = useCallback(() => setExpanded(true), []);

	if (files.length === 0) return null;

	if (files.length > COLLAPSE_ABOVE && !expanded) {
		return (
			<div className="mt-2">
				<AttachmentManifest
					files={files}
					totalSize={totalSize}
					onExpand={expand}
				/>
			</div>
		);
	}

	const shownVisuals = visuals.slice(0, MAX_TILES);
	const shownChips = chips.slice(0, MAX_CHIPS);
	const chipOverflow = chips.length - shownChips.length;
	const visualOverflow = visuals.length - shownVisuals.length;

	const sections: ReactNode[] = [];
	if (shownVisuals.length > 0) {
		sections.push(
			<MediaMosaic
				key="mosaic"
				files={shownVisuals}
				overflowCount={visualOverflow}
				onActivate={(file) =>
					visualOverflow > 0 && file === shownVisuals[shownVisuals.length - 1]
						? onShowAll?.()
						: openMedia(file)
				}
			/>,
		);
	}
	for (const file of audio) {
		sections.push(
			<AudioAttachment key={file.url} file={file} onOpen={onFileClick} />,
		);
	}
	if (shownChips.length > 0) {
		sections.push(
			<div key="chips" className="flex flex-wrap gap-1.5">
				{shownChips.map((file) => (
					<FileChip key={file.url} file={file} onOpen={onFileClick} />
				))}
				{chipOverflow > 0 && (
					<button
						type="button"
						onClick={onShowAll}
						className="h-9 rounded-[10px] border border-dashed px-2.5 font-mono text-[11px] text-muted-foreground outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-primary/50"
					>
						{t("chipoverflowMore", "+{{chipOverflow}} more", { chipOverflow })}
					</button>
				)}
			</div>,
		);
	}

	return (
		<div
			className="mt-2 flex flex-col gap-1.5"
			style={{ maxWidth: "var(--fl-chat-measure, 38rem)" }}
		>
			{files.length > 1 && (
				<StripHeader
					count={files.length}
					totalSize={totalSize}
					onShowAll={
						visualOverflow > 0 || chipOverflow > 0 ? onShowAll : undefined
					}
					onCollapse={expanded ? () => setExpanded(false) : undefined}
				/>
			)}
			{sections}
		</div>
	);
}
