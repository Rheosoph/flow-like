"use client";

import { Loader2, Mic, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Label } from "../../ui/label";
import {
	VOICE_DEFAULT_COLOR,
	VOICE_DEFAULT_RECORDING_COLOR,
	type VoiceInvokeMode,
	type VoiceMode,
	type VoiceSize,
	type VoiceVariant,
	type VoiceVisualState,
	getVoiceVisualizer,
	useSpeakerActivity,
	useSpeechRecognition,
	useVoiceRecorder,
} from "../../voice";
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
	transcript?: string;
	uploading?: boolean;
	uploadError?: string;
}

function toStoredVoice(voice: VoiceData): VoiceData {
	const { uploading: _u, uploadError: _e, ...stored } = voice;
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
	const variantProp = useResolved<VoiceVariant>(component.variant);
	const visualizerProp = useResolved<string>(component.visualizer);
	const variant = (variantProp ??
		(visualizerProp as VoiceVariant) ??
		"waveform") as VoiceVariant;
	const size = (useResolved<VoiceSize>(component.size) ?? "md") as VoiceSize;
	const mode = (useResolved<VoiceMode>(component.mode) ??
		"record") as VoiceMode;
	const invoke = (useResolved<VoiceInvokeMode>(component.invoke) ??
		"manual") as VoiceInvokeMode;
	const color = useResolved<string>(component.color) ?? VOICE_DEFAULT_COLOR;
	const recordingColor =
		useResolved<string>(component.recordingColor) ??
		VOICE_DEFAULT_RECORDING_COLOR;

	const [localVoice, setLocalVoice] = useState<VoiceData | null>(null);
	const [isUploading, setIsUploading] = useState(false);
	const [hover, setHover] = useState(false);

	const display = localVoice || value || null;

	const handleRecorded = useCallback(
		async (file: File, duration: number) => {
			const voiceData: VoiceData = {
				name: file.name,
				size: file.size,
				type: file.type,
				duration,
				uploading: true,
			};
			setLocalVoice(voiceData);
			setIsUploading(true);
			try {
				const backendUrl = await backend.helperState.fileToUrl(file, false);
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
						duration,
					},
				});
				await triggerAction(component.actions, {
					signedUrls: backendUrl,
					duration,
				});
			} catch {
				setLocalVoice({
					...voiceData,
					uploading: false,
					uploadError: "Upload failed",
				});
				setIsUploading(false);
			}
		},
		[
			backend.helperState,
			component.actions,
			component.value,
			componentId,
			onAction,
			setByPath,
			surfaceId,
			triggerAction,
		],
	);

	const handleTranscript = useCallback(
		async (text: string) => {
			const trimmed = text.trim();
			if (!trimmed) return;
			const voiceData: VoiceData = {
				name: "transcript",
				size: trimmed.length,
				type: "text/plain",
				duration: 0,
				transcript: trimmed,
			};
			setLocalVoice(voiceData);
			if (component.value && "path" in component.value) {
				setByPath(component.value.path, voiceData);
			}
			onAction?.({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { value: voiceData, transcript: trimmed },
			});
			await triggerAction(component.actions, { transcript: trimmed });
		},
		[
			component.actions,
			component.value,
			componentId,
			onAction,
			setByPath,
			surfaceId,
			triggerAction,
		],
	);

	const recorder = useVoiceRecorder({
		maxDuration,
		onComplete: (file, duration) => {
			void handleRecorded(file, duration);
		},
	});
	const speech = useSpeechRecognition({
		onEnd: (text) => {
			void handleTranscript(text);
		},
	});

	const effectiveMode: VoiceMode =
		mode === "stt" && speech.isSupported ? "stt" : "record";
	const capturing =
		effectiveMode === "stt" ? speech.isListening : recorder.isRecording;

	const beginCapture = useCallback(() => {
		if (disabled) return;
		setLocalVoice(null);
		if (effectiveMode === "stt") {
			speech.reset();
			speech.start();
		} else {
			void recorder.start();
		}
	}, [disabled, effectiveMode, speech, recorder]);

	const endCapture = useCallback(() => {
		if (effectiveMode === "stt") speech.stop();
		else recorder.stop();
	}, [effectiveMode, speech, recorder]);

	useSpeakerActivity({
		analyser: recorder.analyser,
		active:
			effectiveMode === "record" &&
			recorder.isRecording &&
			(autoStop || invoke === "auto"),
		silenceThreshold,
		silenceDuration,
		onSilence: () => recorder.stop(),
	});

	const clearRecording = useCallback(() => {
		setLocalVoice(null);
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

	const supported =
		effectiveMode === "stt" ? speech.isSupported : recorder.isSupported;
	const blocked = disabled || !supported;

	const visualState: VoiceVisualState = capturing
		? "recording"
		: isUploading || isTriggering
			? "processing"
			: "idle";

	const Visualizer = useMemo(() => getVoiceVisualizer(variant), [variant]);

	const interactionProps =
		invoke === "hold"
			? {
					onPointerDown: () => beginCapture(),
					onPointerUp: () => endCapture(),
					onPointerLeave: () => {
						if (capturing) endCapture();
					},
				}
			: {
					onClick: () => (capturing ? endCapture() : beginCapture()),
				};

	const hint = capturing
		? effectiveMode === "stt"
			? speech.transcript || "Listening…"
			: formatDuration(recorder.recordingTime)
		: invoke === "hold"
			? "Hold to record"
			: effectiveMode === "stt"
				? "Tap to dictate"
				: invoke === "auto"
					? "Tap to start — stops when you pause"
					: "Tap to start recording";

	const containerStyle = resolveStyle(style);
	const inlineStyle = resolveInlineStyle(style);

	return (
		<div className={cn("space-y-2", containerStyle)} style={inlineStyle}>
			{label && <Label className="text-sm font-medium">{label}</Label>}

			<div
				className={cn(
					"relative overflow-hidden rounded-xl border transition-all duration-300",
					capturing
						? "border-primary/40 bg-linear-to-b from-primary/5 to-transparent"
						: display
							? "border-primary/30 bg-linear-to-b from-primary/5 to-transparent"
							: "border-border bg-background hover:border-primary/30",
					error && "border-destructive",
					blocked && "pointer-events-none opacity-50",
				)}
			>
				<div className="flex min-h-40 flex-col items-center justify-center p-6">
					{display && !capturing ? (
						<div className="flex w-full items-center gap-4">
							<div
								className="flex size-12 shrink-0 items-center justify-center rounded-full shadow-md"
								style={{ backgroundColor: color }}
							>
								<Mic className="size-5 text-white" />
							</div>
							<div className="min-w-0 flex-1">
								<p className="truncate text-sm font-medium">
									{display.transcript ?? display.name}
								</p>
								{!display.transcript && (
									<p className="text-xs text-muted-foreground">
										{formatDuration(display.duration)} &middot;{" "}
										{formatFileSize(display.size)}
									</p>
								)}
							</div>
							{display.uploading || isTriggering ? (
								<Loader2 className="size-5 animate-spin text-primary" />
							) : display.uploadError ? (
								<span className="text-xs text-destructive">
									{display.uploadError}
								</span>
							) : (
								<Button
									type="button"
									size="sm"
									variant="ghost"
									className="size-8 rounded-full p-0 hover:bg-destructive/10 hover:text-destructive"
									onClick={clearRecording}
								>
									<Trash2 className="size-4" />
								</Button>
							)}
						</div>
					) : (
						<>
							<button
								type="button"
								disabled={blocked}
								className="group flex select-none flex-col items-center gap-3 focus:outline-none"
								onMouseEnter={() => setHover(true)}
								onMouseLeave={() => setHover(false)}
								{...interactionProps}
							>
								<Visualizer
									analyser={recorder.analyser}
									state={visualState}
									size={size}
									color={color}
									recordingColor={recordingColor}
									hover={hover}
								/>
							</button>
							<p className="mt-4 text-sm text-muted-foreground">{hint}</p>
							{capturing &&
								effectiveMode === "record" &&
								maxDuration > 0 &&
								maxDuration < 300 && (
									<div className="mt-4 w-full">
										<div className="h-1 overflow-hidden rounded-full bg-muted/30">
											<div
												className="h-full rounded-full transition-all duration-1000"
												style={{
													width: `${(recorder.recordingTime / maxDuration) * 100}%`,
													backgroundColor: recordingColor,
												}}
											/>
										</div>
									</div>
								)}
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
