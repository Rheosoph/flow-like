"use client";

import { Phone, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../ui/button";
import {
	VOICE_DEFAULT_COLOR,
	VOICE_DEFAULT_RECORDING_COLOR,
	type VoiceConfig,
	type VoiceVisualState,
	getVoiceVisualizer,
	useSpeakerActivity,
	useVoiceRecorder,
} from "../../voice";

interface VoiceModeProps {
	open: boolean;
	onClose: () => void;
	onSend: (audioFile: File) => void;
	voice?: VoiceConfig;
	/** The answer is streaming or its audio is playing. */
	busy?: boolean;
	/** Answer audio is currently playing back. */
	speaking?: boolean;
	/** Analyser over the answer audio, so the orb reacts to the spoken reply. */
	speakingAnalyser?: AnalyserNode | null;
	/** Stop the playing answer when the user taps the orb to talk again (barge-in). */
	onInterrupt?: () => void;
}

function formatTime(seconds: number): string {
	const mins = Math.floor(seconds / 60);
	const secs = seconds % 60;
	return `${mins}:${secs.toString().padStart(2, "0")}`;
}

export function VoiceMode({
	open,
	onClose,
	onSend,
	voice,
	busy = false,
	speaking = false,
	speakingAnalyser = null,
	onInterrupt,
}: VoiceModeProps) {
	const [sent, setSent] = useState(false);
	const [hover, setHover] = useState(false);
	const sawBusyRef = useRef(false);

	const recorder = useVoiceRecorder({
		maxDuration: voice?.maxDuration ?? 0,
		stopDelay: 700,
		onComplete: (file) => {
			setSent(true);
			onSend(file);
		},
	});

	const Visualizer = useMemo(
		() => getVoiceVisualizer(voice?.variant ?? "orb"),
		[voice?.variant],
	);
	const color = voice?.color ?? VOICE_DEFAULT_COLOR;
	const recordingColor = voice?.recordingColor ?? VOICE_DEFAULT_RECORDING_COLOR;

	useSpeakerActivity({
		analyser: recorder.analyser,
		active: open && recorder.isRecording,
		silenceThreshold: 0.008,
		silenceDuration: 2000,
		startDelay: 1500,
		onSilence: () => recorder.stop(),
	});

	const handleClose = useCallback(() => {
		recorder.cancel();
		setSent(false);
		sawBusyRef.current = false;
		onClose();
	}, [recorder, onClose]);

	const handleOrbTap = useCallback(() => {
		if (recorder.isRecording) {
			recorder.stop();
			return;
		}
		onInterrupt?.();
		setSent(false);
		sawBusyRef.current = false;
		void recorder.start();
	}, [recorder, onInterrupt]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: only react to open transitions; recorder handles are stable.
	useEffect(() => {
		if (open) {
			setSent(false);
			sawBusyRef.current = false;
			void recorder.start();
		} else {
			recorder.cancel();
		}
	}, [open]);

	useEffect(() => {
		if (busy) sawBusyRef.current = true;
	}, [busy]);

	// Auto-close once the answer has been delivered (or never came).
	useEffect(() => {
		if (!open || !sent) return;
		if (sawBusyRef.current && !busy && !speaking) {
			const id = setTimeout(handleClose, 700);
			return () => clearTimeout(id);
		}
		if (!sawBusyRef.current) {
			const id = setTimeout(() => {
				if (!sawBusyRef.current) handleClose();
			}, 6000);
			return () => clearTimeout(id);
		}
	}, [open, sent, busy, speaking, handleClose]);

	useEffect(() => {
		if (!open) return;
		const handler = (e: KeyboardEvent) => {
			if (e.key === "Escape") handleClose();
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [open, handleClose]);

	if (!open) return null;

	const visualState: VoiceVisualState = recorder.isRecording
		? "recording"
		: speaking
			? "speaking"
			: sent || recorder.isArming
				? "processing"
				: "idle";
	const vizAnalyser = speaking ? speakingAnalyser : recorder.analyser;
	const orbHint = recorder.isRecording
		? "Tap to send"
		: speaking
			? "Tap to interrupt"
			: "Tap to talk";

	return (
		<div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-xl animate-in fade-in duration-300">
			<div className="absolute inset-0 bg-gradient-radial from-violet-500/5 via-transparent to-transparent" />

			<Button
				variant="ghost"
				size="sm"
				className="absolute top-6 right-6 h-10 w-10 rounded-full"
				onClick={handleClose}
			>
				<X className="w-5 h-5" />
			</Button>

			<div className="relative flex flex-col items-center gap-8">
				<button
					type="button"
					onClick={handleOrbTap}
					onMouseEnter={() => {
						setHover(true);
						recorder.prewarm();
					}}
					onMouseLeave={() => setHover(false)}
					aria-label={orbHint}
					className="group flex select-none flex-col items-center gap-3 rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<Visualizer
						analyser={vizAnalyser}
						state={visualState}
						size="lg"
						color={color}
						recordingColor={recordingColor}
						hover={hover}
					/>
					<span className="text-xs text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100">
						{orbHint}
					</span>
				</button>

				<div className="flex flex-col items-center gap-2">
					{recorder.isRecording ? (
						<>
							<p className="text-lg font-medium text-foreground">Listening…</p>
							<p className="font-mono text-sm text-muted-foreground">
								{formatTime(recorder.recordingTime)}
							</p>
							<p className="mt-2 text-xs text-muted-foreground/60">
								Will auto-send when you stop talking
							</p>
						</>
					) : speaking ? (
						<p className="text-lg font-medium text-foreground">Speaking…</p>
					) : sent ? (
						<p className="animate-pulse text-lg font-medium text-foreground">
							Thinking…
						</p>
					) : (
						<p className="text-sm text-muted-foreground">
							Initializing microphone…
						</p>
					)}
				</div>

				{recorder.isRecording && (
					<Button
						variant="destructive"
						size="lg"
						className="h-14 w-14 rounded-full shadow-lg shadow-red-500/20"
						onClick={handleClose}
					>
						<Phone className="w-5 h-5 rotate-135" />
					</Button>
				)}
			</div>
		</div>
	);
}
