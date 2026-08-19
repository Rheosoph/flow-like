"use client";

import { useTranslation } from "@flow-like/locales";
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
	useSpeechRecognition,
	useVoiceRecorder,
} from "../../voice";

interface VoiceModeProps {
	open: boolean;
	onClose: () => void;
	onSend: (content: string, audioFile?: File) => void | Promise<void>;
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
	const { t } = useTranslation("chat");
	const [sent, setSent] = useState(false);
	const [hover, setHover] = useState(false);
	const [voiceError, setVoiceError] = useState<string | null>(null);
	const [speechFailed, setSpeechFailed] = useState(false);
	const sawBusyRef = useRef(false);

	const submit = useCallback(
		(content: string, audioFile?: File) => {
			if (!content.trim() && !audioFile) return;
			setSent(true);
			setVoiceError(null);
			void onSend(content, audioFile);
		},
		[onSend],
	);

	const recorder = useVoiceRecorder({
		maxDuration: voice?.maxDuration ?? 0,
		stopDelay: 700,
		onComplete: (file) => submit("", file),
		onError: (error) => {
			console.error("Error accessing microphone:", error);
			setVoiceError("Microphone access failed. Check the app's permissions.");
		},
	});
	const speech = useSpeechRecognition({
		continuous: false,
		onEnd: (text) => submit(text),
		onError: (error) => {
			console.error("Error transcribing speech:", error);
			setSpeechFailed(true);
			setVoiceError(
				t('speechRecognitionIsUnavailableTapTheOrbToRecordAudioInstead', 'Speech recognition is unavailable. Tap the orb to record audio instead.'),
			);
		},
	});

	const {
		analyser,
		cancel: cancelRecording,
		isArming,
		isRecording,
		isSupported: recordingSupported,
		prewarm,
		recordingTime,
		start: startRecording,
		stop: stopRecording,
	} = recorder;
	const {
		cancel: cancelSpeech,
		isListening,
		isSupported: speechSupported,
		reset: resetSpeech,
		start: startSpeech,
		stop: stopSpeech,
		transcript,
	} = speech;
	const effectiveMode =
		voice?.mode === "stt" && speechSupported && !speechFailed
			? "stt"
			: "record";
	const capturing = effectiveMode === "stt" ? isListening : isRecording;
	const arming = effectiveMode === "record" && isArming;

	const Visualizer = useMemo(
		() => getVoiceVisualizer(voice?.variant ?? "orb"),
		[voice?.variant],
	);
	const color = voice?.color ?? VOICE_DEFAULT_COLOR;
	const recordingColor = voice?.recordingColor ?? VOICE_DEFAULT_RECORDING_COLOR;

	useSpeakerActivity({
		analyser,
		active: open && effectiveMode === "record" && isRecording,
		silenceThreshold: 0.008,
		silenceDuration: 2000,
		startDelay: 1500,
		onSilence: stopRecording,
	});

	const beginCapture = useCallback(() => {
		setVoiceError(null);
		if (effectiveMode === "stt") {
			resetSpeech();
			startSpeech();
			return;
		}
		if (!recordingSupported) {
			setVoiceError("Voice recording is not supported on this device.");
			return;
		}
		void startRecording();
	}, [
		effectiveMode,
		recordingSupported,
		resetSpeech,
		startRecording,
		startSpeech,
	]);

	const endCapture = useCallback(() => {
		if (effectiveMode === "stt") stopSpeech();
		else stopRecording();
	}, [effectiveMode, stopRecording, stopSpeech]);

	const handleClose = useCallback(() => {
		cancelSpeech();
		cancelRecording();
		setSent(false);
		setVoiceError(null);
		setSpeechFailed(false);
		sawBusyRef.current = false;
		onClose();
	}, [cancelRecording, cancelSpeech, onClose]);

	const handleOrbTap = useCallback(() => {
		if (capturing || arming) {
			endCapture();
			return;
		}
		onInterrupt?.();
		setSent(false);
		sawBusyRef.current = false;
		beginCapture();
	}, [arming, beginCapture, capturing, endCapture, onInterrupt]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: only react to open transitions; capture handles are stable.
	useEffect(() => {
		if (open) {
			setSent(false);
			setVoiceError(null);
			sawBusyRef.current = false;
			beginCapture();
		} else {
			cancelSpeech();
			cancelRecording();
			setSpeechFailed(false);
		}
	}, [open]);

	useEffect(() => {
		if (busy) sawBusyRef.current = true;
	}, [busy]);

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
		const handler = (event: KeyboardEvent) => {
			if (event.key === "Escape") handleClose();
		};
		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, [open, handleClose]);

	if (!open) return null;

	const visualState: VoiceVisualState = capturing
		? "recording"
		: speaking
			? "speaking"
			: sent || arming
				? "processing"
				: "idle";
	const vizAnalyser = speaking
		? speakingAnalyser
		: effectiveMode === "record"
			? analyser
			: null;
	const orbHint =
		capturing || arming
			? t('tapToSend', 'Tap to send')
			: speaking
				? t('tapToInterrupt', 'Tap to interrupt')
				: t('tapToTalk', 'Tap to talk');

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
						if (effectiveMode === "record") prewarm();
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
					{voiceError ? (
						<p
							className="max-w-sm text-center text-sm text-destructive"
							role="alert"
						>
							{voiceError}
						</p>
					) : capturing ? (
						<>
							<p className="max-w-sm text-center text-lg font-medium text-foreground">
								{effectiveMode === "stt" && transcript
									? transcript
									: "Listening…"}
							</p>
							{effectiveMode === "record" && (
								<p className="font-mono text-sm text-muted-foreground">
									{formatTime(recordingTime)}
								</p>
							)}
							<p className="mt-2 text-xs text-muted-foreground/60">
								{effectiveMode === "stt"
									? t('sendsWhenYouFinishSpeaking', 'Sends when you finish speaking')
									: t('willAutosendWhenYouStopTalking', 'Will auto-send when you stop talking')}
							</p>
						</>
					) : speaking ? (
						<p className="text-lg font-medium text-foreground">{t('speaking', 'Speaking…')}</p>
					) : sent ? (
						<p className="animate-pulse text-lg font-medium text-foreground">
							{t('thinking', 'Thinking…')}
						</p>
					) : arming ? (
						<p className="text-sm text-muted-foreground">
							{t('initializingMicrophone', 'Initializing microphone…')}
						</p>
					) : (
						<p className="text-sm text-muted-foreground">{t('tapTheOrbToTalk', 'Tap the orb to talk')}</p>
					)}
				</div>

				{(capturing || arming) && (
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
