"use client";

import {
	AudioLines,
	AudioWaveform,
	FileIcon,
	ImageIcon,
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
import { cn, humanFileSize, sanitizeImageUrl } from "../../../lib";
import {
	Button,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Textarea,
} from "../../ui";
import type { VoiceInvokeMode, VoiceMode } from "../../voice";
import { FileManagerDialog } from "./chatbox/file-dialog";

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
	sendDisabled?: boolean;
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
			availableTools = ["Reason"],
			defaultActiveTools = ["Reason"],
			sendDisabled = false,
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

		const [isRecording, setIsRecording] = useState(false);
		const [recordedAudio, setRecordedAudio] = useState<File | null>(null);
		const [recordingTime, setRecordingTime] = useState(0);
		const [isTranscribing, setIsTranscribing] = useState(false);

		const chatboxRef = useRef<HTMLTextAreaElement | null>(null);
		const mediaRecorderRef = useRef<MediaRecorder | null>(null);
		const recordingIntervalRef = useRef<NodeJS.Timeout | null>(null);
		const audioChunksRef = useRef<Blob[]>([]);
		const recognitionRef = useRef<any>(null);

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
					setRecordingTime(0);
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

		const handleSubmit = async (e: React.FormEvent) => {
			e.preventDefault();
			if (sendDisabled || !input.trim()) {
				return;
			}

			await onSendMessage(
				input.trim(),
				attachedFiles,
				activeTools,
				recordedAudio || undefined,
			);
			setInput("");
			setAttachedFiles([]);
			setRecordedAudio(null);
			setRecordingTime(0);
			// Dismiss the iOS keyboard and revert any zoom
			try {
				chatboxRef.current?.blur();
			} catch {}
		};

		useEffect(() => {
			if (onContentChange) {
				onContentChange(input, attachedFiles, activeTools);
			}
		}, [input, attachedFiles, activeTools, onContentChange]);

		const startRecording = async () => {
			if (!audioInput) return;
			onInterrupt?.();

			try {
				const stream = await navigator.mediaDevices.getUserMedia({
					audio: true,
				});
				mediaRecorderRef.current = new MediaRecorder(stream);
				audioChunksRef.current = [];

				mediaRecorderRef.current.ondataavailable = (event) => {
					if (event.data.size > 0) {
						audioChunksRef.current.push(event.data);
					}
				};

				mediaRecorderRef.current.onstop = () => {
					const audioBlob = new Blob(audioChunksRef.current, {
						type: "audio/webm",
					});
					const audioFile = new File(
						[audioBlob],
						`recording-${Date.now()}.webm`,
						{
							type: "audio/webm",
						},
					);
					setRecordedAudio(audioFile);

					// Stop all tracks to release microphone
					stream.getTracks().forEach((track) => track.stop());
				};

				mediaRecorderRef.current.start();
				setIsRecording(true);
				setRecordingTime(0);

				// Start timer
				recordingIntervalRef.current = setInterval(() => {
					setRecordingTime((prev) => prev + 1);
				}, 1000);
			} catch (error) {
				console.error("Error accessing microphone:", error);
			}
		};

		const stopRecording = () => {
			if (mediaRecorderRef.current && isRecording) {
				mediaRecorderRef.current.stop();
				setIsRecording(false);

				if (recordingIntervalRef.current) {
					clearInterval(recordingIntervalRef.current);
					recordingIntervalRef.current = null;
				}
			}
		};

		const cancelRecording = () => {
			if (mediaRecorderRef.current && isRecording) {
				mediaRecorderRef.current.stop();
				setIsRecording(false);
				setRecordedAudio(null);
				setRecordingTime(0);

				if (recordingIntervalRef.current) {
					clearInterval(recordingIntervalRef.current);
					recordingIntervalRef.current = null;
				}
			}
		};

		const removeRecordedAudio = () => {
			setRecordedAudio(null);
			setRecordingTime(0);
		};

		const startTranscription = () => {
			if (!audioInput) return;
			onInterrupt?.();
			const SpeechRecognitionApi =
				typeof window !== "undefined"
					? (window as any).SpeechRecognition ||
						(window as any).webkitSpeechRecognition
					: null;
			if (!SpeechRecognitionApi) return;

			const recognition = new SpeechRecognitionApi();
			recognition.continuous = true;
			recognition.interimResults = true;
			recognition.lang = navigator.language || "en-US";

			let finalTranscript = "";

			recognition.onresult = (event: any) => {
				let interim = "";
				for (let i = event.resultIndex; i < event.results.length; i++) {
					const result = event.results[i];
					if (result.isFinal) {
						finalTranscript += result[0].transcript;
					} else {
						interim += result[0].transcript;
					}
				}
				setInput((prev) => {
					const base = prev.endsWith(finalTranscript)
						? prev
						: (prev ? `${prev} ` : "") + finalTranscript;
					return interim ? `${base} ${interim}` : base;
				});
			};

			recognition.onerror = () => {
				setIsTranscribing(false);
			};

			recognition.onend = () => {
				setIsTranscribing(false);
				recognitionRef.current = null;
			};

			recognitionRef.current = recognition;
			recognition.start();
			setIsTranscribing(true);
		};

		const stopTranscription = () => {
			if (recognitionRef.current) {
				recognitionRef.current.stop();
				recognitionRef.current = null;
			}
			setIsTranscribing(false);
		};

		const formatTime = (seconds: number) => {
			const mins = Math.floor(seconds / 60);
			const secs = seconds % 60;
			return `${mins}:${secs.toString().padStart(2, "0")}`;
		};

		// Cleanup on unmount
		useEffect(() => {
			return () => {
				if (recordingIntervalRef.current) {
					clearInterval(recordingIntervalRef.current);
				}
				if (recognitionRef.current) {
					recognitionRef.current.stop();
				}
			};
		}, []);

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

		const isImageFile = (file: File) => {
			return file.type.startsWith("image/");
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
									<span className="text-xs font-medium">Audio Recording</span>
									<span className="text-xs text-muted-foreground">
										{formatTime(recordingTime)} •{" "}
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

						{/* Attached Files */}
						{attachedFiles.length > 0 && (
							<div className="space-y-2">
								{/* File Count Summary */}
								{attachedFiles.length > 6 && (
									<div className="flex items-center justify-between text-xs text-muted-foreground">
										<span>
											{attachedFiles.length} file
											{attachedFiles.length > 1 ? "s" : ""} attached
										</span>
										<Button
											type="button"
											size="sm"
											variant="ghost"
											onClick={() => setAttachedFiles([])}
											className="h-6 px-2 text-xs hover:bg-destructive hover:text-destructive-foreground"
										>
											Clear all
										</Button>
									</div>
								)}

								{/* Files Grid - Max 3 columns, compact layout */}
								<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2 max-h-32 overflow-y-auto">
									{attachedFiles.slice(0, 6).map((file, index) => (
										<div
											key={index}
											className="group relative flex items-center gap-2 p-2 bg-background border border-border rounded-lg hover:border-ring transition-colors min-w-0"
										>
											{isImageFile(file) ? (
												<div className="flex items-center gap-2 min-w-0 flex-1">
													<div className="relative flex-shrink-0">
														<img
															src={sanitizeImageUrl(
																URL.createObjectURL(file),
																"",
															)}
															alt={file.name}
															className="w-8 h-8 object-cover rounded border"
														/>
														<ImageIcon className="w-3 h-3 absolute -top-1 -right-1 bg-background rounded-full p-0.5" />
													</div>
													<div className="flex flex-col min-w-0 flex-1">
														<span className="text-xs font-medium truncate">
															{file.name}
														</span>
														<span className="text-xs text-muted-foreground">
															{humanFileSize(file.size, true)}
														</span>
													</div>
												</div>
											) : (
												<div className="flex items-center gap-2 min-w-0 flex-1">
													<div className="w-8 h-8 bg-muted rounded flex items-center justify-center flex-shrink-0">
														<FileIcon className="w-3 h-3 text-muted-foreground" />
													</div>
													<div className="flex flex-col min-w-0 flex-1">
														<span className="text-xs font-medium truncate">
															{file.name}
														</span>
														<span className="text-xs text-muted-foreground">
															{humanFileSize(file.size, true)}
														</span>
													</div>
												</div>
											)}

											<Button
												type="button"
												size="sm"
												variant="ghost"
												className="h-5 w-5 p-0 rounded-full bg-background border border-border opacity-0 group-hover:opacity-100 transition-opacity hover:bg-destructive hover:text-destructive-foreground flex-shrink-0"
												onClick={() => handleRemoveFile(index)}
											>
												<X className="w-3 h-3" />
											</Button>
										</div>
									))}
								</div>

								{/* Show overflow indicator */}
								{attachedFiles.length > 6 && (
									<button
										className="text-xs text-muted-foreground text-center py-1 bg-muted/30 rounded border border-dashed hover:border-solid hover:border-primary transition-colors w-full mt-2"
										onClick={() => setShowFileManager(true)}
									>
										+{attachedFiles.length - 6} more files
									</button>
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

				<form onSubmit={handleSubmit} className="relative">
					<div
						className="flex flex-col items-start bg-background border border-border rounded-2xl shadow-sm focus-within:ring-2 focus-within:ring-ring focus-within:border-input transition-all duration-200"
						onDrop={handleDrop}
						onDragOver={handleDragOver}
					>
						{/* Text Input */}
						<div className="flex-1 py-2 w-full pr-2">
							<Textarea
								ref={chatboxRef}
								value={input}
								onChange={(e) => setInput(e.target.value)}
								onKeyDown={handleKeyDown}
								onPaste={handlePaste}
								placeholder="Type a message..."
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
						<div className="flex items-center justify-between w-full bg-background rounded-b-2xl">
							{/* Left side buttons */}
							<div className="flex items-center gap-1 p-2 pt-0">
								{/* File Upload Button */}
								{fileUpload && (
									<Popover>
										<PopoverTrigger asChild>
											<Button
												type="button"
												size="sm"
												variant="ghost"
												className="h-8 w-8 p-0 hover:bg-accent rounded-lg transition-colors"
											>
												<Plus className="w-4 h-4 text-muted-foreground" />
											</Button>
										</PopoverTrigger>
										<PopoverContent side="top" className="w-48 p-2 mb-2">
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
											<div className="space-y-1">
												<label
													htmlFor="file-upload"
													className="flex items-center gap-2 p-2 hover:bg-accent rounded cursor-pointer transition-colors"
												>
													<FileIcon className="w-4 h-4" />
													Upload files
												</label>
												<label
													htmlFor="folder-upload"
													className="flex items-center gap-2 p-2 hover:bg-accent rounded cursor-pointer transition-colors"
												>
													<Plus className="w-4 h-4" />
													Upload folder
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
													type="button"
													size="sm"
													variant="ghost"
													className="h-8 w-8 p-0 hover:bg-accent rounded-lg transition-colors relative"
												>
													<WrenchIcon className="w-4 h-4 text-muted-foreground" />
												</Button>
											</PopoverTrigger>
											<PopoverContent side="top" className="w-48 p-2 mb-2">
												<div className="space-y-1">
													<div className="text-xs font-medium text-muted-foreground px-2 pb-1">
														Tools
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
											title="Clear all attachments"
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
								{audioInput &&
									voiceMode === "stt" &&
									typeof window !== "undefined" &&
									((window as any).SpeechRecognition ||
										(window as any).webkitSpeechRecognition) && (
										<Button
											type="button"
											size="sm"
											variant={isTranscribing ? "destructive" : "ghost"}
											className={cn(
												"h-8 w-8 p-0 rounded-full transition-colors",
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
								{audioInput && voiceMode === "record" && (
									<div className="flex items-center gap-1">
										{voiceInvoke === "hold" ? (
											<>
												<Button
													type="button"
													size="sm"
													variant={isRecording ? "destructive" : "ghost"}
													className={cn(
														"h-8 w-8 p-0 rounded-full select-none transition-colors",
														isRecording ? "animate-pulse" : "hover:bg-accent",
													)}
													disabled={
														!!recordedAudio ||
														sendDisabled ||
														typeof navigator?.mediaDevices?.getUserMedia !==
															"function"
													}
													onPointerDown={() => startRecording()}
													onPointerUp={() => stopRecording()}
													onPointerLeave={() => {
														if (isRecording) stopRecording();
													}}
												>
													<MicIcon className="w-4 h-4" />
												</Button>
												{isRecording && (
													<span className="text-xs text-muted-foreground font-mono">
														{formatTime(recordingTime)}
													</span>
												)}
											</>
										) : isRecording ? (
											<>
												<Button
													type="button"
													size="sm"
													variant="destructive"
													className="h-8 w-8 p-0 rounded-full animate-pulse"
													onClick={stopRecording}
												>
													<SquareIcon className="w-3 h-3" />
												</Button>
												<Button
													type="button"
													size="sm"
													variant="ghost"
													className="h-8 w-8 p-0 rounded-full"
													onClick={cancelRecording}
												>
													<X className="w-3 h-3" />
												</Button>
												<span className="text-xs text-muted-foreground font-mono">
													{formatTime(recordingTime)}
												</span>
											</>
										) : (
											<Button
												disabled={
													!!recordedAudio ||
													sendDisabled ||
													typeof navigator?.mediaDevices?.getUserMedia !==
														"function"
												}
												type="button"
												size="sm"
												variant="ghost"
												className="h-8 w-8 p-0 rounded-full hover:bg-accent transition-colors"
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
										className="h-8 w-8 p-0 rounded-full hover:bg-violet-500/10 hover:text-violet-500 transition-colors"
										onClick={onVoiceModeToggle}
										disabled={isRecording || isTranscribing || sendDisabled}
										title="Voice mode"
									>
										<AudioWaveform className="w-4 h-4" />
									</Button>
								)}

								<Button
									type="submit"
									size="sm"
									disabled={sendDisabled || (!input.trim() && !recordedAudio)}
									variant={
										input.trim() || recordedAudio ? "default" : "secondary"
									}
									className="h-9 w-9 sm:h-8 sm:w-8 p-0 rounded-full transition-all duration-200"
								>
									<Send className="w-4 h-4" />
								</Button>
							</div>
						</div>
					</div>
				</form>
			</div>
		);
	},
);
