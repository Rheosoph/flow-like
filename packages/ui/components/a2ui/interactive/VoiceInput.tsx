"use client";

import { Loader2, Mic, Trash2 } from "lucide-react";
import {
	type MouseEvent as ReactMouseEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { cn } from "../../../lib/utils";
import {
	type ITemporaryFlowPath,
	useBackend,
} from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Label } from "../../ui/label";
import {
	AudioPlayback,
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
	useActionContext,
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
	url?: string;
	backendUrl?: string;
	flowPath?: ITemporaryFlowPath;
	transcript?: string;
	uploading?: boolean;
	uploadError?: string;
}

/** Trailing buffer so a manual/hold stop doesn't clip the user's last words. */
const STOP_DELAY_MS = 700;

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
	const { appId, resolveTemporaryUploadTarget } = useActionContext();
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
	const resultModeRaw = useResolved<string>(component.resultMode) ?? "player";
	const resultMode =
		resultModeRaw === "summary"
			? "summary"
			: resultModeRaw === "autoplay"
				? "autoplay"
				: "player";

	const [localVoice, setLocalVoice] = useState<VoiceData | null>(null);
	const [isUploading, setIsUploading] = useState(false);
	const [hover, setHover] = useState(false);
	const [localUrl, setLocalUrl] = useState<string | null>(null);
	const localUrlRef = useRef<string | null>(null);
	const uploadOperationRef = useRef(0);
	const [dismissedResponse, setDismissedResponse] = useState<string | null>(
		null,
	);

	const replaceLocalUrl = useCallback((file: File | null) => {
		if (localUrlRef.current) URL.revokeObjectURL(localUrlRef.current);
		const url = file ? URL.createObjectURL(file) : null;
		localUrlRef.current = url;
		setLocalUrl(url);
	}, []);

	useEffect(
		() => () => {
			uploadOperationRef.current += 1;
			if (localUrlRef.current) URL.revokeObjectURL(localUrlRef.current);
		},
		[],
	);

	const display = localVoice || value || null;
	// The user's own recording.
	const recordingSrc = localUrl ?? display?.backendUrl ?? null;
	// A response the backend pushed onto this element (e.g. via Set Media Source).
	const responseMedia =
		(useResolved<string>(component.src) ??
			useResolved<string>(component.url) ??
			null) ||
		null;
	const responseMediaRef = useRef<string | null>(null);
	responseMediaRef.current = responseMedia;
	// Ignore a stale response once the user records again, until a new one arrives.
	const activeResponse =
		responseMedia && responseMedia !== dismissedResponse ? responseMedia : null;

	const handleRecorded = useCallback(
		async (file: File, duration: number) => {
			const operationId = ++uploadOperationRef.current;
			const voiceData: VoiceData = {
				name: file.name,
				size: file.size,
				type: file.type,
				duration,
				uploading: true,
			};
			replaceLocalUrl(file);
			setLocalVoice(voiceData);
			setIsUploading(true);
			try {
				const executionTarget = await resolveTemporaryUploadTarget?.(
					component.actions?.[0],
				);
				if (uploadOperationRef.current !== operationId) return;
				const temporaryFile = (await backend.helperState.fileToTemporaryFile?.(
					file,
					false,
					appId,
					executionTarget,
				)) ?? {
					url: await backend.helperState.fileToUrl(
						file,
						false,
						appId,
						executionTarget,
					),
				};
				if (uploadOperationRef.current !== operationId) return;
				const uploaded: VoiceData = {
					...voiceData,
					url: temporaryFile.url,
					backendUrl: temporaryFile.url,
					flowPath: temporaryFile.flowPath,
					uploading: false,
				};
				setLocalVoice(uploaded);
				setIsUploading(false);
				if (component.value && "path" in component.value) {
					setByPath(component.value.path, toStoredVoice(uploaded));
				}
				onAction?.({
					type: "userAction",
					name: "change",
					surfaceId,
					sourceComponentId: componentId,
					timestamp: Date.now(),
					context: {
						value: toStoredVoice(uploaded),
						signedUrls: temporaryFile.url,
						duration,
					},
				});
				await triggerAction(component.actions, {
					signedUrls: temporaryFile.url,
					duration,
				});
			} catch {
				if (uploadOperationRef.current !== operationId) return;
				setLocalVoice({
					...voiceData,
					uploading: false,
					uploadError: "Upload failed",
				});
				setIsUploading(false);
			}
		},
		[
			appId,
			backend.helperState,
			component.actions,
			component.value,
			componentId,
			onAction,
			replaceLocalUrl,
			resolveTemporaryUploadTarget,
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
		stopDelay: STOP_DELAY_MS,
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

	useEffect(() => {
		if (mode === "stt" && !speech.isSupported) {
			console.warn(
				"[voiceInput] STT requested but the Web Speech API is unavailable here " +
					"(common in desktop/Tauri webviews); falling back to audio recording. " +
					"Transcribe the recording in the flow instead.",
			);
		}
	}, [mode, speech.isSupported]);

	const capturing =
		effectiveMode === "stt" ? speech.isListening : recorder.isRecording;
	const arming = effectiveMode === "record" && recorder.isArming;
	const autoplayMode = resultMode === "autoplay";
	// In autoplay mode we wait for the backend to push a response before playing —
	// the user's own recording is never auto-played back.
	const awaitingResponse =
		autoplayMode &&
		!activeResponse &&
		!capturing &&
		!arming &&
		!display?.uploadError &&
		(display != null || isTriggering);

	const beginCapture = useCallback(() => {
		if (disabled) return;
		uploadOperationRef.current += 1;
		setIsUploading(false);
		setLocalVoice(null);
		replaceLocalUrl(null);
		setDismissedResponse(responseMediaRef.current);
		if (effectiveMode === "stt") {
			speech.reset();
			speech.start();
		} else {
			void recorder.start();
		}
	}, [disabled, effectiveMode, speech, recorder, replaceLocalUrl]);

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

	const clearRecording = useCallback(
		(executeConfiguredAction: boolean) => {
			uploadOperationRef.current += 1;
			setIsUploading(false);
			setLocalVoice(null);
			replaceLocalUrl(null);
			setDismissedResponse(responseMediaRef.current);
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
			if (executeConfiguredAction) {
				void triggerAction(component.actions, {
					signedUrls: null,
					transcript: null,
					duration: 0,
				});
			}
		},
		[
			component.actions,
			component.value,
			componentId,
			onAction,
			replaceLocalUrl,
			setByPath,
			surfaceId,
			triggerAction,
		],
	);

	useEffect(() => {
		const handleClear = (event: Event) => {
			const { detail } = event as CustomEvent<{
				surfaceId: string;
				componentId: string;
			}>;
			if (
				detail.surfaceId === surfaceId &&
				detail.componentId === componentId
			) {
				clearRecording(false);
			}
		};
		window.addEventListener("a2ui:clearFileInput", handleClear);
		return () => {
			window.removeEventListener("a2ui:clearFileInput", handleClear);
		};
	}, [surfaceId, componentId, clearRecording]);

	const supported =
		effectiveMode === "stt" ? speech.isSupported : recorder.isSupported;
	const blocked = disabled || !supported;

	const visualState: VoiceVisualState = capturing
		? "recording"
		: arming || isUploading || isTriggering
			? "processing"
			: "idle";

	const Visualizer = useMemo(() => getVoiceVisualizer(variant), [variant]);

	const interactionProps =
		invoke === "hold"
			? {
					onPointerEnter: () => recorder.prewarm(),
					onPointerDown: () => beginCapture(),
					onPointerUp: () => endCapture(),
					onPointerLeave: () => {
						if (capturing) endCapture();
					},
					// Suppress the mobile long-press context menu / selection magnifier,
					// which otherwise interrupts the hold-to-record pointer flow.
					onContextMenu: (e: ReactMouseEvent) => e.preventDefault(),
				}
			: {
					onPointerEnter: () => recorder.prewarm(),
					onClick: () => (capturing ? endCapture() : beginCapture()),
				};

	const recordAgainHint =
		invoke === "hold" ? "Hold to record again" : "Tap to record again";

	const hint = arming
		? "Starting…"
		: capturing
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
		<div
			data-card-action-stop
			className={cn("space-y-2", containerStyle)}
			style={inlineStyle}
		>
			{label && <Label className="text-sm font-medium">{label}</Label>}

			<div
				className={cn(
					"relative overflow-hidden rounded-xl border transition-all duration-300",
					capturing || arming
						? "border-primary/40 bg-linear-to-b from-primary/5 to-transparent"
						: display
							? "border-primary/30 bg-linear-to-b from-primary/5 to-transparent"
							: "border-border bg-background hover:border-primary/30",
					error && "border-destructive",
					blocked && "pointer-events-none opacity-50",
				)}
			>
				<div className="flex min-h-40 flex-col items-center justify-center p-6">
					{autoplayMode && activeResponse && !capturing && !arming ? (
						<AudioPlayback
							src={activeResponse}
							variant={variant}
							size={size}
							color={color}
							recordingColor={recordingColor}
							autoPlay
							recordControl={blocked ? undefined : interactionProps}
							recordHint={recordAgainHint}
							onDelete={() => clearRecording(true)}
						/>
					) : awaitingResponse ? (
						<div className="flex w-full flex-col items-center gap-3">
							<div className="flex w-full justify-center">
								<Visualizer
									analyser={null}
									state="processing"
									size={size}
									color={color}
									recordingColor={recordingColor}
								/>
							</div>
							<p className="animate-pulse text-sm text-muted-foreground">
								Processing…
							</p>
							<div className="flex items-center gap-3">
								{!blocked && (
									<Button
										type="button"
										size="icon"
										variant="outline"
										className="size-9 rounded-full"
										style={{
											borderColor: recordingColor,
											color: recordingColor,
										}}
										{...interactionProps}
									>
										<Mic className="size-4" />
										<span className="sr-only">{recordAgainHint}</span>
									</Button>
								)}
								<Button
									type="button"
									size="sm"
									variant="ghost"
									className="size-8 rounded-full p-0 hover:bg-destructive/10 hover:text-destructive"
									onClick={() => clearRecording(true)}
								>
									<Trash2 className="size-4" />
								</Button>
							</div>
						</div>
					) : !autoplayMode &&
						display &&
						!capturing &&
						resultMode !== "summary" &&
						recordingSrc &&
						!display.uploadError ? (
						<AudioPlayback
							src={recordingSrc}
							variant={variant}
							size={size}
							color={color}
							recordingColor={recordingColor}
							title={display.transcript ?? display.name}
							busy={display.uploading || isTriggering}
							recordControl={blocked ? undefined : interactionProps}
							recordHint={recordAgainHint}
							onDelete={() => clearRecording(true)}
						/>
					) : display && !capturing ? (
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
									onClick={() => clearRecording(true)}
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
