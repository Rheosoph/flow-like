"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	CheckCircle2,
	ChevronDown,
	ChevronDownIcon,
	ChevronRight,
	Circle,
	Download,
	FileText,
	HomeIcon,
	ListTodo,
	Loader2,
	Music,
	Paperclip,
	Play,
	RotateCcw,
	Sparkles,
	XCircle,
} from "lucide-react";
import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import type {
	IContent,
	IEvent,
	IEventInput,
	IIntercomEvent,
	IRunPayload,
} from "../../lib";
import { formatDuration } from "../../lib/date";
import { useBackend } from "../../state/backend-state";
import type { IRouteMapping } from "../../state/backend-state/route-state";
import { useExecutionEngine } from "../../state/execution-engine-context";
import {
	Button,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	Input,
	Label,
	ScrollArea,
	Switch,
	TextEditor,
	Textarea,
} from "../ui";
import type { IUseInterfaceProps } from "./interfaces";

type ExecutionStepStatus = "planned" | "progress" | "done" | "failed";

// Attachment type matching the chat interface
type IStreamAttachment =
	| string
	| {
			url: string;
			preview_text?: string;
			thumbnail_url?: string;
			name?: string;
			size?: number;
			type?: string;
			anchor?: string;
			page?: number;
	  };

function contentPartToStreamAttachment(
	part: IContent,
): IStreamAttachment | undefined {
	if (part.image_url?.url) {
		return {
			url: part.image_url.url,
			type: part.image_url.media_type ?? "image/*",
		};
	}
	if (part.audio_url) {
		return { url: part.audio_url, type: part.media_type ?? "audio/*" };
	}
	if (part.video_url) {
		return { url: part.video_url, type: part.media_type ?? "video/*" };
	}
	if (part.document_url) {
		return { url: part.document_url, type: part.media_type };
	}
	return undefined;
}

function mergeStreamAttachments(
	existing: IStreamAttachment[],
	incoming: IStreamAttachment[],
): IStreamAttachment[] {
	const byUrl = new Map<string, IStreamAttachment>();
	for (const attachment of [...existing, ...incoming]) {
		const url = typeof attachment === "string" ? attachment : attachment.url;
		const previous = byUrl.get(url);
		if (!previous || typeof attachment !== "string") {
			byUrl.set(url, attachment);
		}
	}
	return [...byUrl.values()];
}

interface IExecutionStep {
	id: string;
	title: string;
	description?: string;
	status: ExecutionStepStatus;
	startTime?: number;
	endTime?: number;
	reasoning?: string;
}

function normalizeRoute(value: string): string {
	const trimmed = value.trim();
	if (!trimmed) return "/";
	return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

function isAttachmentInput(input: IEventInput): boolean {
	return input.data_type === "PathBuf" || input.data_type === "Byte";
}

function isMultiValueInput(input: IEventInput): boolean {
	return input.value_type === "Array" || input.value_type === "HashSet";
}

function buildUseNavigationUrl(
	appId: string,
	route: string,
	queryParams?: Record<string, string>,
): string {
	if (!route) {
		return `/use?id=${appId}&route=/`;
	}

	if (!route.startsWith("/use") && !route.startsWith("http")) {
		const [routePath, routeQueryString] = route.split("?");
		const params = new URLSearchParams();
		params.set("id", appId);
		params.set("route", normalizeRoute(routePath || "/"));
		params.delete("eventId");

		if (routeQueryString) {
			const routeParams = new URLSearchParams(routeQueryString);
			routeParams.forEach((value, key) => {
				params.set(key, value);
			});
		}

		if (queryParams) {
			for (const [key, value] of Object.entries(queryParams)) {
				params.set(key, value);
			}
		}

		return `/use?${params.toString()}`;
	}

	if (queryParams && Object.keys(queryParams).length > 0) {
		const params = new URLSearchParams(queryParams);
		const separator = route.includes("?") ? "&" : "?";
		return `${route}${separator}${params.toString()}`;
	}

	return route;
}

// ============================================================================
// Execution Steps Component (Plan Steps for Generic Form)
// ============================================================================

function getStepStatusIcon(status: ExecutionStepStatus) {
	switch (status) {
		case "planned":
			return <Circle className="w-4 h-4 text-muted-foreground" />;
		case "progress":
			return <Loader2 className="w-4 h-4 text-primary animate-spin" />;
		case "done":
			return <CheckCircle2 className="w-4 h-4 text-green-500" />;
		case "failed":
			return <XCircle className="w-4 h-4 text-red-500" />;
	}
}

function getStepStatusColor(status: ExecutionStepStatus, isActive: boolean) {
	if (isActive) return "border-primary/70 bg-primary/20 shadow-sm";
	switch (status) {
		case "planned":
			return "border-muted-foreground/30 bg-muted/20";
		case "progress":
			return "border-primary/50 bg-primary/10";
		case "done":
			return "border-green-500/50 bg-green-500/10";
		case "failed":
			return "border-red-500/50 bg-red-500/10";
	}
}

function ExecutionSteps({
	steps,
	currentStepId,
	isComplete,
}: { steps: IExecutionStep[]; currentStepId?: string; isComplete?: boolean }) {
	const [isExpanded, setIsExpanded] = useState(true);
	const [expandedSteps, setExpandedSteps] = useState<Set<string>>(
		new Set(currentStepId ? [currentStepId] : []),
	);

	// Collapse the entire section when complete
	useEffect(() => {
		if (isComplete) {
			setIsExpanded(false);
		}
	}, [isComplete]);

	useEffect(() => {
		if (currentStepId) {
			setExpandedSteps((prev) => {
				if (!prev.has(currentStepId)) {
					const next = new Set(prev);
					next.add(currentStepId);
					return next;
				}
				return prev;
			});
		}
	}, [currentStepId]);

	if (!steps || steps.length === 0) return null;

	const toggleStep = (stepId: string) => {
		setExpandedSteps((prev) => {
			const next = new Set(prev);
			if (next.has(stepId)) next.delete(stepId);
			else next.add(stepId);
			return next;
		});
	};

	const completedCount = steps.filter((s) => s.status === "done").length;
	const allDone = completedCount === steps.length;

	return (
		<div className="rounded-lg border bg-muted/30">
			<button
				type="button"
				onClick={() => setIsExpanded(!isExpanded)}
				className="w-full flex items-center justify-between p-4 hover:bg-muted/50 transition-colors"
			>
				<span className="text-sm font-medium flex items-center gap-2">
					{allDone ? (
						<CheckCircle2 className="w-4 h-4 text-green-500" />
					) : (
						<ListTodo className="w-4 h-4 text-primary" />
					)}
					Progress
				</span>
				<div className="flex items-center gap-2">
					<span className="text-xs text-muted-foreground">
						{completedCount} / {steps.length} completed
					</span>
					{isExpanded ? (
						<ChevronDown className="w-4 h-4 text-muted-foreground" />
					) : (
						<ChevronRight className="w-4 h-4 text-muted-foreground" />
					)}
				</div>
			</button>
			{isExpanded && (
				<div className="px-4 pb-4 pt-4 space-y-2">
					{steps.map((step, index) => {
						const isStepExpanded = expandedSteps.has(step.id);
						const isActive = currentStepId === step.id;
						const hasReasoning = step.reasoning && step.reasoning.trim() !== "";
						const duration =
							step.startTime && step.endTime
								? formatDuration((step.endTime - step.startTime) * 1000)
								: null;

						return (
							<div
								key={step.id}
								className={`rounded-md border transition-all ${getStepStatusColor(step.status, isActive)}`}
							>
								<button
									type="button"
									onClick={() => hasReasoning && toggleStep(step.id)}
									className={`w-full flex items-center gap-3 p-3 text-left ${
										hasReasoning
											? "hover:bg-muted/30 cursor-pointer"
											: "cursor-default"
									}`}
								>
									<div className="shrink-0">
										{getStepStatusIcon(step.status)}
									</div>
									<div className="flex-1 min-w-0">
										<div className="flex items-center gap-2">
											<span className="text-sm font-medium">{step.title}</span>
											{isActive && (
												<span className="text-[10px] px-1.5 py-0.5 rounded bg-primary/20 text-primary font-medium">
													Active
												</span>
											)}
										</div>
										{step.description && (
											<p className="text-xs text-muted-foreground mt-0.5">
												{step.description}
											</p>
										)}
									</div>
									{duration && (
										<span className="text-xs text-muted-foreground">
											{duration}
										</span>
									)}
									{hasReasoning && (
										<div className="shrink-0">
											{isStepExpanded ? (
												<ChevronDown className="w-4 h-4 text-muted-foreground" />
											) : (
												<ChevronRight className="w-4 h-4 text-muted-foreground" />
											)}
										</div>
									)}
								</button>
								{isStepExpanded && hasReasoning && (
									<div className="px-3 pb-3">
										<div className="rounded bg-muted/50 p-2 text-xs text-muted-foreground whitespace-pre-wrap font-mono">
											{step.reasoning}
										</div>
									</div>
								)}
							</div>
						);
					})}
				</div>
			)}
		</div>
	);
}

// ============================================================================
// Streaming Output Display Component
// ============================================================================

interface StreamingOutputProps {
	content: string;
	isStreaming: boolean;
}

function StreamingOutput({ content, isStreaming }: StreamingOutputProps) {
	if (!content && !isStreaming) return null;

	return (
		<div className="rounded-lg border bg-muted/30 p-4">
			<div className="flex items-center justify-between mb-3">
				<span className="text-sm font-medium flex items-center gap-2">
					<Sparkles className="h-4 w-4 text-primary" />
					Response
				</span>
				{isStreaming && (
					<span className="text-xs text-muted-foreground flex items-center gap-2">
						<Loader2 className="h-3 w-3 animate-spin" />
						Generating...
					</span>
				)}
			</div>
			<div className="prose prose-sm dark:prose-invert max-w-none">
				{content ? (
					<TextEditor
						key={content.slice(0, 50)}
						initialContent={content}
						isMarkdown={true}
						editable={false}
					/>
				) : isStreaming ? (
					<div className="flex items-center gap-2 text-muted-foreground">
						<span>Thinking...</span>
					</div>
				) : null}
			</div>
		</div>
	);
}

// ============================================================================
// Attachments Display Component
// ============================================================================

function getAttachmentType(
	attachment: IStreamAttachment,
): "image" | "video" | "audio" | "pdf" | "document" | "other" {
	const url = typeof attachment === "string" ? attachment : attachment.url;
	const mimeType = typeof attachment === "object" ? attachment.type : undefined;

	if (mimeType) {
		if (mimeType.startsWith("image/")) return "image";
		if (mimeType.startsWith("video/")) return "video";
		if (mimeType.startsWith("audio/")) return "audio";
		if (mimeType === "application/pdf") return "pdf";
		if (mimeType.includes("document") || mimeType.includes("text"))
			return "document";
	}

	// Check by URL extension
	const cleanPath = url.split("?")[0].toLowerCase();
	if (cleanPath.match(/\.(jpg|jpeg|png|gif|webp|svg|bmp|tiff)$/))
		return "image";
	if (cleanPath.match(/\.(mp4|webm|mov|avi|mkv|m4v|3gp|ogv)$/)) return "video";
	if (cleanPath.match(/\.(mp3|wav|ogg|m4a|flac|aac|wma)$/)) return "audio";
	if (cleanPath.match(/\.pdf$/)) return "pdf";
	if (cleanPath.match(/\.(doc|docx|txt|md|rtf|xls|xlsx|ppt|pptx)$/))
		return "document";

	// Check data URLs
	if (url.startsWith("data:")) {
		const mimeMatch = url.match(/^data:([^;]+)/);
		const dataMime = mimeMatch?.[1] ?? "";
		if (dataMime.startsWith("image/")) return "image";
		if (dataMime.startsWith("video/")) return "video";
		if (dataMime.startsWith("audio/")) return "audio";
		if (dataMime === "application/pdf") return "pdf";
	}

	return "other";
}

function getAttachmentName(attachment: IStreamAttachment): string {
	if (typeof attachment === "object" && attachment.name) {
		return attachment.name;
	}
	const url = typeof attachment === "string" ? attachment : attachment.url;
	try {
		const urlObj = new URL(url);
		const pathParts = urlObj.pathname.split("/");
		return pathParts[pathParts.length - 1] || "File";
	} catch {
		return "File";
	}
}

function AttachmentsDisplay({
	attachments,
}: { attachments: IStreamAttachment[] }) {
	if (!attachments || attachments.length === 0) return null;

	const handleAttachmentClick = (attachment: IStreamAttachment) => {
		const url = typeof attachment === "string" ? attachment : attachment.url;
		if (url.startsWith("data:")) {
			const type = getAttachmentType(attachment);
			if (type === "image") {
				const newWindow = window.open();
				if (newWindow) {
					newWindow.document.write(
						`<img src="${url}" style="max-width: 100%; height: auto;" />`,
					);
				}
			} else {
				const link = document.createElement("a");
				link.href = url;
				link.download = getAttachmentName(attachment);
				document.body.appendChild(link);
				link.click();
				document.body.removeChild(link);
			}
		} else {
			window.open(url, "_blank", "noopener,noreferrer");
		}
	};

	const images = attachments.filter((a) => getAttachmentType(a) === "image");
	const videos = attachments.filter((a) => getAttachmentType(a) === "video");
	const audio = attachments.filter((a) => getAttachmentType(a) === "audio");
	const others = attachments.filter(
		(a) => !["image", "video", "audio"].includes(getAttachmentType(a)),
	);

	return (
		<div className="rounded-lg border bg-muted/30 p-4 animate-in fade-in duration-200">
			<div className="flex items-center gap-2 mb-3 text-sm font-medium">
				<Paperclip className="h-4 w-4 text-primary" />
				Attachments
				<span className="text-xs text-muted-foreground">
					({attachments.length})
				</span>
			</div>
			<div className="space-y-4">
				{/* Images Grid */}
				{images.length > 0 && (
					<div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
						{images.map((attachment, idx) => {
							const url =
								typeof attachment === "string" ? attachment : attachment.url;
							const thumbnailUrl =
								typeof attachment === "object"
									? attachment.thumbnail_url
									: undefined;
							const name = getAttachmentName(attachment);
							return (
								<div
									key={`img-${idx}`}
									className="relative group rounded-md overflow-hidden border bg-muted/50 cursor-pointer"
									onClick={() => handleAttachmentClick(attachment)}
								>
									<img
										src={thumbnailUrl || url}
										alt={name}
										className="w-full h-32 object-cover hover:opacity-90 transition-opacity"
										onError={(e) => {
											const target = e.target as HTMLImageElement;
											if (thumbnailUrl && target.src === thumbnailUrl) {
												target.src = url;
											}
										}}
									/>
									<div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/70 to-transparent text-white text-xs p-1 truncate">
										{decodeURIComponent(name)}
									</div>
								</div>
							);
						})}
					</div>
				)}

				{/* Videos */}
				{videos.map((attachment, idx) => {
					const url =
						typeof attachment === "string" ? attachment : attachment.url;
					const thumbnailUrl =
						typeof attachment === "object"
							? attachment.thumbnail_url
							: undefined;
					return (
						<div
							key={`vid-${idx}`}
							className="rounded-md overflow-hidden border bg-muted/50 max-w-md"
						>
							<video
								controls
								className="w-full"
								preload="metadata"
								poster={thumbnailUrl}
							>
								<source src={url} />
							</video>
						</div>
					);
				})}

				{/* Audio */}
				{audio.map((attachment, idx) => {
					const url =
						typeof attachment === "string" ? attachment : attachment.url;
					const name = getAttachmentName(attachment);
					return (
						<div
							key={`aud-${idx}`}
							className="rounded-lg border bg-muted/50 p-3"
						>
							<div className="flex items-center gap-2 mb-2">
								<Music className="h-4 w-4 text-muted-foreground" />
								<span className="text-sm truncate">{name}</span>
							</div>
							<audio controls className="w-full h-10">
								<source src={url} />
							</audio>
						</div>
					);
				})}

				{/* Other files */}
				{others.length > 0 && (
					<div className="space-y-2">
						{others.map((attachment, idx) => {
							const url =
								typeof attachment === "string" ? attachment : attachment.url;
							const name = getAttachmentName(attachment);
							const previewText =
								typeof attachment === "object"
									? attachment.preview_text
									: undefined;
							const size =
								typeof attachment === "object" ? attachment.size : undefined;
							return (
								<button
									key={`file-${idx}`}
									type="button"
									onClick={() => handleAttachmentClick(attachment)}
									className="w-full flex items-start gap-3 p-3 rounded-md border bg-background hover:bg-muted/50 transition-colors text-left"
								>
									<FileText className="h-5 w-5 text-muted-foreground shrink-0 mt-0.5" />
									<div className="flex-1 min-w-0">
										<p className="text-sm font-medium truncate">
											{decodeURIComponent(name)}
										</p>
										{previewText && (
											<p className="text-xs text-muted-foreground line-clamp-2 mt-1">
												{previewText}
											</p>
										)}
										{size && (
											<p className="text-xs text-muted-foreground mt-1">
												{size > 1024 * 1024
													? `${(size / (1024 * 1024)).toFixed(1)} MB`
													: size > 1024
														? `${(size / 1024).toFixed(1)} KB`
														: `${size} bytes`}
											</p>
										)}
									</div>
									<Download className="h-4 w-4 text-muted-foreground shrink-0" />
								</button>
							);
						})}
					</div>
				)}
			</div>
		</div>
	);
}

// ============================================================================
// Collapsible Result Component (collapsed by default)
// ============================================================================

function CollapsibleResult({ outputData }: { outputData: unknown }) {
	const [isExpanded, setIsExpanded] = useState(false);

	return (
		<div className="rounded-lg border bg-muted/30 animate-in fade-in slide-in-from-bottom-2 duration-300">
			<button
				type="button"
				onClick={() => setIsExpanded(!isExpanded)}
				className="w-full flex items-center justify-between p-4 hover:bg-muted/50 transition-colors"
			>
				<div className="flex items-center gap-2 text-sm font-medium">
					<CheckCircle2 className="h-4 w-4 text-green-500" />
					Result
				</div>
				{isExpanded ? (
					<ChevronDown className="h-4 w-4 text-muted-foreground" />
				) : (
					<ChevronRight className="h-4 w-4 text-muted-foreground" />
				)}
			</button>
			{isExpanded && (
				<div className="px-4 pb-4 overflow-auto">
					{typeof outputData === "string" ? (
						<div className="prose prose-sm dark:prose-invert max-w-none">
							<TextEditor
								key={outputData.slice(0, 50)}
								initialContent={outputData}
								isMarkdown={true}
								editable={false}
							/>
						</div>
					) : (
						<pre className="text-sm whitespace-pre-wrap break-words font-mono">
							{JSON.stringify(outputData, null, 2)}
						</pre>
					)}
				</div>
			)}
		</div>
	);
}

export function GenericEventFormInterface({
	appId,
	event,
	config,
	toolbarRef,
}: Readonly<IUseInterfaceProps>) {
	const backend = useBackend();
	const executionEngine = useExecutionEngine();
	const router = useRouter();
	const pathname = usePathname();

	const [runEvents, setRunEvents] = useState<IIntercomEvent[]>([]);
	const [isRunning, setIsRunning] = useState(false);
	const [isComplete, setIsComplete] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

	// New state for slick UI
	const [executionSteps, setExecutionSteps] = useState<IExecutionStep[]>([]);
	const [currentStepId, setCurrentStepId] = useState<string | undefined>();
	const [streamingContent, setStreamingContent] = useState<string>("");
	const [streamAttachments, setStreamAttachments] = useState<
		IStreamAttachment[]
	>([]);

	const lastNavigateToRef = useRef<string | null>(null);
	const activeSubscriptionRef = useRef<{
		streamId: string;
		subscriberId: string;
	} | null>(null);

	const routesQuery = useInvoke<IRouteMapping[], [string]>(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		!!appId,
		[appId],
	);

	const eventsQuery = useInvoke<IEvent[], [string]>(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		!!appId,
		[appId],
	);

	const configuredRoutes = useMemo(() => {
		const rawArray = (config as any)?.navigate_to_routes;
		const raw: string[] = Array.isArray(rawArray) ? rawArray : [];
		const normalized = raw
			.map((r) => normalizeRoute(String(r)))
			.filter((r) => !!r);
		return Array.from(new Set(normalized));
	}, [config]);

	const routeEventNames = useMemo(() => {
		const mapping: Record<string, string> = {};
		if (!routesQuery.data || !eventsQuery.data) return mapping;

		for (const route of routesQuery.data) {
			const evt = eventsQuery.data.find((e) => e.id === route.eventId);
			if (evt?.name) {
				mapping[route.path] = evt.name;
			}
		}
		return mapping;
	}, [routesQuery.data, eventsQuery.data]);

	// Use inputs from the event directly (populated by backend)
	// Filter out "payload" field as it's handled separately
	const inputPins = useMemo(() => {
		return (event.inputs ?? []).filter(
			(pin) => pin.name.toLowerCase() !== "payload",
		);
	}, [event.inputs]);

	const [values, setValues] = useState<Record<string, unknown>>({});
	const [files, setFiles] = useState<Record<string, File[]>>({});

	useEffect(() => {
		setValues({});
		setFiles({});
		setFieldErrors({});
		setError(null);
		setRunEvents([]);
		setIsComplete(false);
		setExecutionSteps([]);
		setCurrentStepId(undefined);
		setStreamingContent("");
		setStreamAttachments([]);
	}, [event.id]);

	const handleNavigationEvents = useCallback(
		(events: IIntercomEvent[]) => {
			for (const ev of events) {
				if (ev?.event_type !== "a2ui") continue;
				const message = ev?.payload;
				if (!message || message.type !== "navigateTo") continue;

				const { route, replace, queryParams } = message as {
					route: string;
					replace: boolean;
					queryParams?: Record<string, string>;
				};

				const key = `${route}::${replace ? "r" : "p"}::${JSON.stringify(queryParams ?? {})}`;
				if (lastNavigateToRef.current === key) continue;
				lastNavigateToRef.current = key;

				const navUrl = buildUseNavigationUrl(appId, route, queryParams);
				if (replace) router.replace(navUrl);
				else router.push(navUrl);
			}
		},
		[appId, router],
	);

	useEffect(() => {
		return () => {
			const active = activeSubscriptionRef.current;
			if (!active) return;
			executionEngine.unsubscribeFromEventStream(
				active.streamId,
				active.subscriberId,
			);
			activeSubscriptionRef.current = null;
		};
	}, [executionEngine]);

	const resetOutput = useCallback(() => {
		setRunEvents([]);
		setError(null);
		setIsComplete(false);
		setExecutionSteps([]);
		setCurrentStepId(undefined);
		setStreamingContent("");
		setStreamAttachments([]);
	}, []);

	const handleNavigateTo = useCallback(
		(route: string, replace = false) => {
			const navUrl = buildUseNavigationUrl(appId, route);
			if (replace) router.replace(navUrl);
			else router.push(navUrl);
		},
		[appId, router],
	);

	const getRouteLabel = useCallback(
		(path: string): string => {
			const eventName = routeEventNames[path];
			if (eventName) return eventName;
			if (path === "/") return "Home";
			return path.replace(/^\//, "").replace(/-/g, " ").replace(/\//g, " / ");
		},
		[routeEventNames],
	);

	const toolbarElements = useMemo(() => {
		if (configuredRoutes.length === 0) return [];

		const getRouteIcon = (path: string) => {
			if (path === "/") return <HomeIcon className="h-4 w-4" />;
			return null;
		};

		if (configuredRoutes.length === 1) {
			const route = configuredRoutes[0];
			const icon = getRouteIcon(route);
			return [
				<Button
					key={`navigate-${route}`}
					variant="outline"
					size="sm"
					onClick={() => handleNavigateTo(route, false)}
					className="rounded-full px-4 gap-2 font-medium"
				>
					{icon}
					{getRouteLabel(route)}
				</Button>,
			];
		}

		if (configuredRoutes.length === 2) {
			return [
				<div
					key="route-nav"
					className="inline-flex items-center rounded-full bg-muted/50 p-0.5"
				>
					{configuredRoutes.map((route) => {
						const icon = getRouteIcon(route);
						return (
							<button
								key={route}
								type="button"
								onClick={() => handleNavigateTo(route, false)}
								className="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-full transition-all text-muted-foreground hover:text-foreground hover:bg-background hover:shadow-sm"
							>
								{icon}
								{getRouteLabel(route)}
							</button>
						);
					})}
				</div>,
			];
		}

		return [
			<DropdownMenu key="navigate-menu">
				<DropdownMenuTrigger asChild>
					<Button
						variant="outline"
						size="sm"
						className="rounded-full px-4 gap-2 font-medium"
					>
						Navigate
						<ChevronDownIcon className="h-3.5 w-3.5 opacity-60" />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="start" className="min-w-40">
					{configuredRoutes.map((route) => {
						const icon = getRouteIcon(route);
						return (
							<DropdownMenuItem
								key={route}
								onSelect={() => handleNavigateTo(route, false)}
								className="gap-2"
							>
								{icon}
								{getRouteLabel(route)}
							</DropdownMenuItem>
						);
					})}
				</DropdownMenuContent>
			</DropdownMenu>,
		];
	}, [configuredRoutes, handleNavigateTo, getRouteLabel]);

	useEffect(() => {
		if (!toolbarRef?.current) return;
		toolbarRef.current.pushNavElements(toolbarElements);
	}, [toolbarElements, toolbarRef]);

	const setFieldValue = useCallback((name: string, value: unknown) => {
		setValues((prev) => ({ ...prev, [name]: value }));
	}, []);

	const parseJsonField = (
		raw: unknown,
	): { ok: true; value: unknown } | { ok: false; error: string } => {
		if (raw == null) return { ok: true, value: undefined };
		if (typeof raw !== "string") return { ok: true, value: raw };
		const text = raw.trim();
		if (!text) return { ok: true, value: undefined };
		try {
			return { ok: true, value: JSON.parse(text) };
		} catch {
			return { ok: false, error: "Invalid JSON" };
		}
	};

	const buildRunPayload = useCallback(async (): Promise<IRunPayload | null> => {
		const nextFieldErrors: Record<string, string> = {};
		const payload: Record<string, unknown> = {};

		for (const pin of inputPins) {
			const key = pin.name;

			if (isAttachmentInput(pin)) {
				const selected = files[key] ?? [];
				if (selected.length === 0) continue;
				const urls: string[] = [];
				for (const file of selected) {
					const url = await backend.helperState.fileToUrl(file, false);
					urls.push(url);
				}
				payload[key] = isMultiValueInput(pin) ? urls : urls[0];
				continue;
			}

			const raw = values[key];

			if (
				isMultiValueInput(pin) ||
				pin.data_type === "Struct" ||
				pin.data_type === "Generic"
			) {
				const parsed = parseJsonField(raw);
				if (!parsed.ok) {
					nextFieldErrors[key] = parsed.error;
					continue;
				}
				if (parsed.value !== undefined) payload[key] = parsed.value;
				continue;
			}

			switch (pin.data_type) {
				case "Boolean": {
					payload[key] = Boolean(raw);
					break;
				}
				case "Integer": {
					if (raw === "" || raw == null) break;
					const num = Number.parseInt(String(raw), 10);
					if (Number.isNaN(num)) {
						nextFieldErrors[key] = "Invalid integer";
						break;
					}
					payload[key] = num;
					break;
				}
				case "Float": {
					if (raw === "" || raw == null) break;
					const num = Number.parseFloat(String(raw));
					if (Number.isNaN(num)) {
						nextFieldErrors[key] = "Invalid number";
						break;
					}
					payload[key] = num;
					break;
				}
				case "Date": {
					if (typeof raw === "string" && raw.trim()) payload[key] = raw;
					break;
				}
				default: {
					if (raw === "" || raw == null) break;
					payload[key] = raw;
					break;
				}
			}
		}

		setFieldErrors(nextFieldErrors);
		if (Object.keys(nextFieldErrors).length > 0) {
			return null;
		}

		return {
			id: event.node_id,
			payload,
		};
	}, [backend.helperState, event.node_id, files, inputPins, values]);

	const run = useCallback(async () => {
		setError(null);
		setIsRunning(true);
		setIsComplete(false);

		try {
			const active = activeSubscriptionRef.current;
			if (active) {
				executionEngine.unsubscribeFromEventStream(
					active.streamId,
					active.subscriberId,
				);
				activeSubscriptionRef.current = null;
			}

			const runPayload = await buildRunPayload();
			if (!runPayload) {
				setIsRunning(false);
				return;
			}

			resetOutput();

			const streamId = `generic-${createId()}`;
			const subscriberId = `generic-subscriber-${createId()}`;
			activeSubscriptionRef.current = { streamId, subscriberId };

			executionEngine.subscribeToEventStream(
				streamId,
				subscriberId,
				(events) => {
					handleNavigationEvents(events);
					setRunEvents((prev) => [...prev, ...events]);

					// Process events for plan steps and streaming content
					for (const ev of events) {
						// Handle plan updates (from chat_stream_partial or chat_stream)
						if (
							ev.event_type === "chat_stream_partial" ||
							ev.event_type === "chat_stream" ||
							ev.event_type === "chat_out"
						) {
							if (ev.payload?.plan) {
								const planData = ev.payload.plan as {
									plan: [number, string][];
									current_step: number;
									current_message: string;
								};
								const steps: IExecutionStep[] = [];
								let activeStepId: string | undefined;

								for (const [stepId, stepText] of planData.plan) {
									const id = `step-${stepId}`;
									const colonIndex = stepText.indexOf(":");
									const title =
										colonIndex > 0
											? stepText.substring(0, colonIndex).trim()
											: stepText;
									const description =
										colonIndex > 0
											? stepText.substring(colonIndex + 1).trim()
											: undefined;

									let status: ExecutionStepStatus;
									if (stepId < planData.current_step) {
										status = "done";
									} else if (stepId === planData.current_step) {
										status = "progress";
										activeStepId = id;
									} else {
										status = "planned";
									}

									steps.push({
										id,
										title,
										description,
										status,
										reasoning:
											stepId === planData.current_step
												? planData.current_message
												: undefined,
									});
								}

								setExecutionSteps(steps);
								setCurrentStepId(activeStepId);
							}

							// Handle streaming chunk content (chat_stream_partial with chunk.choices[].delta.content)
							if (ev.payload?.chunk?.choices?.[0]?.delta?.content) {
								setStreamingContent(
									(prev) => prev + ev.payload.chunk.choices[0].delta.content,
								);
							}
							const chunkParts =
								ev.payload?.chunk?.choices?.[0]?.delta?.content_parts;
							if (Array.isArray(chunkParts)) {
								const media = chunkParts
									.map((part: IContent) => contentPartToStreamAttachment(part))
									.filter(
										(
											attachment: IStreamAttachment | undefined,
										): attachment is IStreamAttachment =>
											attachment !== undefined,
									);
								if (media.length)
									setStreamAttachments((prev) =>
										mergeStreamAttachments(prev, media),
									);
							}

							// Handle full response content from both intermediate and terminal chat events.
							if (
								ev.event_type === "chat_stream" ||
								ev.event_type === "chat_out"
							) {
								const responseMessage =
									ev.payload?.response?.choices?.[0]?.message;
								const fullContent = responseMessage?.content;
								const structuredText = Array.isArray(
									responseMessage?.content_parts,
								)
									? responseMessage.content_parts
											.filter((part: IContent) => typeof part.text === "string")
											.map((part: IContent) => part.text)
											.join("\n")
									: "";
								// Replace entire content since this is the full response
								if (typeof fullContent === "string") {
									setStreamingContent(fullContent);
								} else if (structuredText) {
									setStreamingContent(structuredText);
								}
								if (
									typeof responseMessage?.reasoning === "string" &&
									responseMessage.reasoning.trim() &&
									!ev.payload?.plan
								) {
									setExecutionSteps((prev) =>
										prev.length
											? prev
											: [
													{
														id: "response-reasoning",
														title: "Thinking",
														status:
															ev.event_type === "chat_out"
																? "done"
																: "progress",
														reasoning: responseMessage.reasoning,
													},
												],
									);
									if (ev.event_type !== "chat_out") {
										setCurrentStepId("response-reasoning");
									}
								}
								if (Array.isArray(responseMessage?.content_parts)) {
									const media = responseMessage.content_parts
										.map((part: IContent) =>
											contentPartToStreamAttachment(part),
										)
										.filter(
											(
												attachment: IStreamAttachment | undefined,
											): attachment is IStreamAttachment =>
												attachment !== undefined,
										);
									if (media.length)
										setStreamAttachments((prev) =>
											mergeStreamAttachments(prev, media),
										);
								}
							}

							// Handle attachments from streaming response
							if (
								ev.payload?.attachments &&
								Array.isArray(ev.payload.attachments) &&
								ev.payload.attachments.length > 0
							) {
								setStreamAttachments((prev) =>
									mergeStreamAttachments(
										prev,
										ev.payload.attachments as IStreamAttachment[],
									),
								);
							}
						}

						// Handle direct text output events
						if (
							ev.event_type === "text_output" ||
							ev.event_type === "stream_text"
						) {
							if (typeof ev.payload === "string") {
								setStreamingContent((prev) => prev + ev.payload);
							} else if (ev.payload?.text) {
								setStreamingContent((prev) => prev + ev.payload.text);
							}
						}

						// Handle chat completion
						if (ev.event_type === "chat_out") {
							// Mark all steps as done
							setExecutionSteps((prev) =>
								prev.map((step) => ({
									...step,
									status: step.status === "progress" ? "done" : step.status,
								})),
							);
							setCurrentStepId(undefined);
						}
					}
				},
				() => {
					setIsRunning(false);
					setIsComplete(true);
					// Finalize any in-progress steps
					setExecutionSteps((prev) =>
						prev.map((step) => ({
							...step,
							status: step.status === "progress" ? "done" : step.status,
						})),
					);
					setCurrentStepId(undefined);
				},
			);

			await executionEngine.executeEvent(streamId, {
				appId,
				eventId: event.id,
				payload: runPayload,
				streamState: false,
				path: `${pathname}?id=${appId}&eventId=${event.id}`,
				title: event.name || "Run",
				interfaceType: "generic",
			});
		} catch (e) {
			setIsRunning(false);
			setIsComplete(false);
			setError(e instanceof Error ? e.message : "Failed to run event");
		}
	}, [
		appId,
		buildRunPayload,
		event.id,
		event.name,
		executionEngine,
		handleNavigationEvents,
		pathname,
		resetOutput,
	]);

	const outputData = useMemo(() => {
		const returnEvents = runEvents.filter(
			(ev) =>
				ev.event_type === "return" ||
				ev.event_type === "output" ||
				(ev.event_type === "intercom" && ev.payload?.type === "return"),
		);

		if (returnEvents.length > 0) {
			return returnEvents[returnEvents.length - 1]?.payload;
		}

		const lastEvent = runEvents[runEvents.length - 1];
		if (lastEvent?.payload && typeof lastEvent.payload === "object") {
			return lastEvent.payload;
		}

		return null;
	}, [runEvents]);

	const hasOutput =
		runEvents.length > 0 ||
		error ||
		streamingContent ||
		executionSteps.length > 0;

	return (
		<div className="flex flex-col h-full grow min-h-0">
			<ScrollArea className="flex-1 min-h-0">
				<div className="p-6 max-w-2xl mx-auto space-y-6">
					{/* Clean form header */}
					<div className="space-y-2 pb-4 border-b">
						<h1 className="text-2xl font-semibold">
							{event.name || "Run Task"}
						</h1>
						{event.description && (
							<p className="text-muted-foreground">{event.description}</p>
						)}
					</div>

					{/* Form fields */}
					{inputPins.length > 0 ? (
						<div className="space-y-6">
							{inputPins.map((pin, index) => {
								const key = pin.name;
								const label = pin.friendly_name || pin.name;
								const help =
									pin.description && !/^\d+$/.test(pin.description)
										? pin.description
										: undefined;
								const err = fieldErrors[key];

								return (
									<div key={pin.id}>
										{isAttachmentInput(pin) ? (
											<div className="space-y-2">
												<Label className="text-sm font-medium">{label}</Label>
												<Input
													type="file"
													multiple={isMultiValueInput(pin)}
													className="cursor-pointer"
													onChange={(e) => {
														const next = Array.from(e.target.files ?? []);
														setFiles((prev) => ({ ...prev, [key]: next }));
													}}
												/>
												{files[key]?.length > 0 && (
													<p className="text-xs text-muted-foreground">
														{files[key].length} file(s) selected
													</p>
												)}
												{help && (
													<p className="text-xs text-muted-foreground">
														{help}
													</p>
												)}
												{err && (
													<p className="text-xs text-destructive">{err}</p>
												)}
											</div>
										) : pin.data_type === "Boolean" ? (
											<div className="flex items-center justify-between gap-3 rounded-lg border p-4">
												<div className="space-y-0.5">
													<Label className="text-sm font-medium">{label}</Label>
													{help && (
														<p className="text-xs text-muted-foreground">
															{help}
														</p>
													)}
												</div>
												<Switch
													checked={Boolean(values[key] ?? false)}
													onCheckedChange={(checked) =>
														setFieldValue(key, checked)
													}
												/>
											</div>
										) : pin.data_type === "Integer" ||
											pin.data_type === "Float" ? (
											<div className="space-y-2">
												<Label className="text-sm font-medium">{label}</Label>
												<Input
													type="number"
													step={pin.data_type === "Float" ? "any" : "1"}
													placeholder={`Enter ${label.toLowerCase()}`}
													value={String(values[key] ?? "")}
													onChange={(e) => setFieldValue(key, e.target.value)}
												/>
												{help && (
													<p className="text-xs text-muted-foreground">
														{help}
													</p>
												)}
												{err && (
													<p className="text-xs text-destructive">{err}</p>
												)}
											</div>
										) : pin.data_type === "Date" ? (
											<div className="space-y-2">
												<Label className="text-sm font-medium">{label}</Label>
												<Input
													type="date"
													value={String(values[key] ?? "")}
													onChange={(e) => setFieldValue(key, e.target.value)}
												/>
												{help && (
													<p className="text-xs text-muted-foreground">
														{help}
													</p>
												)}
											</div>
										) : pin.data_type === "Struct" ||
											pin.data_type === "Generic" ||
											isMultiValueInput(pin) ? (
											<div className="space-y-2">
												<Label className="text-sm font-medium">{label}</Label>
												<Textarea
													value={String(values[key] ?? "")}
													onChange={(e) => setFieldValue(key, e.target.value)}
													placeholder="Enter JSON value"
													className="font-mono text-sm min-h-24"
												/>
												{help && (
													<p className="text-xs text-muted-foreground">
														{help}
													</p>
												)}
												{err && (
													<p className="text-xs text-destructive">{err}</p>
												)}
											</div>
										) : (
											<div className="space-y-2">
												<Label className="text-sm font-medium">{label}</Label>
												<Input
													type="text"
													placeholder={`Enter ${label.toLowerCase()}`}
													value={String(values[key] ?? "")}
													onChange={(e) => setFieldValue(key, e.target.value)}
												/>
												{help && (
													<p className="text-xs text-muted-foreground">
														{help}
													</p>
												)}
											</div>
										)}
									</div>
								);
							})}
						</div>
					) : null}

					{/* Submit button area */}
					<div className="flex items-center gap-3 pt-4 border-t">
						<Button
							size="lg"
							onClick={run}
							disabled={isRunning}
							className="flex-1 gap-2"
						>
							{isRunning ? (
								<>
									<Loader2 className="h-4 w-4 animate-spin" />
									Running...
								</>
							) : (
								<>
									<Play className="h-4 w-4" />
									Send
								</>
							)}
						</Button>
						{hasOutput && !isRunning && (
							<Button
								variant="outline"
								size="lg"
								onClick={resetOutput}
								className="gap-2"
							>
								<RotateCcw className="h-4 w-4" />
								Reset
							</Button>
						)}
					</div>

					{error && (
						<div className="rounded-lg border border-destructive/50 bg-destructive/5 p-4 animate-in fade-in duration-200">
							<div className="flex items-start gap-3">
								<XCircle className="h-5 w-5 text-destructive shrink-0 mt-0.5" />
								<div>
									<p className="text-sm font-medium text-destructive">
										Something went wrong
									</p>
									<p className="text-sm text-destructive/80 mt-1">{error}</p>
								</div>
							</div>
						</div>
					)}

					{/* Show execution steps when available */}
					{executionSteps.length > 0 && (
						<ExecutionSteps
							steps={executionSteps}
							currentStepId={currentStepId}
							isComplete={isComplete}
						/>
					)}

					{/* Show streaming content when available - always visible when there's content */}
					{streamingContent && (
						<StreamingOutput
							content={streamingContent}
							isStreaming={isRunning}
						/>
					)}

					{/* Show attachments when available */}
					{streamAttachments.length > 0 && (
						<AttachmentsDisplay attachments={streamAttachments} />
					)}

					{/* Show loading indicator when running but no streaming content yet */}
					{isRunning && !streamingContent && !executionSteps.length && (
						<div className="rounded-lg border bg-muted/30 p-4">
							<div className="flex items-center gap-2 text-sm text-muted-foreground">
								<Loader2 className="h-4 w-4 animate-spin" />
								Processing...
							</div>
						</div>
					)}

					{/* Success state - only show if no streaming content */}
					{isComplete && !error && !streamingContent && (
						<div className="flex flex-col items-center py-8 animate-in fade-in zoom-in duration-300">
							<div className="flex items-center justify-center h-12 w-12 rounded-full bg-green-500/10 mb-3">
								<CheckCircle2 className="h-6 w-6 text-green-500" />
							</div>
							<p className="font-medium text-green-600 dark:text-green-400">
								All done!
							</p>
							<p className="text-sm text-muted-foreground">
								Your task completed successfully
							</p>
						</div>
					)}

					{/* Show final output data after completion (if not already shown in streaming) */}
					{isComplete && outputData && !streamingContent && (
						<CollapsibleResult outputData={outputData} />
					)}

					{/* Legacy output display (fallback for non-streaming) */}
					{hasOutput &&
						!isComplete &&
						!streamingContent &&
						runEvents.length > 0 && (
							<div className="rounded-lg border bg-muted/30 p-4 animate-in fade-in duration-200">
								<div className="flex items-center justify-between mb-3">
									<span className="text-sm font-medium">Processing</span>
									{isRunning && (
										<div className="flex items-center gap-2 text-sm text-muted-foreground">
											<Loader2 className="h-3 w-3 animate-spin" />
											Working...
										</div>
									)}
								</div>
								{outputData ? (
									<pre className="text-sm whitespace-pre-wrap break-words font-mono">
										{typeof outputData === "string"
											? outputData
											: JSON.stringify(outputData, null, 2)}
									</pre>
								) : (
									<div className="space-y-2">
										{runEvents.slice(-3).map((ev, idx) => (
											<div
												key={`${ev.event_id}-${idx}`}
												className="text-xs text-muted-foreground"
											>
												{ev.event_type}:{" "}
												{typeof ev.payload === "string"
													? ev.payload.slice(0, 100)
													: "..."}
											</div>
										))}
									</div>
								)}
							</div>
						)}
				</div>
			</ScrollArea>
		</div>
	);
}
