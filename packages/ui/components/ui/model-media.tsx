"use client";

import { useTranslation } from "@flow-like/locales";
import { PauseIcon, PlayIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";

/** The page plays one sample at a time; starting another pauses this one. */
let activeAudio: HTMLAudioElement | null = null;

export function useAudioSample(src: string | undefined) {
	const audioRef = useRef<HTMLAudioElement | null>(null);
	const [playing, setPlaying] = useState(false);
	const [current, setCurrent] = useState(0);
	const [duration, setDuration] = useState(0);

	useEffect(() => {
		setPlaying(false);
		setCurrent(0);
		setDuration(0);
		if (!src) return;
		return () => {
			const audio = audioRef.current;
			audioRef.current = null;
			if (!audio) return;
			audio.pause();
			if (activeAudio === audio) activeAudio = null;
		};
	}, [src]);

	const toggle = useCallback(() => {
		if (!src) return;
		let audio = audioRef.current;
		if (!audio) {
			const created = new Audio(src);
			created.preload = "metadata";
			created.addEventListener("timeupdate", () =>
				setCurrent(created.currentTime),
			);
			created.addEventListener("loadedmetadata", () =>
				setDuration(Number.isFinite(created.duration) ? created.duration : 0),
			);
			created.addEventListener("play", () => setPlaying(true));
			created.addEventListener("pause", () => setPlaying(false));
			created.addEventListener("ended", () => {
				setPlaying(false);
				setCurrent(0);
			});
			audioRef.current = created;
			audio = created;
		}
		if (!audio.paused) {
			audio.pause();
			return;
		}
		if (activeAudio && activeAudio !== audio) activeAudio.pause();
		activeAudio = audio;
		audio.play().catch(() => setPlaying(false));
	}, [src]);

	return {
		playing,
		current,
		duration,
		progress: duration > 0 ? current / duration : 0,
		toggle,
	};
}

function clock(seconds: number): string {
	const total = Math.max(0, Math.round(seconds));
	return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Decorative bars with a stable shape per sample; filled up to the playback position. */
export function Waveform({
	seed,
	bars = 40,
	progress,
	className,
}: Readonly<{
	seed: string;
	bars?: number;
	progress: number;
	className?: string;
}>) {
	const shape = useMemo(() => {
		let hash = 2166136261;
		for (let i = 0; i < seed.length; i++) {
			hash ^= seed.charCodeAt(i);
			hash = Math.imul(hash, 16777619) >>> 0;
		}
		return Array.from({ length: bars }, (_, i) => {
			hash = (Math.imul(hash, 1664525) + 1013904223) >>> 0;
			const noise = hash / 4294967296;
			const envelope = Math.sqrt(Math.sin((Math.PI * (i + 0.5)) / bars));
			const words = 0.55 + 0.45 * Math.abs(Math.sin(i * 0.75 + seed.length));
			return {
				id: `bar-${i}`,
				at: (i + 0.5) / bars,
				height: Math.max(
					12,
					Math.round(envelope * words * (0.3 + 0.7 * noise) * 100),
				),
			};
		});
	}, [seed, bars]);
	return (
		<div
			aria-hidden="true"
			className={cn("flex h-6 items-center gap-0.5", className)}
		>
			{shape.map((bar) => (
				<span
					key={bar.id}
					className={cn(
						"min-w-0.5 flex-1 rounded-full transition-colors",
						bar.at <= progress ? "bg-primary" : "bg-muted-foreground/30",
					)}
					style={{ height: `${bar.height}%` }}
				/>
			))}
		</div>
	);
}

function PlayPauseButton({
	playing,
	onToggle,
	label,
	className,
}: Readonly<{
	playing: boolean;
	onToggle: () => void;
	label: string;
	className?: string;
}>) {
	const { t } = useTranslation("common");
	return (
		<button
			type="button"
			onClick={(event) => {
				event.stopPropagation();
				onToggle();
			}}
			aria-label={
				playing
					? t("pauseSampleOf", "Pause the sample of {{name}}", { name: label })
					: t("playSampleOf", "Play a sample of {{name}}", { name: label })
			}
			className={cn(
				"grid size-8 shrink-0 place-items-center rounded-full bg-foreground text-background transition-transform hover:scale-105 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2",
				className,
			)}
		>
			{playing ? (
				<PauseIcon className="size-3.5 fill-current" />
			) : (
				<PlayIcon className="ml-0.5 size-3.5 fill-current" />
			)}
		</button>
	);
}

/** Play button, waveform and time for one sample. */
export function AudioSample({
	src,
	label,
	caption,
	size = "default",
	className,
}: Readonly<{
	src: string;
	/** Read by screen readers: whose sample this is. */
	label: string;
	caption?: string;
	size?: "default" | "large";
	className?: string;
}>) {
	const sample = useAudioSample(src);
	const time = sample.playing
		? clock(sample.current)
		: sample.duration > 0
			? clock(sample.duration)
			: "";
	return (
		<div
			className={cn(
				"flex items-center gap-2.5 rounded-lg p-2 pr-2.5 transition-colors",
				sample.playing ? "bg-primary/10" : "bg-muted/60",
				className,
			)}
		>
			<PlayPauseButton
				playing={sample.playing}
				onToggle={sample.toggle}
				label={label}
				className={size === "large" ? "size-9" : undefined}
			/>
			<div className="min-w-0 flex-1">
				<Waveform
					seed={src}
					bars={size === "large" ? 56 : 38}
					progress={sample.progress}
					className={size === "large" ? "h-7" : undefined}
				/>
				{caption && (
					<p className="mt-0.5 truncate text-[11.5px] text-muted-foreground">
						{caption}
					</p>
				)}
			</div>
			{time && (
				<span className="self-start pt-1 font-mono text-[11px] tabular-nums text-muted-foreground">
					{time}
				</span>
			)}
		</div>
	);
}

/** Just the play button, for dense rows. */
export function AudioSampleButton({
	src,
	label,
}: Readonly<{ src: string; label: string }>) {
	const sample = useAudioSample(src);
	return (
		<PlayPauseButton
			playing={sample.playing}
			onToggle={sample.toggle}
			label={label}
			className="size-7"
		/>
	);
}

/** "voice-aria_en-us.mp3" → "voice aria en us": the only label a sample file carries. */
export function mediaLabel(url: string): string {
	try {
		const path = new URL(url, "https://local.invalid").pathname;
		const file = decodeURIComponent(path.split("/").pop() ?? "");
		return file
			.replace(/\.[a-z0-9]+$/i, "")
			.replace(/[-_]+/g, " ")
			.trim();
	} catch {
		return "";
	}
}

/** The card header for image and video models: their own sample output. */
export function MediaStrip({
	images,
	videos,
	className,
}: Readonly<{ images: string[]; videos: string[]; className?: string }>) {
	const shown = images.slice(0, 3);
	if (shown.length === 0 && videos.length > 0) {
		return (
			<div
				className={cn(
					"media relative h-37.5 overflow-hidden bg-muted",
					className,
				)}
			>
				<video
					src={videos[0]}
					muted
					playsInline
					preload="metadata"
					className="size-full object-cover"
				/>
				<span
					aria-hidden="true"
					className="absolute inset-0 grid place-items-center"
				>
					<span className="grid size-10 place-items-center rounded-full bg-black/55 text-white">
						<PlayIcon className="ml-0.5 size-4 fill-current" />
					</span>
				</span>
			</div>
		);
	}
	if (shown.length === 0) return null;
	return (
		<div
			className={cn(
				"grid h-37.5 gap-0.5 overflow-hidden bg-muted",
				shown.length === 2 && "grid-cols-2",
				shown.length >= 3 && "grid-cols-[2fr_1fr] grid-rows-2",
				className,
			)}
		>
			{shown.map((src, index) => (
				<img
					key={src}
					src={src}
					alt=""
					loading="lazy"
					className={cn(
						"size-full object-cover transition-transform duration-500 group-hover:scale-[1.04]",
						shown.length >= 3 && index === 0 && "row-span-2",
					)}
				/>
			))}
		</div>
	);
}

/** Large sample with thumbnails to switch between them. */
export function MediaGallery({
	images,
	videos,
	name,
}: Readonly<{ images: string[]; videos: string[]; name: string }>) {
	const { t } = useTranslation("common");
	const items = useMemo(
		() => [
			...videos.map((src) => ({ kind: "video" as const, src })),
			...images.map((src) => ({ kind: "image" as const, src })),
		],
		[images, videos],
	);
	const [index, setIndex] = useState(0);
	const active = Math.min(index, items.length - 1);
	const shown = items[active];
	if (!shown) return null;
	const position = active + 1;

	return (
		<section
			aria-label={t("sampleOutputs", "Sample outputs")}
			className="flex flex-col gap-3"
		>
			<figure className="relative isolate aspect-16/10 overflow-hidden rounded-xl bg-muted">
				{shown.kind === "video" ? (
					// biome-ignore lint/a11y/useMediaCaption: catalog samples ship without caption files
					<video
						key={shown.src}
						src={shown.src}
						controls
						playsInline
						preload="metadata"
						className="size-full bg-black object-contain"
					/>
				) : (
					<>
						<img
							key={`${shown.src}#backdrop`}
							src={shown.src}
							alt=""
							aria-hidden="true"
							className="absolute inset-0 -z-10 size-full scale-110 object-cover opacity-50 blur-2xl"
						/>
						<img
							key={shown.src}
							src={shown.src}
							alt={t(
								"sampleOutputOf",
								"Sample {{position}} of {{total}} from {{name}}",
								{
									position,
									total: items.length,
									name,
								},
							)}
							className="size-full object-contain animate-in fade-in-0 duration-200"
						/>
					</>
				)}
				{items.length > 1 && (
					<figcaption className="absolute top-3 left-3 rounded-md bg-black/55 px-2 py-0.5 text-[11.5px] font-medium text-white">
						{t("sampleCounter", "Sample {{position}} of {{total}}", {
							position,
							total: items.length,
						})}
					</figcaption>
				)}
			</figure>
			{items.length > 1 && (
				<div className="flex flex-wrap gap-2">
					{items.map((item, i) => (
						<button
							key={item.src}
							type="button"
							aria-pressed={i === active}
							aria-label={t("showSample", "Show sample {{position}}", {
								position: i + 1,
							})}
							onClick={() => setIndex(i)}
							className={cn(
								"relative aspect-16/10 w-22 overflow-hidden rounded-lg bg-muted transition-opacity after:pointer-events-none after:absolute after:inset-0 after:rounded-lg after:ring-inset focus-visible:outline-none focus-visible:after:ring-2 focus-visible:after:ring-ring",
								i === active
									? "opacity-100 after:ring-2 after:ring-primary"
									: "opacity-55 after:ring-1 after:ring-border hover:opacity-100",
							)}
						>
							{item.kind === "video" ? (
								<video
									src={item.src}
									muted
									preload="metadata"
									className="size-full object-cover"
								/>
							) : (
								<img
									src={item.src}
									alt=""
									loading="lazy"
									className="size-full object-cover"
								/>
							)}
						</button>
					))}
				</div>
			)}
		</section>
	);
}
