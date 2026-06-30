"use client";

import { Download, Loader2, Pause, Play, Trash2 } from "lucide-react";
import { type ButtonHTMLAttributes, useMemo, useState } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import {
	VOICE_DEFAULT_COLOR,
	VOICE_DEFAULT_RECORDING_COLOR,
	type VoiceSize,
	type VoiceVariant,
} from "./types";
import { useAudioPlayback } from "./use-audio-playback";
import { getVoiceVisualizer } from "./visualizers";

function formatTime(seconds: number): string {
	if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
	const mins = Math.floor(seconds / 60);
	const secs = Math.floor(seconds % 60);
	return `${mins}:${secs.toString().padStart(2, "0")}`;
}

/** Pointer/click handlers the caller builds to match its own invoke mode (tap vs hold). */
export type RecordControlProps = Pick<
	ButtonHTMLAttributes<HTMLButtonElement>,
	| "onClick"
	| "onPointerDown"
	| "onPointerUp"
	| "onPointerLeave"
	| "onPointerEnter"
>;

export interface AudioPlaybackProps {
	src: string | null | undefined;
	variant?: VoiceVariant;
	size?: VoiceSize;
	color?: string;
	recordingColor?: string;
	title?: string;
	autoPlay?: boolean;
	busy?: boolean;
	downloadName?: string;
	onDelete?: () => void;
	/** When set, the visualizer becomes a record control using these handlers (tap or hold, per the caller's invoke mode). Otherwise it toggles playback. */
	recordControl?: RecordControlProps;
	/** Hover label for the record control (e.g. "Tap to record again" / "Hold to record again"). */
	recordHint?: string;
	className?: string;
}

/**
 * Animated, analyser-driven audio player (ChatGPT voice-mode style). The
 * visualizer is the primary control: tapping it records again when `onRecord`
 * is set (voiceInput conversations), otherwise it toggles playback (audio
 * containers fed by Set Media Source). A compact play/pause + elapsed time sit
 * beneath. Shared by both surfaces.
 */
export function AudioPlayback({
	src,
	variant = "waveform",
	size = "md",
	color = VOICE_DEFAULT_COLOR,
	recordingColor = VOICE_DEFAULT_RECORDING_COLOR,
	title,
	autoPlay = false,
	busy = false,
	downloadName,
	onDelete,
	recordControl,
	recordHint,
	className,
}: AudioPlaybackProps) {
	const { isPlaying, currentTime, duration, analyser, toggle } =
		useAudioPlayback(src, autoPlay);
	const Visualizer = useMemo(() => getVoiceVisualizer(variant), [variant]);
	const [hover, setHover] = useState(false);

	const orbInteraction: RecordControlProps = recordControl ?? {
		onClick: toggle,
	};
	const orbDisabled = busy || (!recordControl && !src);
	const hint = recordControl
		? (recordHint ?? "Tap to record again")
		: isPlaying
			? "Tap to pause"
			: "Tap to play";

	return (
		<div className={cn("flex w-full flex-col items-center gap-3", className)}>
			<button
				type="button"
				disabled={orbDisabled}
				onMouseEnter={() => setHover(true)}
				onMouseLeave={() => setHover(false)}
				aria-label={hint}
				className="group flex w-full select-none flex-col items-center gap-2 rounded-xl focus:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-60"
				{...orbInteraction}
			>
				<Visualizer
					analyser={analyser}
					state={isPlaying ? "speaking" : "idle"}
					size={size}
					color={color}
					recordingColor={recordingColor}
					hover={hover}
				/>
				<span className="text-xs text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100">
					{hint}
				</span>
			</button>

			<div className="flex w-full items-center gap-3">
				<Button
					type="button"
					size="icon"
					disabled={!src || busy}
					onClick={toggle}
					className="size-11 shrink-0 rounded-full text-white shadow-md hover:opacity-90"
					style={{ backgroundColor: color }}
				>
					{busy ? (
						<Loader2 className="size-5 animate-spin" />
					) : isPlaying ? (
						<Pause className="size-5" />
					) : (
						<Play className="size-5 translate-x-px" />
					)}
				</Button>

				<div className="min-w-0 flex-1">
					{title && <p className="truncate text-sm font-medium">{title}</p>}
					<p className="font-mono text-xs text-muted-foreground">
						{formatTime(currentTime)}
						{duration > 0 ? ` / ${formatTime(duration)}` : ""}
					</p>
				</div>

				{downloadName && src && (
					<a
						href={src}
						download={downloadName}
						className="flex size-8 items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground"
					>
						<Download className="size-4" />
					</a>
				)}
				{onDelete && (
					<Button
						type="button"
						size="sm"
						variant="ghost"
						onClick={onDelete}
						className="size-8 rounded-full p-0 hover:bg-destructive/10 hover:text-destructive"
					>
						<Trash2 className="size-4" />
					</Button>
				)}
			</div>
		</div>
	);
}
