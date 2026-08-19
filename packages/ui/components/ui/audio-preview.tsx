"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AudioLines,
	Download,
	Gauge,
	Pause,
	Play,
	Repeat,
	SkipBack,
	SkipForward,
	Volume2,
	VolumeX,
} from "lucide-react";
import {
	type CSSProperties,
	type KeyboardEvent,
	type MouseEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";
import { buttonVariants } from "./button";
import { Slider } from "./slider";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

const PLAYBACK_RATES = [0.5, 0.75, 1, 1.25, 1.5, 2];
const WAVEFORM_BAR_COUNT = 72;

function formatTime(seconds: number) {
	if (!Number.isFinite(seconds) || seconds < 0) return "--:--";

	const totalSeconds = Math.floor(seconds);
	const hours = Math.floor(totalSeconds / 3600);
	const minutes = Math.floor((totalSeconds % 3600) / 60);
	const remainingSeconds = totalSeconds % 60;

	if (hours > 0) {
		return `${hours}:${minutes.toString().padStart(2, "0")}:${remainingSeconds
			.toString()
			.padStart(2, "0")}`;
	}

	return `${minutes}:${remainingSeconds.toString().padStart(2, "0")}`;
}

function rawAudioName(src: string, title?: string) {
	if (title) return title;

	try {
		const parsed = new URL(src, window.location.href);
		const queryFilename = parsed.searchParams.get("filename");
		if (queryFilename) return queryFilename;
		const pathName = parsed.pathname.split("/").pop();
		if (pathName) return decodeURIComponent(pathName);
	} catch {
		// Fall through to string parsing for non-URL values.
	}

	if (src.startsWith("data:")) {
		const mediaType = src.split(";")[0].split(":")[1];
		const extension = mediaType?.split("/")[1];
		return extension ? `audio.${extension}` : "Audio";
	}

	return src.split("?")[0].split("/").pop() || "Audio";
}

function audioFormatLabel(src: string, title?: string, mimeType?: string) {
	const mimeSubtype = mimeType?.toLowerCase().startsWith("audio/")
		? mimeType.split("/")[1]
		: undefined;
	if (mimeSubtype) return mimeSubtype.split(";")[0].toUpperCase();

	const extension = rawAudioName(src, title).split(".").pop();
	return extension && extension !== rawAudioName(src, title)
		? extension.toUpperCase()
		: "AUDIO";
}

function createWaveformBars(seed: string) {
	let hash = 2166136261;
	for (let index = 0; index < seed.length; index++) {
		hash ^= seed.charCodeAt(index);
		hash = Math.imul(hash, 16777619);
	}

	return Array.from({ length: WAVEFORM_BAR_COUNT }, (_, index) => {
		hash ^= index + 1;
		hash = Math.imul(hash, 16777619);
		const id = (hash >>> 0).toString(36);
		const normalized = (hash >>> 0) / 4294967295;
		const wave = Math.abs(Math.sin(index * 0.48 + normalized * 2.6));
		return {
			height: Math.round(18 + wave * 42 + normalized * 34),
			id,
		};
	});
}

function finiteDuration(audio: HTMLAudioElement) {
	return Number.isFinite(audio.duration) ? audio.duration : 0;
}

export function AudioPreview({
	src,
	title,
	mimeType,
	showControls = true,
	showDownload = true,
	className,
	style,
	onError,
}: Readonly<{
	src: string;
	title?: string;
	mimeType?: string;
	showControls?: boolean;
	showDownload?: boolean;
	className?: string;
	style?: CSSProperties;
	onError?: () => void;
}>) {
	const { t } = useTranslation("common");
	const audioRef = useRef<HTMLAudioElement>(null);
	const waveformRef = useRef<HTMLButtonElement>(null);
	const [currentTime, setCurrentTime] = useState(0);
	const [duration, setDuration] = useState(0);
	const [isPlaying, setIsPlaying] = useState(false);
	const [volume, setVolume] = useState(0.85);
	const [isMuted, setIsMuted] = useState(false);
	const [playbackRate, setPlaybackRate] = useState(1);
	const [isLooping, setIsLooping] = useState(false);
	const [isMetadataLoaded, setIsMetadataLoaded] = useState(false);
	const [hasError, setHasError] = useState(false);

	const displayName = rawAudioName(src, title);
	const formatLabel = audioFormatLabel(src, title, mimeType);
	const waveformBars = useMemo(
		() => createWaveformBars(`${src}:${displayName}`),
		[src, displayName],
	);
	const canSeek = duration > 0;
	const progressPercent = canSeek
		? Math.min(100, Math.max(0, (currentTime / duration) * 100))
		: 0;
	const sliderMax = canSeek ? duration : 100;
	const sliderValue = canSeek ? currentTime : 0;

	useEffect(() => {
		if (!src) return;

		setCurrentTime(0);
		setDuration(0);
		setIsPlaying(false);
		setIsMetadataLoaded(false);
		setHasError(false);
	}, [src]);

	useEffect(() => {
		const audio = audioRef.current;
		if (!audio) return;

		audio.volume = volume;
		audio.muted = isMuted;
		audio.playbackRate = playbackRate;
		audio.loop = isLooping;
	}, [volume, isMuted, playbackRate, isLooping]);

	const seekTo = useCallback(
		(nextTime: number) => {
			const audio = audioRef.current;
			if (!audio || !canSeek) return;

			const clampedTime = Math.min(duration, Math.max(0, nextTime));
			audio.currentTime = clampedTime;
			setCurrentTime(clampedTime);
		},
		[canSeek, duration],
	);

	const seekFromPointer = useCallback(
		(event: MouseEvent<HTMLButtonElement>) => {
			if (!canSeek || !waveformRef.current) return;

			const bounds = waveformRef.current.getBoundingClientRect();
			const ratio = Math.min(
				1,
				Math.max(0, (event.clientX - bounds.left) / bounds.width),
			);
			seekTo(ratio * duration);
		},
		[canSeek, duration, seekTo],
	);

	const handleWaveformKeyDown = useCallback(
		(event: KeyboardEvent<HTMLButtonElement>) => {
			if (!canSeek) return;

			if (event.key === "ArrowLeft") {
				event.preventDefault();
				seekTo(currentTime - 5);
			}
			if (event.key === "ArrowRight") {
				event.preventDefault();
				seekTo(currentTime + 5);
			}
			if (event.key === "Home") {
				event.preventDefault();
				seekTo(0);
			}
			if (event.key === "End") {
				event.preventDefault();
				seekTo(duration);
			}
		},
		[canSeek, currentTime, duration, seekTo],
	);

	const togglePlayback = useCallback(() => {
		const audio = audioRef.current;
		if (!audio) return;

		if (audio.paused) {
			void audio.play().catch(() => {
				setHasError(true);
				onError?.();
			});
			return;
		}

		audio.pause();
	}, [onError]);

	const seekBy = useCallback(
		(delta: number) => {
			seekTo(currentTime + delta);
		},
		[currentTime, seekTo],
	);

	const updateVolume = useCallback((value: number[]) => {
		const audio = audioRef.current;
		const nextVolume = Math.min(1, Math.max(0, (value[0] ?? 0) / 100));
		if (audio) {
			audio.volume = nextVolume;
			audio.muted = nextVolume === 0;
		}
		setVolume(nextVolume);
		setIsMuted(nextVolume === 0);
	}, []);

	const toggleMute = useCallback(() => {
		const audio = audioRef.current;
		if (!audio) return;

		const nextMuted = !audio.muted;
		audio.muted = nextMuted;
		if (!nextMuted && audio.volume === 0) {
			audio.volume = 0.85;
			setVolume(0.85);
		}
		setIsMuted(nextMuted);
	}, []);

	const cyclePlaybackRate = useCallback(() => {
		const audio = audioRef.current;
		const currentIndex = PLAYBACK_RATES.indexOf(playbackRate);
		const nextRate =
			PLAYBACK_RATES[(currentIndex + 1) % PLAYBACK_RATES.length] ?? 1;

		if (audio) audio.playbackRate = nextRate;
		setPlaybackRate(nextRate);
	}, [playbackRate]);

	const toggleLoop = useCallback(() => {
		const audio = audioRef.current;
		const nextLooping = !isLooping;
		if (audio) audio.loop = nextLooping;
		setIsLooping(nextLooping);
	}, [isLooping]);

	if (hasError) {
		return (
			<div
				className={cn(
					"flex h-full w-full min-h-0 items-center justify-center overflow-auto p-4",
					className,
				)}
				style={style}
			>
				<div className="flex w-full max-w-md flex-col items-center gap-3 rounded-lg border bg-card p-6 text-center shadow-sm">
					<div className="rounded-md border bg-muted/40 p-3">
						<AudioLines className="h-6 w-6 text-muted-foreground" />
					</div>
					<div>
						<p className="font-medium">{t('audioPreviewUnavailable', 'Audio preview unavailable')}</p>
						<p className="mt-1 text-sm text-muted-foreground">
							{t('theFileCouldNotBeLoadedByTheBrowserAudioPlayer', 'The file could not be loaded by the browser audio player.')}
						</p>
					</div>
				</div>
			</div>
		);
	}

	return (
		<div
			className={cn(
				"flex h-full w-full min-h-0 items-center justify-center overflow-auto p-4",
				className,
			)}
			style={style}
		>
			<audio
				ref={audioRef}
				src={src}
				className="hidden"
				preload="metadata"
				onLoadedMetadata={(event) => {
					const audio = event.currentTarget;
					audio.volume = volume;
					audio.muted = isMuted;
					audio.playbackRate = playbackRate;
					audio.loop = isLooping;
					setDuration(finiteDuration(audio));
					setCurrentTime(audio.currentTime);
					setIsMetadataLoaded(true);
				}}
				onDurationChange={(event) =>
					setDuration(finiteDuration(event.currentTarget))
				}
				onTimeUpdate={(event) =>
					setCurrentTime(event.currentTarget.currentTime)
				}
				onPlay={() => setIsPlaying(true)}
				onPause={() => setIsPlaying(false)}
				onEnded={() => setIsPlaying(false)}
				onVolumeChange={(event) => {
					setVolume(event.currentTarget.volume);
					setIsMuted(event.currentTarget.muted);
				}}
				onError={() => {
					setHasError(true);
					onError?.();
				}}
			>
				<track kind="captions" srcLang="en" label="Captions" />
			</audio>

			<div className="w-full max-w-2xl overflow-hidden rounded-lg border bg-card shadow-sm">
				<div className="flex items-start justify-between gap-3 border-b p-4">
					<div className="flex min-w-0 items-center gap-3">
						<div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-md border bg-primary/10 text-primary">
							<AudioLines className="h-5 w-5" />
						</div>
						<div className="min-w-0">
							<p className="truncate text-sm font-semibold text-foreground sm:text-base">
								{displayName}
							</p>
							<div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
								<span>{formatLabel}</span>
								<span className="h-1 w-1 rounded-full bg-muted-foreground/50" />
								<span>
									{isMetadataLoaded
										? t('valDuration', '{{val}} duration', { val: formatTime(duration) })
										: "Loading metadata"}
								</span>
							</div>
						</div>
					</div>

					{showDownload && (
						<Tooltip>
							<TooltipTrigger asChild>
								<a
									href={src}
									download={displayName}
									className={cn(
										buttonVariants({ variant: "outline", size: "icon" }),
										"h-8 w-8",
									)}
								>
									<Download className="h-4 w-4" />
									<span className="sr-only">{t('downloadAudio', 'Download audio')}</span>
								</a>
							</TooltipTrigger>
							<TooltipContent>{t('downloadAudio', 'Download audio')}</TooltipContent>
						</Tooltip>
					)}
				</div>

				<div className="space-y-4 p-4">
					<button
						ref={waveformRef}
						type="button"
						className="group relative flex h-28 w-full items-center gap-1 overflow-hidden rounded-md border bg-background px-4 outline-none transition-colors hover:bg-muted/30 focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] disabled:pointer-events-none disabled:opacity-60"
						onClick={seekFromPointer}
						onKeyDown={handleWaveformKeyDown}
						disabled={!showControls || !canSeek}
						aria-label={t('seekAudioWaveform', 'Seek audio waveform')}
						role="slider"
						aria-valuemin={0}
						aria-valuemax={Math.round(duration)}
						aria-valuenow={Math.round(currentTime)}
						aria-valuetext={t('valOfVal2', '{{val}} of {{val2}}', { val: formatTime(currentTime), val2: formatTime(duration) })}
					>
						{waveformBars.map((bar, index) => {
							const active =
								progressPercent >= ((index + 0.5) / waveformBars.length) * 100;
							return (
								<span
									key={bar.id}
									className={cn(
										"h-8 flex-1 rounded-full transition-colors",
										active
											? "bg-primary shadow-sm shadow-primary/20"
											: "bg-muted-foreground/25 group-hover:bg-muted-foreground/35",
									)}
									style={{ height: `${bar.height}%` }}
								/>
							);
						})}
					</button>

					<div className="flex items-center justify-between gap-3 text-xs font-mono text-muted-foreground">
						<span>{formatTime(currentTime)}</span>
						<span>{formatTime(duration)}</span>
					</div>

					{showControls && (
						<div className="space-y-3 rounded-md border bg-background/80 p-3">
							<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
								<div className="flex items-center justify-center gap-1">
									<Tooltip>
										<TooltipTrigger asChild>
											<button
												type="button"
												className={cn(
													buttonVariants({ variant: "ghost", size: "icon" }),
													"h-8 w-8",
												)}
												onClick={() => seekBy(-10)}
												disabled={!canSeek}
											>
												<SkipBack className="h-4 w-4" />
												<span className="sr-only">{t('back10Seconds', 'Back 10 seconds')}</span>
											</button>
										</TooltipTrigger>
										<TooltipContent>{t('back10Seconds', 'Back 10 seconds')}</TooltipContent>
									</Tooltip>

									<Tooltip>
										<TooltipTrigger asChild>
											<button
												type="button"
												className={cn(
													buttonVariants({ variant: "default", size: "icon" }),
													"h-10 w-10 rounded-full",
												)}
												onClick={togglePlayback}
											>
												{isPlaying ? (
													<Pause className="h-4 w-4" />
												) : (
													<Play className="h-4 w-4" />
												)}
												<span className="sr-only">
													{isPlaying ? "Pause audio" : "Play audio"}
												</span>
											</button>
										</TooltipTrigger>
										<TooltipContent>
											{isPlaying ? "Pause" : "Play"}
										</TooltipContent>
									</Tooltip>

									<Tooltip>
										<TooltipTrigger asChild>
											<button
												type="button"
												className={cn(
													buttonVariants({ variant: "ghost", size: "icon" }),
													"h-8 w-8",
												)}
												onClick={() => seekBy(10)}
												disabled={!canSeek}
											>
												<SkipForward className="h-4 w-4" />
												<span className="sr-only">{t('forward10Seconds', 'Forward 10 seconds')}</span>
											</button>
										</TooltipTrigger>
										<TooltipContent>{t('forward10Seconds', 'Forward 10 seconds')}</TooltipContent>
									</Tooltip>
								</div>

								<Slider
									value={[sliderValue]}
									min={0}
									max={sliderMax}
									step={0.1}
									disabled={!canSeek}
									onValueChange={(value) => seekTo(value[0] ?? 0)}
									className="min-w-0 flex-1"
									aria-label={t('audioPosition', 'Audio position')}
								/>

								<div className="flex items-center justify-center gap-1">
									<Tooltip>
										<TooltipTrigger asChild>
											<button
												type="button"
												className={cn(
													buttonVariants({
														variant: isLooping ? "secondary" : "ghost",
														size: "icon",
													}),
													"h-8 w-8",
												)}
												onClick={toggleLoop}
												aria-pressed={isLooping}
											>
												<Repeat className="h-4 w-4" />
												<span className="sr-only">
													{isLooping ? "Disable loop" : "Enable loop"}
												</span>
											</button>
										</TooltipTrigger>
										<TooltipContent>
											{isLooping ? "Loop on" : "Loop off"}
										</TooltipContent>
									</Tooltip>

									<Tooltip>
										<TooltipTrigger asChild>
											<button
												type="button"
												className={cn(
													buttonVariants({ variant: "outline", size: "sm" }),
													"h-8 gap-1.5 px-2",
												)}
												onClick={cyclePlaybackRate}
											>
												<Gauge className="h-3.5 w-3.5" />
												<span className="text-xs">{`${playbackRate}x`}</span>
											</button>
										</TooltipTrigger>
										<TooltipContent>{t('playbackSpeed', 'Playback speed')}</TooltipContent>
									</Tooltip>

									<Tooltip>
										<TooltipTrigger asChild>
											<button
												type="button"
												className={cn(
													buttonVariants({ variant: "ghost", size: "icon" }),
													"h-8 w-8",
												)}
												onClick={toggleMute}
												aria-pressed={isMuted}
											>
												{isMuted || volume === 0 ? (
													<VolumeX className="h-4 w-4" />
												) : (
													<Volume2 className="h-4 w-4" />
												)}
												<span className="sr-only">
													{isMuted ? "Unmute audio" : "Mute audio"}
												</span>
											</button>
										</TooltipTrigger>
										<TooltipContent>
											{isMuted ? "Unmute" : "Mute"}
										</TooltipContent>
									</Tooltip>

									<Slider
										value={[isMuted ? 0 : Math.round(volume * 100)]}
										min={0}
										max={100}
										step={1}
										onValueChange={updateVolume}
										className="hidden w-20 sm:flex"
										aria-label={t('audioVolume', 'Audio volume')}
									/>
								</div>
							</div>
						</div>
					)}
				</div>
			</div>
		</div>
	);
}
