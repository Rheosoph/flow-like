"use client";

import {
	Badge,
	Button,
	Input,
	Label,
	ScrollArea,
	Slider,
	Switch,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
import { type RpaCapability, ensureRpaSystemPermissions } from "./rpa-consent";

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
	| {
			BrowserAttach: {
				debugger_address: string;
				webdriver_url: string;
				browser_type: string;
			};
	  }
	| {
			Browser: {
				action: { kind: string; selector: string; value: string; url: string };
			};
	  }
	| { Click: { button: string; modifiers: string[] } }
	| { DoubleClick: { button: string; modifiers?: string[] } }
	| {
			Drag: {
				start: [number, number];
				end: [number, number];
				button?: string;
				modifiers?: string[];
			};
	  }
	| { MouseMove: { x: number; y: number } }
	| { Wait: { milliseconds: number } }
	| { Scroll: { direction: string; amount: number } }
	| { KeyType: { text: string } }
	| { KeyPress: { key: string; modifiers: string[] } }
	| { Copy: { clipboard_content: string | null } }
	| { Paste: { clipboard_content: string | null } }
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
	window_id?: string | null;
	window_title: string | null;
	process_name: string | null;
	monitor_index: number | null;
}

interface RecordingSettings {
	browser_debugger_address: string | null;
	browser_webdriver_url: string;
	browser_type: "Chrome" | "Edge";
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
	const { t } = useTranslation("common");
	const [status, setStatus] = useState<RecordingStatus>("Idle");
	const [actions, setActions] = useState<RecordedAction[]>([]);
	const [elapsed, setElapsed] = useState(0);
	const [minimized, setMinimized] = useState(false);
	const [showSettings, setShowSettings] = useState(false);
	const [showActions, setShowActions] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [inserting, setInserting] = useState(false);
	const [starting, setStarting] = useState(true);
	const activeRef = useRef(false);
	const mountedRef = useRef(true);
	const recoveryEpochRef = useRef(0);
	const [settings, setSettings] = useState<RecordingSettings>({
		browser_debugger_address: null,
		browser_webdriver_url: "http://127.0.0.1:9515",
		browser_type: "Chrome",
		capture_screenshots: true,
		capture_fingerprints: false,
		aggregate_keystrokes: true,
		ignore_system_apps: ["SystemUIServer", "loginwindow"],
		capture_region_size: 150,
		use_pattern_matching: false,
		template_confidence: 0.8,
		bot_detection_evasion: false,
		use_fingerprints: false,
	});
	const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

	useEffect(() => {
		mountedRef.current = true;
		let cancelled = false;
		const unlisten: UnlistenFn[] = [];
		const addListener = async <T,>(
			name: string,
			callback: (payload: T) => void,
		) => {
			const stop = await listen<T>(name, (event) => callback(event.payload));
			if (cancelled) stop();
			else unlisten.push(stop);
		};
		const hydrate = async () => {
			const epoch = recoveryEpochRef.current;
			const [currentStatus, recorded] = await Promise.all([
				invoke<RecordingStatus>("get_recording_status", {
					appId: appId || null,
					boardId,
				}),
				invoke<RecordedAction[]>("get_recorded_actions", {
					appId: appId || null,
					boardId,
				}),
			]);
			if (cancelled || epoch !== recoveryEpochRef.current) return;
			activeRef.current =
				currentStatus === "Recording" || currentStatus === "Paused";
			setStatus(currentStatus);
			setActions(recorded);
		};
		void addListener<RecordedAction>("recording:action", () => {
			void hydrate().catch(console.warn);
		});
		void addListener<RecordedAction[]>("recording:stopped", () => {
			if (cancelled) return;
			const wasActive = activeRef.current;
			activeRef.current = false;
			setStatus("Idle");
			void hydrate().catch(console.warn);
			if (wasActive)
				void getCurrentWindow()
					.unminimize()
					.then(() => getCurrentWindow().setFocus())
					.catch(console.warn);
		});
		void addListener<null>("recording:reset", () => {
			if (cancelled) return;
			recoveryEpochRef.current += 1;
			activeRef.current = false;
			setStatus("Idle");
			setActions([]);
			setElapsed(0);
			setError(null);
			void hydrate().catch(console.warn);
		});
		void addListener<string>("recording:error", (message) => {
			if (!cancelled) setError(message);
		});
		void hydrate()
			.catch((error) => {
				if (!cancelled) setError(String(error));
			})
			.finally(() => {
				if (!cancelled) setStarting(false);
			});
		const refresh = setInterval(() => {
			if (activeRef.current) void hydrate().catch(console.warn);
		}, 500);
		return () => {
			cancelled = true;
			recoveryEpochRef.current += 1;
			mountedRef.current = false;
			clearInterval(refresh);
			for (const stop of unlisten) stop();
			if (activeRef.current) {
				activeRef.current = false;
				void invoke("stop_recording").catch(console.error);
			}
		};
	}, [appId, boardId]);

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
		const epoch = recoveryEpochRef.current;
		try {
			setStatus("Processing");
			const recordedActions = await invoke<RecordedAction[]>("stop_recording");
			if (epoch !== recoveryEpochRef.current) return true;
			activeRef.current = false;
			setStatus("Idle");

			// Restore the window
			try {
				const appWindow = getCurrentWindow();
				await appWindow.unminimize();
				await appWindow.setFocus();
			} catch (e) {
				console.warn("Could not restore window:", e);
			}

			if (epoch === recoveryEpochRef.current) setActions(recordedActions);
			return true;
		} catch (err) {
			if (epoch !== recoveryEpochRef.current) return true;
			console.error("Failed to stop recording:", err);
			setError(String(err));
			try {
				const currentStatus = await invoke<RecordingStatus>(
					"get_recording_status",
					{ appId: appId || null, boardId },
				);
				activeRef.current =
					currentStatus === "Recording" || currentStatus === "Paused";
				setStatus(currentStatus);
			} catch (_) {
				setStatus("Recording");
			}

			// Try to restore window even on error
			try {
				const appWindow = getCurrentWindow();
				await appWindow.unminimize();
				await appWindow.setFocus();
			} catch (_) {}
			return false;
		}
	}, [appId, boardId]);

	const closeRecording = useCallback(async () => {
		if (activeRef.current && !(await stopRecording())) return;
		onClose();
	}, [onClose, stopRecording]);

	const startRecording = useCallback(async () => {
		if (starting || activeRef.current || actions.length > 0) return;
		const epoch = recoveryEpochRef.current;
		setStarting(true);
		try {
			setError(null);
			const browserRecording = Boolean(
				settings.browser_debugger_address?.trim(),
			);
			const required: RpaCapability[] = browserRecording
				? ["browser"]
				: ["input_monitoring", "window_management"];
			if (settings.capture_screenshots && !browserRecording)
				required.push("screen_capture");
			if (settings.use_fingerprints && !browserRecording)
				required.push("accessibility");
			if (
				!(await ensureRpaSystemPermissions({ appId, boardId, required })) ||
				!mountedRef.current ||
				epoch !== recoveryEpochRef.current
			)
				return;
			await invoke<string>("start_recording", {
				appId: appId || null,
				boardId,
				settings: {
					...settings,
					browser_debugger_address:
						settings.browser_debugger_address?.trim() || null,
					capture_screenshots:
						settings.capture_screenshots && !browserRecording,
					capture_fingerprints: settings.use_fingerprints && !browserRecording,
				},
				token: token || null,
			});
			if (!mountedRef.current) {
				await invoke("stop_recording");
				return;
			}
			if (epoch !== recoveryEpochRef.current) return;
			activeRef.current = true;
			const currentStatus = await invoke<RecordingStatus>(
				"get_recording_status",
				{ appId: appId || null, boardId },
			);
			if (!mountedRef.current) {
				await invoke("stop_recording");
				return;
			}
			if (epoch !== recoveryEpochRef.current) return;
			setStatus(currentStatus);
			if (currentStatus !== "Recording") {
				activeRef.current = currentStatus === "Paused";
				const recorded = await invoke<RecordedAction[]>(
					"get_recorded_actions",
					{
						appId: appId || null,
						boardId,
					},
				);
				if (epoch === recoveryEpochRef.current) setActions(recorded);
				return;
			}
			setElapsed(0);
			setActions([]);
			setShowSettings(false);

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
		} finally {
			setStarting(false);
		}
	}, [appId, boardId, settings, token, starting, actions.length]);

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

	const clearRecording = useCallback(async () => {
		try {
			await invoke("clear_recorded_actions", { appId: appId || null, boardId });
			setActions([]);
			setElapsed(0);
			setError(null);
		} catch (error) {
			setError(String(error));
		}
	}, [appId, boardId]);

	const insertActions = useCallback(async () => {
		if (actions.length === 0) return;

		try {
			setInserting(true);
			setError(null);
			await invoke("insert_recording_to_board", {
				boardId,
				appId: appId || null,
				actions,
				position: [window.innerWidth / 2 - 200, window.innerHeight / 2 - 100],
				version: version ?? null,
				usePatternMatching: settings.use_pattern_matching,
				templateConfidence: settings.template_confidence,
				useFingerprints: settings.use_fingerprints,
				botDetectionEvasion: settings.bot_detection_evasion,
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
	}, [actions, appId, boardId, version, onClose, onInsertActions, settings]);

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
		if ("Click" in type)
			return t("clickButton", "Click ({{button}})", {
				button: type.Click.button,
			});
		if ("DoubleClick" in type) return "Double Click";
		if ("Drag" in type) return "Drag";
		if ("Scroll" in type)
			return t("scrollDirectionAmount", "Scroll {{direction}} ({{amount}})", {
				direction: type.Scroll.direction,
				amount: type.Scroll.amount,
			});
		if ("KeyType" in type) {
			const text = type.KeyType.text;
			return text.length > MAX_LABEL_LENGTH
				? `"${text.slice(0, MAX_LABEL_LENGTH)}..."`
				: `"${text}"`;
		}
		if ("KeyPress" in type) return type.KeyPress.key;
		if ("Browser" in type) return `Browser: ${type.Browser.action.kind}`;
		if ("BrowserAttach" in type) return "Attach browser";
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
							<TooltipContent>{t("minimize", "Minimize")}</TooltipContent>
						</Tooltip>
						<Tooltip>
							<TooltipTrigger asChild>
								<Button
									variant="ghost"
									size="icon"
									className="h-8 w-8 rounded-full"
									onClick={closeRecording}
								>
									<X className="h-4 w-4" />
								</Button>
							</TooltipTrigger>
							<TooltipContent>{t("close", "Close")}</TooltipContent>
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
										disabled={starting || actions.length > 0}
									>
										<Circle className="h-4 w-4 fill-current" />
										{t("startRecording", "Start Recording")}
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
										<TooltipContent>{t("settings", "Settings")}</TooltipContent>
									</Tooltip>
								</div>
								<div className="flex items-center gap-2 text-xs text-muted-foreground bg-muted/50 rounded-lg px-3 py-2">
									<Info className="h-3.5 w-3.5" />
									<span>
										{t("press", "Press")}{" "}
										<kbd className="px-1.5 py-0.5 bg-background rounded border text-[10px] font-mono">
											{STOP_SHORTCUT}
										</kbd>{" "}
										{t("toStopRecording", "to stop recording")}
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
									<TooltipContent>{t("pause", "Pause")}</TooltipContent>
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
									<TooltipContent>{t("stop", "Stop")}</TooltipContent>
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
									<TooltipContent>{t("resume", "Resume")}</TooltipContent>
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
									<TooltipContent>{t("stop", "Stop")}</TooltipContent>
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
								<div className="space-y-2">
									<Label htmlFor="browser-debugger-address">
										Browser recording
									</Label>
									<Input
										id="browser-debugger-address"
										placeholder="Debugger address (optional), e.g. 127.0.0.1:9222"
										value={settings.browser_debugger_address ?? ""}
										onChange={(event) =>
											setSettings((previous) => ({
												...previous,
												browser_debugger_address: event.target.value || null,
											}))
										}
									/>
									{settings.browser_debugger_address && (
										<>
											<Label htmlFor="recording-browser-type">Browser</Label>
											<select
												id="recording-browser-type"
												value={settings.browser_type}
												className="h-9 w-full rounded-md border bg-background px-3 text-sm"
												onChange={(event) =>
													setSettings((previous) => ({
														...previous,
														browser_type: event.target.value as
															| "Chrome"
															| "Edge",
													}))
												}
											>
												<option value="Chrome">Chrome</option>
												<option value="Edge">Edge</option>
											</select>
											<Label htmlFor="browser-webdriver-url">
												Browser driver URL for replay
											</Label>
											<Input
												id="browser-webdriver-url"
												value={settings.browser_webdriver_url}
												onChange={(event) =>
													setSettings((previous) => ({
														...previous,
														browser_webdriver_url: event.target.value,
													}))
												}
											/>
											<p className="text-xs text-muted-foreground">
												Records elements in a debugging-enabled Chrome or Edge
												browser. Use its matching WebDriver and keep the
												recorded tabs open. Replay returns each tab to its first
												recorded URL. Leave the address empty to record the
												desktop.
											</p>
										</>
									)}
								</div>
								<div className="flex items-center justify-between">
									<div className="flex items-center gap-2">
										<Image className="h-4 w-4 text-muted-foreground" />
										<Label
											htmlFor="capture_screenshots"
											className="text-sm cursor-pointer"
										>
											{t("captureScreenshots", "Capture Screenshots")}
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
											{t("groupKeystrokes", "Group Keystrokes")}
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
										<Label className="text-sm">
											{t("screenshotRegion", "Screenshot Region")}
										</Label>
										<span className="text-sm text-muted-foreground font-mono bg-muted px-2 py-0.5 rounded">{`${settings.capture_region_size}px`}</span>
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
											{t(
												"patternMatchingForClicks",
												"Pattern Matching for Clicks",
											)}
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
								<p className="text-xs text-muted-foreground">
									Pattern matching and element fingerprinting stop replay when
									the target is missing or ambiguous. Disable both to use
									recorded coordinates. Pattern matching takes priority when
									both are enabled.
								</p>
								{settings.use_pattern_matching && (
									<div className="space-y-3 pl-6">
										<div className="flex items-center justify-between">
											<Label className="text-sm">
												{t("matchConfidence", "Match Confidence")}
											</Label>
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
											{t("elementFingerprinting", "Element Fingerprinting")}
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
									{t("actionsRecorded", "actions recorded")}
								</span>
							</div>
							<div className="flex items-center gap-1">
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="ghost"
											size="icon"
											className="h-7 w-7"
											disabled={status !== "Idle"}
											onClick={(e) => {
												e.stopPropagation();
												void clearRecording();
											}}
										>
											<Trash2 className="h-3.5 w-3.5" />
										</Button>
									</TooltipTrigger>
									<TooltipContent>{t("clearAll", "Clear All")}</TooltipContent>
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
						<p className="mb-3 text-xs text-muted-foreground">
							Insert or clear this recording before starting another on this
							board. Recordings stay available across boards while Desktop is
							open. Insert them before quitting.
						</p>
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
