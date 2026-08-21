"use client";

import { i18n as i18next } from "@flow-like/locales";
import {
	AudioLines,
	AudioWaveform,
	CornerDownRight,
	FileIcon,
	FolderIcon,
	MicIcon,
	Plus,
	Send,
	SquareIcon,
	WrenchIcon,
	X,
} from "lucide-react";
import {
	forwardRef,
	useCallback,
	useEffect,
	useImperativeHandle,
	useRef,
	useState,
} from "react";
import { cn, humanFileSize } from "../../../lib";
import {
	Button,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Textarea,
} from "../../ui";
import {
	type VoiceInvokeMode,
	type VoiceMode,
	useSpeechRecognition,
	useVoiceRecorder,
} from "../../voice";
import { FileManagerDialog } from "./chatbox/file-dialog";
import { isImageFile, useImagePreviewUrls } from "./chatbox/image-previews";

export type ISendMessageFunction = (
	content: string,
	filesAttached?: File[],
	activeTools?: string[],
	audioFile?: File,
) => Promise<void>;

interface ChatBoxProps {
	onSendMessage: ISendMessageFunction;
	onContentChange?: (
		content: string,
		filesAttached?: File[],
		activeTools?: string[],
	) => void;
	availableTools?: string[];
	defaultActiveTools?: string[];
	fileUpload: boolean;
	audioInput: boolean;
	voiceMode?: VoiceMode;
	voiceInvoke?: VoiceInvokeMode;
	voiceMaxDuration?: number;
	sendDisabled?: boolean;
	/** Tooltip/aria override for the send button when a plain "Send" would be misleading —
	 * e.g. "Queue this message" once every response slot is busy. */
	sendHint?: string;
	/**
	 * Push the composer's text into the turn that is already running instead of starting a new
	 * one. Present only while something is generating; renders the extra "add to running response"
	 * button next to Send. Resolves false when the backend refused it, in which case the text is
	 * restored so nothing is silently lost.
	 */
	onSteer?: (content: string) => Promise<boolean>;
	onVoiceModeToggle?: () => void;
	/** Called when the user starts speaking/recording, to interrupt answer playback. */
	onInterrupt?: () => void;
}

export interface ChatBoxRef {
	setInput: (text: string) => void;
	clearInput?: () => void;
	addFile?: (file: File) => void;
	addFiles?: (files: File[]) => void;
	removeFile?: (index: number) => void;
	removeFiles?: (indices: number[]) => void;
	toggleTool?: (tool: string) => void;
	getInput: () => string;
	getAttachedFiles: () => File[];
	getActiveTools: () => string[];
	setActiveTools?: (tools: string[]) => void;
	focusInput?: () => void;
	blurInput?: () => void;
}

export const ChatBox = forwardRef<ChatBoxRef, ChatBoxProps>(
	(
		{
			onContentChange,
			onSendMessage,
			fileUpload = true,
			audioInput = false,
			voiceMode = "record",
			voiceInvoke = "manual",
			voiceMaxDuration = 0,
			availableTools = ["Reason"],
			defaultActiveTools = ["Reason"],
			sendDisabled = false,
			sendHint,
			onSteer,
			onVoiceModeToggle,
			onInterrupt,
		}: Readonly<ChatBoxProps>,
		ref,
	) => {
		const [input, setInput] = useState("");
		const [activeTools, setActiveTools] =
			useState<string[]>(defaultActiveTools);
		const [attachedFiles, setAttachedFiles] = useState<File[]>([]);
		const [showFileManager, setShowFileManager] = useState(false);
		const previewUrls = useImagePreviewUrls(attachedFiles);

		const [recordedAudio, setRecordedAudio] = useState<File | null>(null);
		const [recordedDuration, setRecordedDuration] = useState(0);
		const [isSteering, setIsSteering] = useState(false);
		const [voiceError, setVoiceError] = useState<string | null>(null);
		const [speechFailed, setSpeechFailed] = useState(false);

		const chatboxRef = useRef<HTMLTextAreaElement | null>(null);
		const transcriptionBaseRef = useRef("");

		const recorder = useVoiceRecorder({
			maxDuration: voiceMaxDuration,
			stopDelay: 700,
			onComplete: (file, duration) => {
				setRecordedAudio(file);
				setRecordedDuration(duration);
			},
			onError: (error) => {
				console.error("Error accessing microphone:", error);
				setVoiceError("Microphone access failed. Check the app's permissions.");
			},
		});

		const speech = useSpeechRecognition({
			onResult: (finalText, interimText) => {
				setInput(
					[transcriptionBaseRef.current, finalText, interimText]
						.map((part) => part.trim())
						.filter(Boolean)
						.join(" "),
				);
			},
			onEnd: (finalText) => {
				if (!finalText) return;
				setInput(
					[transcriptionBaseRef.current, finalText]
						.map((part) => part.trim())
						.filter(Boolean)
						.join(" "),
				);
			},
			onError: (error) => {
				console.error("Error transcribing speech:", error);
				setSpeechFailed(true);
				setVoiceError(
					i18next.t(
						"speechRecognitionIsUnavailableYouCanRecordAudioInstead",
						"Speech recognition is unavailable. You can record audio instead.",
					),
				);
			},
		});

		const effectiveVoiceMode: VoiceMode =
			voiceMode === "stt" && speech.isSupported && !speechFailed
				? "stt"
				: "record";
		const isRecording = recorder.isRecording || recorder.isArming;
		const isTranscribing = speech.isListening;

		useEffect(() => {
			setSpeechFailed(false);
			setVoiceError(null);
		}, [voiceMode]);

		useImperativeHandle(
			ref,
			() => ({
				setInput: (text: string) => {
					setInput(text);
				},
				addFile: (file: File) => {
					if (!fileUpload) return;
					setAttachedFiles((prev) => [...prev, file]);
				},
				addFiles: (files: File[]) => {
					if (!fileUpload) return;
					setAttachedFiles((prev) => [...prev, ...files]);
				},
				removeFile: (index: number) => {
					setAttachedFiles((prev) => prev.filter((_, i) => i !== index));
				},
				removeFiles: (indices: number[]) => {
					setAttachedFiles((prev) =>
						prev.filter((_, i) => !indices.includes(i)),
					);
				},
				toggleTool: (tool: string) => {
					setActiveTools((prev) =>
						prev.includes(tool)
							? prev.filter((t) => t !== tool)
							: [...prev, tool],
					);
				},
				clearInput: () => {
					setInput("");
					setAttachedFiles([]);
					setRecordedAudio(null);
					setRecordedDuration(0);
				},
				getInput: () => input,
				getAttachedFiles: () => attachedFiles,
				getActiveTools: () => activeTools,
				setActiveTools: (tools: string[]) => {
					setActiveTools(tools);
				},
				focusInput: () => {
					if (chatboxRef.current) {
						chatboxRef.current.focus();
					}
				},
				blurInput: () => {
					try {
						chatboxRef.current?.blur();
					} catch {}
				},
			}),
			[],
		);

		/**
		 * Send the composer's text into the turn already running. Clears optimistically like a normal
		 * send; restores the text if the run refused it, so a rejected instruction is never lost.
		 */
		const handleSteer = async () => {
			const message = input.trim();
			if (!onSteer || !message || isSteering || isRecording || isTranscribing) {
				return;
			}
			setIsSteering(true);
			setInput("");
			try {
				const delivered = await onSteer(message);
				if (!delivered) setInput(message);
			} catch {
				setInput(message);
			} finally {
				setIsSteering(false);
			}
		};

		const handleSubmit = async (e: React.FormEvent) => {
			e.preventDefault();
			if (
				sendDisabled ||
				isRecording ||
				isTranscribing ||
				(!input.trim() && !recordedAudio)
			) {
				return;
			}

			const message = input.trim();
			const files = attachedFiles;
			const audio = recordedAudio || undefined;
			const audioDuration = recordedDuration;

			// Clear the composer immediately on send — onSendMessage may not resolve until the
			// whole response has streamed back, and leaving the sent text sitting there reads as
			// "nothing happened". Restore it if the send fails so the user can retry.
			setInput("");
			setAttachedFiles([]);
			setRecordedAudio(null);
			setRecordedDuration(0);
			// Dismiss the iOS keyboard and revert any zoom
			try {
				chatboxRef.current?.blur();
			} catch {}

			try {
				await onSendMessage(message, files, activeTools, audio);
			} catch (error) {
				setInput(message);
				setAttachedFiles(files);
				setRecordedAudio(audio ?? null);
				setRecordedDuration(audioDuration);
				throw error;
			}
		};

		useEffect(() => {
			if (onContentChange) {
				onContentChange(input, attachedFiles, activeTools);
			}
		}, [input, attachedFiles, activeTools, onContentChange]);

		const startRecording = () => {
			if (!audioInput) return;
			onInterrupt?.();
			setVoiceError(null);
			void recorder.start();
		};

		const stopRecording = () => {
			recorder.stop();
		};

		const cancelRecording = () => {
			recorder.cancel();
			setRecordedAudio(null);
			setRecordedDuration(0);
		};

		const removeRecordedAudio = () => {
			setRecordedAudio(null);
			setRecordedDuration(0);
		};

		const startTranscription = () => {
			if (!audioInput || effectiveVoiceMode !== "stt") return;
			onInterrupt?.();
			setVoiceError(null);
			transcriptionBaseRef.current = input.trim();
			speech.reset();
			speech.start();
		};

		const stopTranscription = () => {
			speech.stop();
		};

		const formatTime = (seconds: number) => {
			const mins = Math.floor(seconds / 60);
			const secs = seconds % 60;
			return `${mins}:${secs.toString().padStart(2, "0")}`;
		};

		const handleKeyDown = (e: React.KeyboardEvent) => {
			if (e.key === "Enter" && !e.shiftKey) {
				e.preventDefault();
				void handleSubmit(e);
			}
		};

		const handleFileUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
			if (!fileUpload) return;
			const files = e.target.files;
			if (files) {
				const fileArray = Array.from(files);
				setAttachedFiles((prev) => [...prev, ...fileArray]);
			}
		};

		const addFiles = useCallback(
			(files: File[]) => {
				if (!fileUpload) return;
				setAttachedFiles((prev) => [...prev, ...files]);
			},
			[fileUpload],
		);

		const handlePaste = useCallback(
			(e: React.ClipboardEvent) => {
				const items = Array.from(e.clipboardData.items);
				const files: File[] = [];

				for (const item of items) {
					if (item.kind === "file") {
						const file = item.getAsFile();
						if (file) {
							files.push(file);
						}
					}
				}

				if (files.length > 0) {
					e.preventDefault();
					addFiles(files);
				}
			},
			[addFiles],
		);

		const handleDrop = useCallback(
			(e: React.DragEvent) => {
				e.preventDefault();
				const items = Array.from(e.dataTransfer.items);
				const files: File[] = [];

				const processEntry = async (entry: FileSystemEntry) => {
					if (entry.isFile) {
						const fileEntry = entry as FileSystemFileEntry;
						return new Promise<void>((resolve) => {
							fileEntry.file((file) => {
								files.push(file);
								resolve();
							});
						});
					} else if (entry.isDirectory) {
						const dirEntry = entry as FileSystemDirectoryEntry;
						const reader = dirEntry.createReader();
						return new Promise<void>((resolve) => {
							reader.readEntries(async (entries) => {
								await Promise.all(entries.map(processEntry));
								resolve();
							});
						});
					}
				};

				Promise.all(
					items.map((item) => {
						const entry = item.webkitGetAsEntry();
						return entry ? processEntry(entry) : Promise.resolve();
					}),
				).then(() => {
					if (files.length > 0) {
						addFiles(files);
					}
				});
			},
			[addFiles],
		);

		const handleDragOver = useCallback((e: React.DragEvent) => {
			e.preventDefault();
		}, []);

		const handleRemoveFile = (index: number) => {
			setAttachedFiles((prev) => prev.filter((_, i) => i !== index));
		};

		const handleRemoveFiles = (indices: number[]) => {
			setAttachedFiles((prev) => prev.filter((_, i) => !indices.includes(i)));
		};

		const handleToolToggle = (tool: string) => {
			setActiveTools((prev) =>
				prev.includes(tool) ? prev.filter((t) => t !== tool) : [...prev, tool],
			);
		};

		return (
			<div className="w-full max-w-screen-xl px-2">
				{/* Attachments Preview */}
				{(activeTools.length > 0 ||
					attachedFiles.length > 0 ||
					recordedAudio) && (
					<div className="mb-3 space-y-2">
						{/* Recorded Audio Preview */}
						{recordedAudio && (
							<div className="flex items-center gap-2 p-2 bg-background border border-border rounded-lg">
								<div className="w-8 h-8 bg-primary/10 rounded flex items-center justify-center flex-shrink-0">
									<MicIcon className="w-4 h-4 text-primary" />
								</div>
								<div className="flex flex-col min-w-0 flex-1">
									<span className="text-xs font-medium">
										{i18next.t("audioRecording", "Audio Recording")}
									</span>
									<span className="text-xs text-muted-foreground">
										{formatTime(recordedDuration)} •{" "}
										{(recordedAudio.size / 1024).toFixed(1)} KB
									</span>
								</div>
								<Button
									type="button"
									size="sm"
									variant="ghost"
									className="h-5 w-5 p-0 rounded-full hover:bg-destructive hover:text-destructive-foreground flex-shrink-0"
									onClick={removeRecordedAudio}
								>
									<X className="w-3 h-3" />
								</Button>
							</div>
						)}

						{/* Attached files — one quiet chip per file, wrapping inline. */}
						{attachedFiles.length > 0 && (
							<div className="flex flex-wrap items-center gap-1.5">
								{attachedFiles.slice(0, 6).map((file, index) => (
									<div
										key={`${file.name}-${index}`}
										className="group flex h-8 min-w-0 max-w-56 items-center gap-2 rounded-full border py-0 pr-1 pl-1.5 transition-colors hover:border-ring"
										style={{
											borderColor: "var(--fl-chat-rule, var(--border))",
										}}
									>
										{isImageFile(file) ? (
											<img
												src={previewUrls.get(file)}
												alt={file.name}
												className="size-6 shrink-0 rounded-full object-cover"
											/>
										) : (
											<span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-muted">
												<FileIcon className="size-3 text-muted-foreground" />
											</span>
										)}
										<span className="min-w-0 flex-1 truncate text-xs font-medium">
											{file.name}
										</span>
										<span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
											{humanFileSize(file.size, true)}
										</span>
										<Button
											type="button"
											size="sm"
											variant="ghost"
											aria-label={i18next.t("removeName", "Remove {{name}}", {
												name: file.name,
											})}
											className="size-6 shrink-0 rounded-full p-0 text-muted-foreground opacity-60 transition-opacity hover:bg-destructive hover:text-destructive-foreground group-hover:opacity-100"
											onClick={() => handleRemoveFile(index)}
										>
											<X className="size-3" />
										</Button>
									</div>
								))}

								{attachedFiles.length > 6 && (
									<Button
										type="button"
										size="sm"
										variant="ghost"
										className="h-8 rounded-full px-3 text-xs text-muted-foreground"
										onClick={() => setShowFileManager(true)}
									>
										+{attachedFiles.length - 6} more
									</Button>
								)}

								{attachedFiles.length > 1 && (
									<Button
										type="button"
										size="sm"
										variant="ghost"
										className="h-8 rounded-full px-3 text-xs text-muted-foreground hover:text-destructive"
										onClick={() => setAttachedFiles([])}
									>
										{i18next.t("clearAll", "Clear all")}
									</Button>
								)}
							</div>
						)}
					</div>
				)}

				{/* File Manager Dialog */}
				<FileManagerDialog
					open={showFileManager}
					onOpenChange={setShowFileManager}
					files={attachedFiles}
					onRemoveFile={handleRemoveFile}
					onRemoveFiles={handleRemoveFiles}
					onClearAll={() => setAttachedFiles([])}
				/>

				<form
					onSubmit={handleSubmit}
					className="relative"
					data-fl-chat-composer
				>
					<div
						className="flex flex-col items-start overflow-hidden rounded-xl border border-border shadow-sm transition-all duration-200 focus-within:border-input focus-within:ring-2 focus-within:ring-ring"
						onDrop={handleDrop}
						onDragOver={handleDragOver}
						style={{
							backgroundColor:
								"var(--fl-chat-composer-background, var(--background))",
						}}
					>
						{/* Text Input */}
						<div className="flex-1 py-2 w-full pr-2">
							<Textarea
								aria-label="Message"
								ref={chatboxRef}
								value={input}
								onChange={(e) => setInput(e.target.value)}
								onKeyDown={handleKeyDown}
								onPaste={handlePaste}
								placeholder={i18next.t("typeAMessage", "Type a message...")}
								className="border-0 focus:ring-0 resize-none bg-transparent! placeholder:text-muted-foreground text-[16px] sm:text-sm leading-relaxed min-h-[44px] max-h-[180px] overflow-y-auto w-full"
								rows={Math.min(5, Math.max(1, input.split("\n").length))}
								style={{
									boxShadow: "none",
									outline: "none",
								}}
								// iOS keyboard and input behavior tweaks
								inputMode="text"
								enterKeyHint="send"
								autoCapitalize="sentences"
								autoCorrect="on"
								spellCheck
							/>
						</div>

						{/* Tool bar and settings */}
						<div className="flex w-full items-center justify-between rounded-b-xl">
							{/* Left side buttons */}
							<div className="flex items-center gap-1 p-2 pt-0">
								{/* File Upload Button */}
								{fileUpload && (
									<Popover>
										<PopoverTrigger asChild>
											<Button
												aria-label={i18next.t(
													"addAttachment",
													"Add attachment",
												)}
												type="button"
												size="sm"
												variant="ghost"
												className="h-11 w-11 sm:h-8 sm:w-8 p-0 hover:bg-accent rounded-lg transition-colors"
											>
												<Plus className="w-4 h-4 text-muted-foreground" />
											</Button>
										</PopoverTrigger>
										<PopoverContent
											side="top"
											align="start"
											className="mb-2 w-52 rounded-xl p-1.5"
										>
											<input
												type="file"
												id="file-upload"
												className="hidden"
												onChange={handleFileUpload}
												multiple
											/>
											<input
												type="file"
												id="folder-upload"
												className="hidden"
												onChange={handleFileUpload}
												multiple
												// @ts-ignore
												directory=""
												webkitdirectory=""
											/>
											<div className="space-y-0.5">
												<label
													htmlFor="file-upload"
													className="flex cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition-colors hover:bg-accent"
												>
													<FileIcon className="size-4 text-muted-foreground" />
													{i18next.t("files", "Files")}
												</label>
												<label
													htmlFor="folder-upload"
													className="flex cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition-colors hover:bg-accent"
												>
													<FolderIcon className="size-4 text-muted-foreground" />
													{i18next.t("folder", "Folder")}
												</label>
											</div>
										</PopoverContent>
									</Popover>
								)}

								{/* Tools Settings Button with Active Tools Badge */}
								{(availableTools?.length ?? 0) > 0 && (
									<div className="relative">
										<Popover>
											<PopoverTrigger asChild>
												<Button
													aria-label={i18next.t("chooseTools", "Choose tools")}
													type="button"
													size="sm"
													variant="ghost"
													className="h-11 w-11 sm:h-8 sm:w-8 p-0 hover:bg-accent rounded-lg transition-colors relative"
												>
													<WrenchIcon className="w-4 h-4 text-muted-foreground" />
												</Button>
											</PopoverTrigger>
											<PopoverContent side="top" className="w-48 p-2 mb-2">
												<div className="space-y-1">
													<div className="text-xs font-medium text-muted-foreground px-2 pb-1">
														{i18next.t("tools", "Tools")}
													</div>
													{availableTools.map((tool) => (
														<div
															key={tool}
															className="flex items-center gap-2 p-2 hover:bg-accent rounded cursor-pointer transition-colors"
															onClick={() => handleToolToggle(tool)}
														>
															<div
																className={`w-2 h-2 rounded-full transition-colors ${
																	activeTools.includes(tool)
																		? "bg-primary"
																		: "bg-muted"
																}`}
															/>
															<span className="text-sm">{tool}</span>
															{activeTools.includes(tool) && (
																<span className="text-xs text-primary ml-auto">
																	✓
																</span>
															)}
														</div>
													))}
												</div>
											</PopoverContent>
										</Popover>
									</div>
								)}

								{/* Active Tools */}
								{activeTools.length > 0 && (
									<div className="flex items-center gap-2 flex-wrap ml-2">
										{activeTools.map((tool) => (
											<span
												key={tool}
												className="inline-flex items-center gap-1.5 px-2 py-1 bg-primary/10 text-primary text-xs rounded-full border border-primary/20"
											>
												<div className="w-1.5 h-1.5 bg-primary rounded-full" />
												{tool}
											</span>
										))}
									</div>
								)}

								{/* Attachments Count Badge */}
								{attachedFiles.length > 0 && (
									<div className="relative group">
										<button
											type="button"
											onClick={() => setAttachedFiles([])}
											className="flex items-center gap-1 px-2 py-1 bg-accent/50 hover:bg-destructive hover:text-destructive-foreground rounded-full transition-colors cursor-pointer"
											title={i18next.t(
												"clearAllAttachments",
												"Clear all attachments",
											)}
										>
											<FileIcon className="w-3 h-3 text-muted-foreground group-hover:text-destructive-foreground" />
											<span className="text-xs text-muted-foreground group-hover:text-destructive-foreground font-medium">
												{attachedFiles.length}
											</span>
										</button>
										<div className="absolute -top-1 -right-1 w-4 h-4 bg-background border border-border rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity hover:bg-destructive hover:text-destructive-foreground">
											<X className="w-2.5 h-2.5" />
										</div>
									</div>
								)}
							</div>

							{/* Send Button & Audio Controls */}
							<div className="p-2 pt-0 flex items-center gap-2">
								{/* Voice-to-text transcription button (STT mode) */}
								{audioInput && effectiveVoiceMode === "stt" && (
									<Button
										aria-label={
											isTranscribing
												? i18next.t(
														"stopVoiceTranscription",
														"Stop voice transcription",
													)
												: i18next.t(
														"startVoiceTranscription",
														"Start voice transcription",
													)
										}
										type="button"
										size="sm"
										variant={isTranscribing ? "destructive" : "ghost"}
										className={cn(
											"h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full transition-colors",
											isTranscribing && "animate-pulse",
										)}
										onClick={
											isTranscribing ? stopTranscription : startTranscription
										}
										disabled={isRecording || sendDisabled}
									>
										<AudioLines className="w-4 h-4" />
									</Button>
								)}

								{/* Audio Recording Button (record mode) */}
								{audioInput && effectiveVoiceMode === "record" && (
									<div className="flex items-center gap-1">
										{voiceInvoke === "hold" ? (
											<>
												<Button
													aria-label={i18next.t(
														"holdToRecord",
														"Hold to record",
													)}
													type="button"
													size="sm"
													variant={isRecording ? "destructive" : "ghost"}
													className={cn(
														"h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full select-none transition-colors",
														isRecording ? "animate-pulse" : "hover:bg-accent",
													)}
													disabled={
														!!recordedAudio ||
														sendDisabled ||
														!recorder.isSupported
													}
													onPointerDown={() => startRecording()}
													onPointerUp={() => stopRecording()}
													onPointerLeave={() => {
														if (isRecording) stopRecording();
													}}
													onContextMenu={(e) => e.preventDefault()}
												>
													<MicIcon className="w-4 h-4" />
												</Button>
												{isRecording && (
													<span className="text-xs text-muted-foreground font-mono">
														{formatTime(recorder.recordingTime)}
													</span>
												)}
											</>
										) : isRecording ? (
											<>
												<Button
													aria-label={i18next.t(
														"stopAudioRecording",
														"Stop audio recording",
													)}
													type="button"
													size="sm"
													variant="destructive"
													className="h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full animate-pulse"
													onClick={stopRecording}
												>
													<SquareIcon className="w-3 h-3" />
												</Button>
												<Button
													aria-label={i18next.t(
														"cancelAudioRecording",
														"Cancel audio recording",
													)}
													type="button"
													size="sm"
													variant="ghost"
													className="h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full"
													onClick={cancelRecording}
												>
													<X className="w-3 h-3" />
												</Button>
												<span className="text-xs text-muted-foreground font-mono">
													{formatTime(recorder.recordingTime)}
												</span>
											</>
										) : (
											<Button
												aria-label={i18next.t(
													"startAudioRecording",
													"Start audio recording",
												)}
												disabled={
													!!recordedAudio ||
													sendDisabled ||
													!recorder.isSupported
												}
												type="button"
												size="sm"
												variant="ghost"
												className="h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full hover:bg-accent transition-colors"
												onClick={startRecording}
											>
												<MicIcon className="w-4 h-4 text-muted-foreground" />
											</Button>
										)}
									</div>
								)}

								{/* Voice Mode Button */}
								{audioInput && onVoiceModeToggle && (
									<Button
										type="button"
										size="sm"
										variant="ghost"
										className="h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full hover:bg-violet-500/10 hover:text-violet-500 transition-colors"
										onClick={onVoiceModeToggle}
										disabled={isRecording || isTranscribing || sendDisabled}
										title={i18next.t("voiceMode", "Voice mode")}
									>
										<AudioWaveform className="w-4 h-4" />
									</Button>
								)}

								{onSteer && (
									<Button
										aria-label={i18next.t(
											"addToTheRunningResponse",
											"Add to the running response",
										)}
										title={i18next.t(
											"addToTheRunningResponseFlowpilotPicksItUpWithoutRestarting",
											"Add to the running response — FlowPilot picks it up without restarting",
										)}
										type="button"
										size="sm"
										disabled={
											sendDisabled ||
											!input.trim() ||
											isSteering ||
											isRecording ||
											isTranscribing
										}
										variant="secondary"
										onClick={handleSteer}
										className="h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full transition-all duration-200"
									>
										<CornerDownRight className="w-4 h-4" />
									</Button>
								)}

								<Button
									aria-label={
										sendHint ?? i18next.t("sendMessage", "Send message")
									}
									title={sendHint}
									type="submit"
									size="sm"
									disabled={
										sendDisabled ||
										isRecording ||
										isTranscribing ||
										(!input.trim() && !recordedAudio)
									}
									variant={
										input.trim() || recordedAudio ? "default" : "secondary"
									}
									className="h-11 w-11 sm:h-8 sm:w-8 p-0 rounded-full transition-all duration-200"
								>
									<Send className="w-4 h-4" />
								</Button>
							</div>
						</div>
					</div>
				</form>
				{voiceError && (
					<p className="mt-2 px-2 text-xs text-destructive" role="alert">
						{voiceError}
					</p>
				)}
			</div>
		);
	},
);
