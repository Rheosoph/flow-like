"use client";

import { Loader2, Mic, Square, Volume2 } from "lucide-react";
import { cn } from "../../../lib/utils";
import { withAlpha } from "../color";
import { VOICE_DIMENSIONS, type VoiceVisualizerProps } from "../types";

export function Conservative({
	state,
	size,
	color,
	recordingColor,
	hover,
}: VoiceVisualizerProps) {
	const dim = VOICE_DIMENSIONS[size].icon;
	const recording = state === "recording";
	const speaking = state === "speaking";
	const main = recording ? recordingColor : color;
	const iconSize = Math.round(dim * 0.42);
	const ping = recording || speaking || (hover && state === "idle");

	return (
		<div
			className="relative flex items-center justify-center"
			style={{ width: dim, height: dim }}
		>
			{ping && (
				<span
					className="absolute inset-0 rounded-full animate-ping"
					style={{ backgroundColor: withAlpha(main, recording ? 0.3 : 0.2) }}
				/>
			)}
			<div
				className={cn(
					"relative flex items-center justify-center rounded-full shadow-md transition-transform duration-200",
					state === "idle" && hover && "scale-110",
					speaking && "animate-pulse",
				)}
				style={{
					width: dim,
					height: dim,
					backgroundColor: main,
					boxShadow: ping
						? `0 0 ${dim * 0.4}px ${withAlpha(main, 0.5)}`
						: undefined,
				}}
			>
				{state === "processing" ? (
					<Loader2
						className="animate-spin text-white"
						style={{ width: iconSize, height: iconSize }}
					/>
				) : recording ? (
					<Square
						className="fill-white text-white"
						style={{ width: iconSize * 0.8, height: iconSize * 0.8 }}
					/>
				) : speaking ? (
					<Volume2
						className="text-white"
						style={{ width: iconSize, height: iconSize }}
					/>
				) : (
					<Mic
						className="text-white"
						style={{ width: iconSize, height: iconSize }}
					/>
				)}
			</div>
		</div>
	);
}
