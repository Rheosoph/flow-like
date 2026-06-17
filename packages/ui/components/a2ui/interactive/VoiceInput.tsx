"use client";

import { Loader2, Mic, Square, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Label } from "../../ui/label";
import {
	useComponentActionTrigger,
	useIsComponentTriggering,
	useOnAction,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, VoiceInputComponent } from "../types";

interface VoiceData {
	name: string;
	size: number;
	type: string;
	duration: number;
	backendUrl?: string;
	uploading?: boolean;
	uploadError?: string;
}

function toStoredVoice(voice: VoiceData): VoiceData {
	const { uploading: _uploading, uploadError: _uploadError, ...stored } = voice;
	return stored;
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function formatDuration(seconds: number): string {
	const mins = Math.floor(seconds / 60);
	const secs = Math.floor(seconds % 60);
	return `${mins}:${secs.toString().padStart(2, "0")}`;
}

function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

// Waveform visualizer that reacts to live audio input
function WaveformVisualizer({
	analyser,
	isRecording,
	visualizer,
}: {
	analyser: AnalyserNode | null;
	isRecording: boolean;
	visualizer: string;
}) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const animationRef = useRef<number>(0);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas || !analyser || !isRecording) {
			if (canvas) {
				const ctx = canvas.getContext("2d");
				if (ctx) {
					ctx.clearRect(0, 0, canvas.width, canvas.height);
				}
			}
			return;
		}

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		const bufferLength = analyser.frequencyBinCount;
		const dataArray = new Uint8Array(bufferLength);

		const draw = () => {
			animationRef.current = requestAnimationFrame(draw);
			analyser.getByteTimeDomainData(dataArray);

			const { width, height } = canvas;
			ctx.clearRect(0, 0, width, height);

			if (visualizer === "bars") {
				drawBars(ctx, dataArray, bufferLength, width, height);
			} else {
				drawWaveform(ctx, dataArray, bufferLength, width, height);
			}
		};

		draw();

		return () => {
			cancelAnimationFrame(animationRef.current);
		};
	}, [analyser, isRecording, visualizer]);

	return (
		<canvas
			ref={canvasRef}
			width={300}
			height={80}
			className="w-full h-20 rounded-lg"
		/>
	);
}

function drawWaveform(
	ctx: CanvasRenderingContext2D,
	dataArray: Uint8Array,
	bufferLength: number,
	width: number,
	height: number,
) {
	const sliceWidth = width / bufferLength;
	let x = 0;

	// Glow effect
	ctx.shadowBlur = 6;
	ctx.shadowColor = "rgba(139, 92, 246, 0.5)";
	ctx.lineWidth = 2.5;

	// Gradient stroke
	const gradient = ctx.createLinearGradient(0, 0, width, 0);
	gradient.addColorStop(0, "rgba(139, 92, 246, 0.9)");
	gradient.addColorStop(0.5, "rgba(59, 130, 246, 0.9)");
	gradient.addColorStop(1, "rgba(139, 92, 246, 0.9)");
	ctx.strokeStyle = gradient;

	ctx.beginPath();
	for (let i = 0; i < bufferLength; i++) {
		const v = dataArray[i] / 128.0;
		const y = (v * height) / 2;
		if (i === 0) {
			ctx.moveTo(x, y);
		} else {
			ctx.lineTo(x, y);
		}
		x += sliceWidth;
	}
	ctx.stroke();

	// Mirror line (subtle)
	ctx.shadowBlur = 0;
	ctx.globalAlpha = 0.15;
	x = 0;
	ctx.beginPath();
	for (let i = 0; i < bufferLength; i++) {
		const v = dataArray[i] / 128.0;
		const y = height - (v * height) / 2;
		if (i === 0) {
			ctx.moveTo(x, y);
		} else {
			ctx.lineTo(x, y);
		}
		x += sliceWidth;
	}
	ctx.stroke();
	ctx.globalAlpha = 1;
}

function drawBars(
	ctx: CanvasRenderingContext2D,
	dataArray: Uint8Array,
	bufferLength: number,
	width: number,
	height: number,
) {
	const barCount = 48;
	const barWidth = width / barCount - 2;
	const step = Math.floor(bufferLength / barCount);

	for (let i = 0; i < barCount; i++) {
		const value = dataArray[i * step] / 128.0;
		const barHeight = Math.max(2, Math.abs(value - 1) * height * 0.8);

		const hue = 250 + (i / barCount) * 60;
		ctx.fillStyle = `hsla(${hue}, 80%, 65%, 0.85)`;

		const x = i * (barWidth + 2);
		const y = (height - barHeight) / 2;

		ctx.beginPath();
		ctx.roundRect(x, y, barWidth, barHeight, 2);
		ctx.fill();
	}
}

// Idle pulsing ring animation
function IdlePulseRing() {
	return (
		<div className="relative flex items-center justify-center">
			<div className="absolute w-20 h-20 rounded-full bg-primary/5 animate-ping" />
			<div className="absolute w-16 h-16 rounded-full bg-primary/10 animate-pulse" />
			<div className="relative w-14 h-14 rounded-full bg-linear-to-br from-violet-500 to-blue-500 flex items-center justify-center shadow-lg shadow-violet-500/20 cursor-pointer hover:scale-105 transition-transform">
				<Mic className="w-6 h-6 text-white" />
			</div>
		</div>
	);
}

// Recording pulse ring
function RecordingPulseRing({ duration }: { duration: number }) {
	return (
		<div className="relative flex items-center justify-center">
			<div className="absolute w-20 h-20 rounded-full bg-red-500/10 animate-ping" />
			<div className="absolute w-16 h-16 rounded-full bg-red-500/15 animate-pulse" />
			<div className="relative w-14 h-14 rounded-full bg-linear-to-br from-red-500 to-rose-600 flex items-center justify-center shadow-lg shadow-red-500/30 cursor-pointer hover:scale-105 transition-transform">
				<Square className="w-5 h-5 text-white fill-white" />
			</div>
			<div className="absolute -bottom-6 text-xs font-mono text-red-500 font-medium">
				{formatDuration(duration)}
			</div>
		</div>
	);
}

export function A2UIVoiceInput({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<VoiceInputComponent>) {
	const onAction = useOnAction();
	const triggerAction = useComponentActionTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);
	const backend = useBackend();
	const { setByPath } = useData();

	const value = useResolved<VoiceData>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const error = useResolved<boolean>(component.error);
	const label = useResolved<string>(component.label);
	const helperText = useResolved<string>(component.helperText);
	const maxDuration = useResolved<number>(component.maxDuration) ?? 300;
	const autoStop = useResolved<boolean>(component.autoStop) ?? false;
	const silenceThreshold =
		useResolved<number>(component.silenceThreshold) ?? 0.01;
	const silenceDuration =
		useResolved<number>(component.silenceDuration) ?? 2000;
	const visualizer = useResolved<string>(component.visualizer) ?? "waveform";

	const [isRecording, setIsRecording] = useState(false);
	const [recordingTime, setRecordingTime] = useState(0);
	const [isUploading, setIsUploading] = useState(false);
	const [localVoice, setLocalVoice] = useState<VoiceData | null>(null);
	const [analyserNode, setAnalyserNode] = useState<AnalyserNode | null>(null);

	const mediaRecorderRef = useRef<MediaRecorder | null>(null);
	const audioChunksRef = useRef<Blob[]>([]);
	const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const audioContextRef = useRef<AudioContext | null>(null);
	const silenceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const analyserRef = useRef<AnalyserNode | null>(null);

	const display = localVoice || value || null;

	// Cleanup on unmount
	useEffect(() => {
		return () => {
			if (timerRef.current) clearInterval(timerRef.current);
			if (silenceTimerRef.current) clearTimeout(silenceTimerRef.current);
			if (audioContextRef.current) {
				audioContextRef.current.close().catch(() => {});
			}
		};
	}, []);

	const detectSilence = useCallback(
		(analyser: AnalyserNode) => {
			const dataArray = new Float32Array(analyser.fftSize);
			let silentSince: number | null = null;

			const check = () => {
				if (
					!mediaRecorderRef.current ||
					mediaRecorderRef.current.state !== "recording"
				)
					return;

				analyser.getFloatTimeDomainData(dataArray);
				let sumSquares = 0;
				for (let i = 0; i < dataArray.length; i++) {
					sumSquares += dataArray[i] * dataArray[i];
				}
				const rms = Math.sqrt(sumSquares / dataArray.length);

				if (rms < silenceThreshold) {
					if (!silentSince) silentSince = Date.now();
					if (Date.now() - silentSince > silenceDuration) {
						stopRecording();
						return;
					}
				} else {
					silentSince = null;
				}

				silenceTimerRef.current = setTimeout(check, 100);
			};

			check();
		},
		[silenceThreshold, silenceDuration],
	);

	const startRecording = useCallback(async () => {
		if (disabled) return;

		try {
			const stream = await navigator.mediaDevices.getUserMedia({
				audio: true,
			});

			// Set up audio context for visualization + silence detection
			const audioContext = new AudioContext();
			const source = audioContext.createMediaStreamSource(stream);
			const analyser = audioContext.createAnalyser();
			analyser.fftSize = 2048;
			source.connect(analyser);
			audioContextRef.current = audioContext;
			analyserRef.current = analyser;
			setAnalyserNode(analyser);

			const mediaRecorder = new MediaRecorder(stream);
			mediaRecorderRef.current = mediaRecorder;
			audioChunksRef.current = [];

			mediaRecorder.ondataavailable = (event) => {
				if (event.data.size > 0) {
					audioChunksRef.current.push(event.data);
				}
			};

			mediaRecorder.onstop = () => {
				stream.getTracks().forEach((track) => track.stop());
				if (audioContextRef.current) {
					audioContextRef.current.close().catch(() => {});
					audioContextRef.current = null;
				}
				setAnalyserNode(null);
			};

			mediaRecorder.start();
			setIsRecording(true);
			setRecordingTime(0);
			setLocalVoice(null);

			timerRef.current = setInterval(() => {
				setRecordingTime((prev) => {
					if (prev + 1 >= maxDuration) {
						stopRecording();
						return prev;
					}
					return prev + 1;
				});
			}, 1000);

			if (autoStop) {
				detectSilence(analyser);
			}
		} catch (err) {
			console.error("Microphone access denied:", err);
		}
	}, [disabled, maxDuration, autoStop, detectSilence]);

	const stopRecording = useCallback(() => {
		const recorder = mediaRecorderRef.current;
		if (!recorder || recorder.state !== "recording") return;

		// Grab the final duration before stopping
		const finalDuration = recordingTime;

		recorder.addEventListener(
			"stop",
			async () => {
				if (timerRef.current) {
					clearInterval(timerRef.current);
					timerRef.current = null;
				}
				if (silenceTimerRef.current) {
					clearTimeout(silenceTimerRef.current);
					silenceTimerRef.current = null;
				}

				const audioBlob = new Blob(audioChunksRef.current, {
					type: "audio/webm",
				});
				const audioFile = new File([audioBlob], `voice-${Date.now()}.webm`, {
					type: "audio/webm",
				});

				const voiceData: VoiceData = {
					name: audioFile.name,
					size: audioFile.size,
					type: audioFile.type,
					duration: finalDuration,
					uploading: true,
				};

				setLocalVoice(voiceData);
				setIsRecording(false);
				setIsUploading(true);

				try {
					const backendUrl = await backend.helperState.fileToUrl(
						audioFile,
						false,
					);

					const uploaded: VoiceData = {
						...voiceData,
						backendUrl,
						uploading: false,
					};

					setLocalVoice(uploaded);
					setIsUploading(false);

					if (component.value && "path" in component.value) {
						setByPath(component.value.path, uploaded);
					}

					onAction?.({
						type: "userAction",
						name: "change",
						surfaceId,
						sourceComponentId: componentId,
						timestamp: Date.now(),
						context: {
							value: toStoredVoice(uploaded),
							signedUrls: backendUrl,
							duration: finalDuration,
						},
					});

					await triggerAction(component.actions, {
						signedUrls: backendUrl,
						duration: finalDuration,
					});
				} catch (err) {
					const errored: VoiceData = {
						...voiceData,
						uploading: false,
						uploadError: "Upload failed",
					};
					setLocalVoice(errored);
					setIsUploading(false);
				}
			},
			{ once: true },
		);

		recorder.stop();
	}, [
		recordingTime,
		backend.helperState,
		component.actions,
		component.value,
		componentId,
		onAction,
		setByPath,
		surfaceId,
		triggerAction,
	]);

	const clearRecording = useCallback(() => {
		setLocalVoice(null);
		setRecordingTime(0);
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, null);
		}
		onAction?.({
			type: "userAction",
			name: "change",
			surfaceId,
			sourceComponentId: componentId,
			timestamp: Date.now(),
			context: { value: null },
		});
	}, [component.value, componentId, onAction, setByPath, surfaceId]);

	// Clear event listener
	useEffect(() => {
		const handleClear = (
			event: CustomEvent<{ surfaceId: string; componentId: string }>,
		) => {
			if (
				event.detail.surfaceId === surfaceId &&
				event.detail.componentId === componentId
			) {
				clearRecording();
			}
		};
		window.addEventListener(
			"a2ui:clearFileInput" as never,
			handleClear as EventListener,
		);
		return () => {
			window.removeEventListener(
				"a2ui:clearFileInput" as never,
				handleClear as EventListener,
			);
		};
	}, [surfaceId, componentId, clearRecording]);

	const containerStyle = resolveStyle(style);
	const inlineStyle = resolveInlineStyle(style);

	return (
		<div className={cn("space-y-2", containerStyle)} style={inlineStyle}>
			{label && <Label className="text-sm font-medium">{label}</Label>}

			<div
				className={cn(
					"relative rounded-xl border overflow-hidden transition-all duration-300",
					isRecording
						? "border-red-500/50 bg-linear-to-b from-red-500/5 to-transparent shadow-lg shadow-red-500/5"
						: display
							? "border-primary/30 bg-linear-to-b from-primary/5 to-transparent"
							: "border-border bg-background hover:border-primary/30",
					error && "border-destructive",
					disabled && "opacity-50 pointer-events-none",
				)}
			>
				{/* Main content area */}
				<div className="flex flex-col items-center justify-center p-6 min-h-40">
					{isRecording ? (
						<>
							{/* Live visualizer */}
							<div className="w-full mb-4">
								<WaveformVisualizer
									analyser={analyserNode}
									isRecording={isRecording}
									visualizer={visualizer}
								/>
							</div>

							{/* Recording controls */}
							<div className="flex items-center gap-4">
								<button type="button" onClick={stopRecording} className="group">
									<RecordingPulseRing duration={recordingTime} />
								</button>
							</div>

							{maxDuration < 300 && (
								<div className="mt-8 w-full">
									<div className="h-1 bg-muted/30 rounded-full overflow-hidden">
										<div
											className="h-full bg-linear-to-r from-red-500 to-rose-400 rounded-full transition-all duration-1000"
											style={{
												width: `${(recordingTime / maxDuration) * 100}%`,
											}}
										/>
									</div>
								</div>
							)}
						</>
					) : display ? (
						<>
							{/* Recorded state */}
							<div className="flex items-center gap-4 w-full">
								<div className="shrink-0 w-12 h-12 rounded-full bg-linear-to-br from-violet-500 to-blue-500 flex items-center justify-center shadow-md">
									<Mic className="w-5 h-5 text-white" />
								</div>
								<div className="flex-1 min-w-0">
									<p className="text-sm font-medium truncate">{display.name}</p>
									<p className="text-xs text-muted-foreground">
										{formatDuration(display.duration)} &middot;{" "}
										{formatFileSize(display.size)}
									</p>
								</div>
								{display.uploading || isTriggering ? (
									<Loader2 className="w-5 h-5 animate-spin text-primary" />
								) : display.uploadError ? (
									<span className="text-xs text-destructive">
										{display.uploadError}
									</span>
								) : (
									<Button
										type="button"
										size="sm"
										variant="ghost"
										className="h-8 w-8 p-0 rounded-full hover:bg-destructive/10 hover:text-destructive"
										onClick={clearRecording}
									>
										<Trash2 className="w-4 h-4" />
									</Button>
								)}
							</div>
						</>
					) : (
						<>
							{/* Idle state - click to record */}
							<button
								type="button"
								onClick={startRecording}
								disabled={
									disabled ||
									typeof navigator?.mediaDevices?.getUserMedia !== "function"
								}
								className="group focus:outline-none"
							>
								<IdlePulseRing />
							</button>
							<p className="mt-8 text-sm text-muted-foreground">
								Click to start recording
							</p>
						</>
					)}
				</div>
			</div>

			{helperText && !error && (
				<p className="text-xs text-muted-foreground">{helperText}</p>
			)}
			{error && (
				<p className="text-xs text-destructive">
					{typeof error === "string" ? error : "Recording error"}
				</p>
			)}
		</div>
	);
}
