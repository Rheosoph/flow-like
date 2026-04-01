"use client";

import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
	Badge,
	Button,
	Label,
	ScrollArea,
	Slider,
	Switch,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
} from "@tm9657/flow-like-ui";
import { AnimatePresence, motion } from "framer-motion";
import {
	AlertCircle,
	ChevronDown,
	ChevronUp,
	Circle,
	Clipboard,
	ClipboardPaste,
	Download,
	Fingerprint,
	Image,
	Info,
	Keyboard,
	Minimize2,
	MousePointer2,
	Move,
	Pause,
	Play,
	Scroll,
	Settings2,
	Square,
	Trash2,
	X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

const MAX_LABEL_LENGTH = 15;
const MAX_VISIBLE_ACTIONS = 8;

interface RecordedAction {
	id: string;
	timestamp: string;
	action_type: ActionType;
	coordinates: [number, number] | null;
	screenshot_ref: string | null;
	fingerprint: RecordedFingerprint | null;
	metadata: ActionMetadata;
}

type ActionType =
	| { Click: { button: string; modifiers: string[] } }
	| { DoubleClick: { button: string } }
	| { Drag: { start: [number, number]; end: [number, number] } }
	| { Scroll: { direction: string; amount: number } }
	| { KeyType: { text: string } }
	| { KeyPress: { key: string; modifiers: string[] } }
	| { Copy: { content: string | null } }
	| { Paste: { content: string | null } }
	| { AppLaunch: { app_name: string; app_path: string } }
	| { WindowFocus: { window_title: string; process: string } };

interface RecordedFingerprint {
	id: string;
	role: string | null;
	name: string | null;
	text: string | null;
	bounding_box: [number, number, number, number] | null;
}

interface ActionMetadata {
	window_title: string | null;
	process_name: string | null;
	monitor_index: number | null;
}

interface RecordingSettings {
	capture_screenshots: boolean;
	capture_fingerprints: boolean;
	aggregate_keystrokes: boolean;
	ignore_system_apps: string[];
	capture_region_size: number;
	use_pattern_matching: boolean;
	template_confidence: number;
	bot_detection_evasion: boolean;
	use_fingerprints: boolean;
}

type RecordingStatus = "Idle" | "Recording" | "Paused" | "Processing";

// Platform-specific stop shortcut
const STOP_SHORTCUT =
	typeof navigator !== "undefined" &&
	/Mac|iPod|iPhone|iPad/.test(navigator.platform)
		? "⌘+Shift+S"
		: "Ctrl+Shift+S";

interface RecordingDockProps {
	boardId: string;
	appId?: string;
	token?: string | null;
	version?: [number, number, number];
	onClose: () => void;
	onInsertActions?: (actions: RecordedAction[]) => void;
}

export function RecordingDock({
	boardId,
	appId,
	token,
	version,
	onClose,
	onInsertActions,
}: RecordingDockProps) {
	const [status, setStatus] = useState<RecordingStatus>("Idle");
	const [actions, setActions] = useState<RecordedAction[]>([]);
	const [elapsed, setElapsed] = useState(0);
	const [minimized, setMinimized] = useState(false);
	const [showSettings, setShowSettings] = useState(false);
	const [showActions, setShowActions] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [inserting, setInserting] = useState(false);
	const [isRecordingMode, setIsRecordingMode] = useState(false);
	const [settings, setSettings] = useState<RecordingSettings>({
		capture_screenshots: true,
		capture_fingerprints: true,
		aggregate_keystrokes: true,
		ignore_system_apps: ["SystemUIServer", "loginwindow"],
		capture_region_size: 150,
		use_pattern_matching: false,
		template_confidence: 0.8,
		bot_detection_evasion: false,
		use_fingerprints: true,
	});
	const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

	useEffect(() => {
		let unlisten: UnlistenFn | null = null;

		const setupListener = async () => {
			unlisten = await listen<RecordedAction>("recording:action", (event) => {
				setActions((prev) => [...prev, event.payload]);
			});
		};

		setupListener();

		return () => {
			unlisten?.();
		};
	}, []);

	useEffect(() => {
		if (status === "Recording") {
			timerRef.current = setInterval(() => {
				setElapsed((prev) => prev + 1);
			}, 1000);
		} else {
			if (timerRef.current) {
				clearInterval(timerRef.current);
				timerRef.current = null;
			}
		}

		return () => {
			if (timerRef.current) {
				clearInterval(timerRef.current);
			}
		};
	}, [status]);

	const stopRecording = useCallback(async () => {
		try {
			const recordedActions = await invoke<RecordedAction[]>("stop_recording");
			setStatus("Idle");
			setIsRecordingMode(false);

			// Restore the window
			try {
				const appWindow = getCurrentWindow();
				await appWindow.unminimize();
				await appWindow.setFocus();
			} catch (e) {
				console.warn("Could not restore window:", e);
			}

			if (recordedActions.length > 0) {
				setActions(recordedActions);
			}
		} catch (err) {
			console.error("Failed to stop recording:", err);
			setError(String(err));
			setStatus("Idle");
			setIsRecordingMode(false);

			// Try to restore window even on error
			try {
				const appWindow = getCurrentWindow();
				await appWindow.unminimize();
				await appWindow.setFocus();
			} catch (_) {}
		}
	}, []);

	// Keyboard shortcut listener for stop recording
	useEffect(() => {
		if (!isRecordingMode) return;

		const handleKeyDown = (e: KeyboardEvent) => {
			const isMac = /Mac|iPod|iPhone|iPad/.test(navigator.platform);
			const modifierPressed = isMac ? e.metaKey : e.ctrlKey;

			if (modifierPressed && e.shiftKey && e.key.toLowerCase() === "s") {
				e.preventDefault();
				stopRecording();
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [isRecordingMode, stopRecording]);

	const startRecording = useCallback(async () => {
		try {
			setError(null);
			await invoke<string>("start_recording", {
				appId: appId || null,
				boardId,
				settings,
				token: token || null,
			});
			setStatus("Recording");
			setElapsed(0);
			setActions([]);
			setShowSettings(false);
			setIsRecordingMode(true);

			// Minimize the main window
			try {
				const appWindow = getCurrentWindow();
				await appWindow.minimize();
			} catch (e) {
				console.warn("Could not minimize window:", e);
			}
		} catch (err) {
			console.error("Failed to start recording:", err);
			setError(String(err));
		}
	}, [appId, boardId, settings, token]);

	const pauseRecording = useCallback(async () => {
		try {
			await invoke("pause_recording");
			setStatus("Paused");
		} catch (err) {
			console.error("Failed to pause recording:", err);
			setError(String(err));
		}
	}, []);

	const resumeRecording = useCallback(async () => {
		try {
			await invoke("resume_recording");
			setStatus("Recording");
		} catch (err) {
			console.error("Failed to resume recording:", err);
			setError(String(err));
		}
	}, []);

	const clearRecording = useCallback(() => {
		setActions([]);
		setElapsed(0);
		setError(null);
	}, []);

	const insertActions = useCallback(async () => {
		if (actions.length === 0) return;

		try {
			setInserting(true);
			setError(null);
			await invoke("insert_recording_to_board", {
				boardId,
				actions,
				position: [window.innerWidth / 2 - 200, window.innerHeight / 2 - 100],
				version: version ?? null,
				usePatternMatching: settings.use_pattern_matching || null,
				templateConfidence: settings.template_confidence || null,
				useFingerprints: settings.use_fingerprints || null,
			});
			// Trigger board refresh to show the new nodes
			window.dispatchEvent(new CustomEvent("flow:refetch-board"));
			onInsertActions?.(actions);
			setActions([]);
			onClose();
		} catch (err) {
			console.error("Failed to insert actions:", err);
			setError(`Failed to insert: ${String(err)}`);
		} finally {
			setInserting(false);
		}
	}, [actions, boardId, version, onClose, onInsertActions]);

	const formatTime = (seconds: number) => {
		const mins = Math.floor(seconds / 60);
		const secs = seconds % 60;
		return `${mins.toString().padStart(2, "0")}:${secs.toString().padStart(2, "0")}`;
	};

	const getActionIcon = (action: RecordedAction) => {
		const type = action.action_type;
		if ("Click" in type || "DoubleClick" in type)
			return <MousePointer2 className="h-3 w-3" />;
		if ("Drag" in type) return <Move className="h-3 w-3" />;
		if ("Scroll" in type) return <Scroll className="h-3 w-3" />;
		if ("KeyType" in type || "KeyPress" in type)
			return <Keyboard className="h-3 w-3" />;
		if ("Copy" in type) return <Clipboard className="h-3 w-3" />;
		if ("Paste" in type) return <ClipboardPaste className="h-3 w-3" />;
		return <Circle className="h-3 w-3" />;
	};

	const getActionLabel = (action: RecordedAction): string => {
		const type = action.action_type;
		if ("Click" in type) return `Click (${type.Click.button})`;
		if ("DoubleClick" in type) return "Double Click";
		if ("Drag" in type) return "Drag";
		if ("Scroll" in type)
			return `Scroll ${type.Scroll.direction} (${type.Scroll.amount})`;
		if ("KeyType" in type) {
			const text = type.KeyType.text;
			return text.length > MAX_LABEL_LENGTH
				? `"${text.slice(0, MAX_LABEL_LENGTH)}..."`
				: `"${text}"`;
		}
		if ("KeyPress" in type) return type.KeyPress.key;
		if ("Copy" in type) return "Copy";
		if ("Paste" in type) return "Paste";
		if ("AppLaunch" in type) return type.AppLaunch.app_name;
		if ("WindowFocus" in type)
			return (
				type.WindowFocus.window_title?.slice(0, MAX_LABEL_LENGTH) || "Window"
			);
		return "Action";
	};

	const content = minimized ? (
		<motion.div
			initial={{ scale: 0.8, opacity: 0 }}
			animate={{ scale: 1, opacity: 1 }}
			className="fixed bottom-6 left-1/2 -translate-x-1/2 z-[9999]"
		>
			<Button
				size="lg"
				variant={status === "Recording" ? "destructive" : "secondary"}
				className="gap-3 rounded-full shadow-2xl px-6 h-12"
				onClick={() => setMinimized(false)}
			>
				{status === "Recording" && (
					<span className="relative flex h-3 w-3">
						<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-white opacity-75" />
						<span className="relative inline-flex rounded-full h-3 w-3 bg-white" />
					</span>
				)}
				{status !== "Recording" && <Circle className="h-4 w-4" />}
				<span className="font-mono font-medium">{formatTime(elapsed)}</span>
				{actions.length > 0 && (
					<Badge variant="secondary" className="ml-1">
						{actions.length}
					</Badge>
				)}
			</Button>
		</motion.div>
	) : (
		<motion.div
			initial={{ scale: 0.95, opacity: 0 }}
			animate={{ scale: 1, opacity: 1 }}
			className="fixed inset-0 z-[9999] flex items-center justify-center pointer-events-none p-8"
		>
			<div className="bg-background/95 backdrop-blur-xl border border-border/50 rounded-2xl shadow-2xl overflow-hidden min-w-[380px] max-w-[420px] pointer-events-auto max-h-[calc(100vh-4rem)] flex flex-col">
				{/* Header */}
				<div className="flex items-center justify-between px-4 py-3 border-b border-border/50">
					<div className="flex items-center gap-3">
						{status === "Recording" ? (
							<span className="relative flex h-3 w-3">
								<span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-red-500 opacity-75" />
								<span className="relative inline-flex rounded-full h-3 w-3 bg-red-500" />
							</span>
						) : status === "Paused" ? (
							<span className="h-3 w-3 rounded-full bg-amber-500" />
						) : (
							<span className="h-3 w-3 rounded-full bg-muted-foreground/50" />
						)}
						<div>
							<p className="text-sm font-medium">
								{status === "Idle" ? "RPA Recorder" : status}
							</p>
							<p className="text-xs text-muted-foreground font-mono">
								{formatTime(elapsed)}
							</p>
						</div>
					</div>
					<div className="flex items-center gap-1">
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 rounded-full"
									onClick={() => setMinimized(true)}
								>
									<Minimize2 className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>Minimize</TooltipContent>
						</Tooltip>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 rounded-full"
									onClick={onClose}
								>
									<X className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>Close</TooltipContent>
						</Tooltip>
					</div>
				</div>

				{/* Error Display */}
				<AnimatePresence>
					{error && (
						<motion.div
							initial={{ height: 0, opacity: 0 }}
							animate={{ height: "auto", opacity: 1 }}
							exit={{ height: 0, opacity: 0 }}
							className="bg-destructive/10 border-b border-destructive/20 px-4 py-2"
						>
							<div className="flex items-center gap-2 text-destructive text-xs">
								<AlertCircle className="h-3 w-3 shrink-0" />
								<span className="truncate">{error}</span>
								<Button
									variant="ghost"
									size="icon"
									className="h-5 w-5 ml-auto shrink-0"
									onClick={() => setError(null)}
								>
									<X className="h-3 w-3" />
								</Button>
							</div>
						</motion.div>
					)}
				</AnimatePresence>

				{/* Main Controls */}
				<div className="px-4 py-5">
					<div className="flex flex-col items-center gap-4">
						{status === "Idle" && (
							<>
								<div className="flex items-center gap-3">
									<Button
										size="lg"
										className="gap-2 rounded-full px-8 h-12 bg-red-500 hover:bg-red-600 text-white font-medium shadow-lg"
										onClick={startRecording}
									>
										<Circle className="h-4 w-4 fill-current" />
										Start Recording
									</Button>
									<Tooltip>
										<TooltipTrigger asChild>
											<Button
												variant="outline"
												size="icon"
												className="rounded-full h-12 w-12"
												onClick={() => setShowSettings(!showSettings)}
											>
												<Settings2
													className={cn(
														"h-5 w-5",
														showSettings && "text-primary",
													)}
												/>
											</Button>
										</TooltipTrigger>
										<TooltipContent>Settings</TooltipContent>
									</Tooltip>
								</div>
								<div className="flex items-center gap-2 text-xs text-muted-foreground bg-muted/50 rounded-lg px-3 py-2">
									<Info className="h-3.5 w-3.5" />
									<span>
										Press{" "}
										<kbd className="px-1.5 py-0.5 bg-background rounded border text-[10px] font-mono">
											{STOP_SHORTCUT}
										</kbd>{" "}
										to stop recording
									</span>
								</div>
							</>
						)}

						{status === "Recording" && (
							<div className="flex items-center gap-3">
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="outline"
											size="icon"
											className="rounded-full h-14 w-14"
											onClick={pauseRecording}
										>
											<Pause className="h-6 w-6" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>Pause</TooltipContent>
								</Tooltip>
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="destructive"
											size="icon"
											className="rounded-full h-14 w-14 shadow-lg"
											onClick={stopRecording}
										>
											<Square className="h-6 w-6 fill-current" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>Stop</TooltipContent>
								</Tooltip>
							</div>
						)}

						{status === "Paused" && (
							<div className="flex items-center gap-3">
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="outline"
											size="icon"
											className="rounded-full h-14 w-14"
											onClick={resumeRecording}
										>
											<Play className="h-6 w-6" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>Resume</TooltipContent>
								</Tooltip>
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="destructive"
											size="icon"
											className="rounded-full h-14 w-14 shadow-lg"
											onClick={stopRecording}
										>
											<Square className="h-6 w-6 fill-current" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>Stop</TooltipContent>
								</Tooltip>
							</div>
						)}
					</div>
				</div>

				{/* Settings Panel */}
				<AnimatePresence>
					{showSettings && status === "Idle" && (
						<motion.div
							initial={{ height: 0, opacity: 0 }}
							animate={{ height: "auto", opacity: 1 }}
							exit={{ height: 0, opacity: 0 }}
							className="border-t border-border/50 overflow-hidden"
						>
							<div className="px-4 py-4 space-y-4">
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<Image className="h-4 w-4 text-muted-foreground" />
										<Label
											htmlFor="capture_screenshots"
											className="text-sm cursor-pointer"
										>
											Capture Screenshots
										</Label>
									</div>
									<Switch
										id="capture_screenshots"
										checked={settings.capture_screenshots}
										onCheckedChange={(checked) =>
											setSettings((s) => ({
												...s,
												capture_screenshots: checked,
											}))
										}
									/>
								</div>
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<Keyboard className="h-4 w-4 text-muted-foreground" />
										<Label
											htmlFor="aggregate_keystrokes"
											className="text-sm cursor-pointer"
										>
											Group Keystrokes
										</Label>
									</div>
									<Switch
										id="aggregate_keystrokes"
										checked={settings.aggregate_keystrokes}
										onCheckedChange={(checked) =>
											setSettings((s) => ({
												...s,
												aggregate_keystrokes: checked,
											}))
										}
									/>
								</div>
								<div className="space-y-3">
									<div className="flex items-center justify-between">
										<Label className="text-sm">Screenshot Region</Label>
										<span className="text-sm text-muted-foreground font-mono bg-muted px-2 py-0.5 rounded">
											{settings.capture_region_size}px
										</span>
									</div>
									<Slider
										value={[settings.capture_region_size]}
										onValueChange={([value]) =>
											setSettings((s) => ({
												...s,
												capture_region_size: value,
											}))
										}
										min={50}
										max={300}
										step={25}
										className="w-full"
									/>
								</div>
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<MousePointer2 className="h-4 w-4 text-muted-foreground" />
										<Label
											htmlFor="use_pattern_matching"
											className="text-sm cursor-pointer"
										>
											Pattern Matching for Clicks
										</Label>
									</div>
									<Switch
										id="use_pattern_matching"
										checked={settings.use_pattern_matching}
										onCheckedChange={(checked) =>
											setSettings((s) => ({
												...s,
												use_pattern_matching: checked,
											}))
										}
									/>
								</div>
								{settings.use_pattern_matching && (
									<div className="space-y-3 pl-6">
										<div className="flex items-center justify-between">
											<Label className="text-sm">Match Confidence</Label>
											<span className="text-sm text-muted-foreground font-mono bg-muted px-2 py-0.5 rounded">
												{Math.round(settings.template_confidence * 100)}%
											</span>
										</div>
										<Slider
											value={[settings.template_confidence * 100]}
											onValueChange={([value]) =>
												setSettings((s) => ({
													...s,
													template_confidence: value / 100,
												}))
											}
											min={50}
											max={99}
											step={1}
											className="w-full"
										/>
									</div>
								)}
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<Fingerprint className="h-4 w-4 text-muted-foreground" />
										<Label
											htmlFor="use_fingerprints"
											className="text-sm cursor-pointer"
										>
											Element Fingerprinting
										</Label>
									</div>
									<Switch
										id="use_fingerprints"
										checked={settings.use_fingerprints}
										onCheckedChange={(checked) =>
											setSettings((s) => ({
												...s,
												use_fingerprints: checked,
											}))
										}
									/>
								</div>
							</div>
						</motion.div>
					)}
				</AnimatePresence>

				{/* Actions List */}
				{actions.length > 0 && (
					<div className="border-t border-border/50">
						<button
							type="button"
							onClick={() => setShowActions(!showActions)}
							className="w-full flex items-center justify-between px-4 py-3 hover:bg-muted/50 transition-colors"
						>
							<div className="flex items-center gap-2">
								<Badge variant="secondary" className="font-mono">
									{actions.length}
								</Badge>
								<span className="text-sm text-muted-foreground">
									actions recorded
								</span>
							</div>
							<div className="flex items-center gap-1">
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="ghost"
											size="icon"
											className="h-7 w-7"
											onClick={(e) => {
												e.stopPropagation();
												clearRecording();
											}}
										>
											<Trash2 className="h-3.5 w-3.5" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>Clear All</TooltipContent>
								</Tooltip>
								{showActions ? (
									<ChevronUp className="h-4 w-4 text-muted-foreground" />
								) : (
									<ChevronDown className="h-4 w-4 text-muted-foreground" />
								)}
							</div>
						</button>

						<AnimatePresence>
							{showActions && (
								<motion.div
									initial={{ height: 0 }}
									animate={{ height: "auto" }}
									exit={{ height: 0 }}
									className="overflow-hidden"
								>
									<ScrollArea className="max-h-36 overflow-y-auto">
										<div className="px-4 pb-3 space-y-1.5">
											{actions
												.slice(-MAX_VISIBLE_ACTIONS)
												.map((action, index) => (
													<div
														key={action.id}
														className="flex items-center gap-2 rounded-lg bg-muted/40 px-3 py-2 text-xs"
													>
														<span className="text-muted-foreground w-5 text-right font-mono">
															{actions.length > 8
																? actions.length - 8 + index + 1
																: index + 1}
														</span>
														<span className="text-muted-foreground">
															{getActionIcon(action)}
														</span>
														<span className="truncate flex-1">
															{getActionLabel(action)}
														</span>
														{action.screenshot_ref && (
															<Image className="h-3 w-3 text-green-500" />
														)}
													</div>
												))}
										</div>
									</ScrollArea>
								</motion.div>
							)}
						</AnimatePresence>
					</div>
				)}

				{/* Insert Button */}
				{status === "Idle" && actions.length > 0 && (
					<div className="border-t border-border/50 p-4">
						<Button
							onClick={insertActions}
							disabled={inserting}
							className="w-full gap-2 rounded-full h-12 font-medium shadow-lg"
							size="lg"
						>
							<Download className="h-4 w-4" />
							{inserting
								? "Inserting..."
								: `Insert ${actions.length} Actions to Board`}
						</Button>
					</div>
				)}
			</div>
		</motion.div>
	);

	// Use portal to escape any parent transforms that break fixed positioning
	if (typeof document === "undefined") return content;
	return createPortal(content, document.body);
}
