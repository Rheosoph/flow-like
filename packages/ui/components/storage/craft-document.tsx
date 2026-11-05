"use client";

import { DownloadIcon, Loader2Icon, PencilIcon, SaveIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type { BulkUploadProgressCallback } from "../../lib/bulk-upload";
import { Button } from "../ui/button";
import { downloadCraftFile } from "./craft-download";
import type { CraftToolTarget } from "./craft-mcp-session";
import { CraftMcpTargetButton } from "./craft-mcp-target-button";
import { type CraftViewer, craftSaveLabel } from "./craft-viewers";
import {
	FileSaveProgress,
	type FileSaveStatus,
	fileSaveError,
	storageSaveProgress,
} from "./file-save-progress";
import { useCraftMcpSession } from "./use-craft-mcp-session";
import { ViewerAbout } from "./viewer-about";
import { readWordcraftMessage } from "./wordcraft-protocol";

interface CraftDocumentProps {
	toolTarget?: CraftToolTarget;
	viewer: CraftViewer;
	url?: string;
	create?: boolean;
	filename: string;
	outputFilename?: string;
	title?: ReactNode;
	editable: boolean;
	initialEditing?: boolean;
	showDownload?: boolean;
	onSave?: (
		file: File,
		onProgress?: BulkUploadProgressCallback,
	) => Promise<void>;
	onDirtyChange?: (dirty: boolean) => void;
	onBusyChange?: (busy: boolean) => void;
	onError?: (message: string) => void;
	onReady?: () => void;
}

export function CraftDocument(props: CraftDocumentProps) {
	const [attempt, setAttempt] = useState(0);
	return (
		<CraftSession
			key={`${props.url}\n${props.create}\n${props.filename}\n${props.viewer.id}\n${attempt}`}
			{...props}
			onRetry={() => setAttempt((value) => value + 1)}
		/>
	);
}

function CraftSession({
	toolTarget,
	viewer,
	url,
	create = false,
	filename,
	outputFilename,
	title,
	editable: requestedEditable,
	initialEditing = false,
	showDownload = true,
	onSave,
	onDirtyChange,
	onBusyChange,
	onError,
	onReady,
	onRetry,
}: CraftDocumentProps & { onRetry: () => void }) {
	const editable = requestedEditable && !!onSave;
	const channel = "flow-like-craft";
	const [runtimeRequested, setRuntimeRequested] = useState(
		create || (initialEditing && editable) || !viewer.nativePreview,
	);
	const [runtimeAvailable, setRuntimeAvailable] = useState(
		process.env.NODE_ENV !== "development",
	);
	const [outputName] = useState(
		() =>
			outputFilename ??
			(viewer.extension === viewer.outputExtension
				? filename
				: `${filename.replace(/\.[^.]+$/, "")}-converted-${crypto.randomUUID()}.${viewer.outputExtension}`),
	);
	const [downloadName, setDownloadName] = useState(filename);
	const frame = useRef<HTMLIFrameElement>(null);
	const [session] = useState(() => crypto.randomUUID());
	const [status, setStatus] = useState<"loading" | "ready" | "failed">(
		"loading",
	);
	const [interactiveLoading, setInteractiveLoading] = useState(false);
	const [editing, setEditing] = useState(initialEditing && editable);
	const [dirty, setDirty] = useState(false);
	const [saving, setSaving] = useState(false);
	const [saveProgress, setSaveProgress] = useState<FileSaveStatus | null>(null);
	const [saveFailed, setSaveFailed] = useState(false);
	const [error, setError] = useState("");
	const reportedError = useRef<string | null>(null);
	const [warnings, setWarnings] = useState<string[]>([]);
	const [saved, setSaved] = useState(false);
	const [download, setDownload] = useState<Blob | null>(null);
	const [downloadUrl, setDownloadUrl] = useState(url ?? "");
	const sourceBlob = useRef<Blob | null>(null);
	const active = useRef(true);
	const saveAction = useRef<() => void>(() => {});
	const pending = useRef<{
		id: string;
		timer: ReturnType<typeof setTimeout>;
		persisting: boolean;
		purpose: "save" | "download";
		onTimeout: () => void;
	} | null>(null);
	const latest = useRef({
		editable,
		editing,
		onSave,
		onDirtyChange,
		onBusyChange,
		onError,
		onReady,
	});
	latest.current = {
		editable,
		editing,
		onSave,
		onDirtyChange,
		onBusyChange,
		onError,
		onReady,
	};
	const reportError = useCallback((message: string) => {
		setError(message);
		if (reportedError.current === message) return;
		reportedError.current = message;
		latest.current.onError?.(message);
	}, []);

	useEffect(() => {
		if (!download) return;
		const objectUrl = URL.createObjectURL(download);
		setDownloadUrl(objectUrl);
		return () => URL.revokeObjectURL(objectUrl);
	}, [download]);

	const post = useCallback(
		(data: Record<string, unknown>, transfer: Transferable[] = []) => {
			const runtimeFrame = frame.current;
			if (!runtimeFrame?.contentWindow) return;
			runtimeFrame.contentWindow.postMessage(
				{ channel, session, ...data },
				new URL(runtimeFrame.src, runtimeFrame.ownerDocument.baseURI).origin,
				transfer,
			);
		},
		[session],
	);

	useEffect(() => {
		if (!runtimeRequested || runtimeAvailable) return;
		const controller = new AbortController();
		let timer: ReturnType<typeof setTimeout>;
		const check = async () => {
			try {
				const response = await fetch(`/craft/${viewer.id}/runtime.json`, {
					cache: "no-store",
					signal: controller.signal,
				});
				if (response.ok) {
					setRuntimeAvailable(true);
					return;
				}
			} catch {
				if (controller.signal.aborted) return;
			}
			if (!controller.signal.aborted) timer = setTimeout(check, 2000);
		};
		void check();
		return () => {
			controller.abort();
			clearTimeout(timer);
		};
	}, [runtimeRequested, runtimeAvailable, viewer.id]);

	const finishSave = useCallback(() => {
		if (pending.current) clearTimeout(pending.current.timer);
		pending.current = null;
		setSaving(false);
		setSaveProgress(null);
		latest.current.onBusyChange?.(false);
	}, []);
	useCraftMcpSession({
		target: toolTarget,
		sessionId: session,
		editor: viewer.id,
		channel,
		frame: () => frame.current,
		post,
		ready: status === "ready" && runtimeRequested && runtimeAvailable,
		editable,
		editing,
		busy: saving,
	});

	useEffect(() => {
		active.current = true;
		setInteractiveLoading(false);
		const runtimeFrame = frame.current;
		const ownerWindow = runtimeFrame?.ownerDocument.defaultView ?? window;
		const origin = runtimeFrame
			? new URL(runtimeFrame.src, runtimeFrame.ownerDocument.baseURI).origin
			: window.location.origin;
		const controller = new AbortController();
		let ready = false;
		let opened = false;
		let sent = false;
		let bytes: ArrayBuffer | undefined;
		let timeout: ReturnType<typeof setTimeout> | undefined;
		const onLoadTimeout = () => {
			if (runtimeRequested && runtimeAvailable && !opened) {
				controller.abort();
				setStatus("failed");
				reportError(
					"The document viewer did not start. Try opening the file again.",
				);
			}
		};
		const resetLoadTimeout = () => {
			clearTimeout(timeout);
			timeout = setTimeout(onLoadTimeout, 90_000);
		};
		resetLoadTimeout();
		const open = () => {
			if (!ready || sent || (!create && !bytes) || controller.signal.aborted)
				return;
			sent = true;
			if (create) {
				post({
					type: "create",
					name: filename,
					readOnly: !latest.current.editable || !latest.current.editing,
				});
				return;
			}
			if (!bytes) return;
			post(
				{
					type: "open",
					name: filename,
					bytes,
					readOnly: !latest.current.editable || !latest.current.editing,
				},
				[bytes],
			);
			bytes = undefined;
		};
		const receive = (event: MessageEvent) => {
			const message = readWordcraftMessage(
				event,
				frame.current?.contentWindow ?? null,
				origin,
				session,
				channel,
				viewer.maxBytes,
			);
			if (!message || controller.signal.aborted) return;
			switch (message.type) {
				case "ready":
					ready = true;
					open();
					break;
				case "loading":
					if (!sent || opened) break;
					setInteractiveLoading(message.interactive);
					resetLoadTimeout();
					break;
				case "opened":
					if (opened) break;
					setWarnings([
						...(message.previewOnly
							? [
									"This file opened from its embedded preview. Original layers and raw image data are unavailable.",
								]
							: []),
						...(message.warnings ?? []),
					]);
					opened = true;
					clearTimeout(timeout);
					setStatus("ready");
					latest.current.onReady?.();
					break;
				case "dirty":
					if (!opened) break;
					setDirty(message.dirty);
					if (message.dirty) setSaved(false);
					latest.current.onDirtyChange?.(message.dirty);
					break;
				case "save-request":
					if (opened) saveAction.current();
					break;
				case "export-progress": {
					const request = pending.current;
					if (
						!request ||
						request.id !== message.requestId ||
						request.persisting
					)
						break;
					// Active exports can exceed the timeout for an unresponsive viewer.
					clearTimeout(request.timer);
					request.timer = setTimeout(request.onTimeout, 10 * 60_000);
					setSaveProgress({
						stage: {
							preparing: "Preparing video",
							rendering: "Rendering video",
							finalizing: "Finalizing video",
						}[message.stage],
						progress: message.progress,
					});
					break;
				}
				case "error":
					if (message.requestId && message.requestId !== pending.current?.id)
						break;
					reportError(message.error || "The document could not be processed.");
					setSaveFailed(!!pending.current && !pending.current.persisting);
					if (!opened) {
						clearTimeout(timeout);
						setStatus("failed");
					}
					if (!pending.current?.persisting) finishSave();
					break;
				case "exported": {
					const request = pending.current;
					if (
						!request ||
						request.id !== message.requestId ||
						request.persisting
					)
						break;
					clearTimeout(request.timer);
					if (
						request.purpose === "save" &&
						(!latest.current.editable || !latest.current.onSave)
					) {
						setSaveFailed(true);
						reportError("You do not have permission to save this file.");
						finishSave();
						break;
					}
					request.persisting = true;
					setSaveProgress({ stage: "Saving file", progress: null });
					if (message.warnings?.length) setWarnings(message.warnings);
					const file = new File([message.bytes], outputName, {
						type: viewer.mime,
					});
					if (request.purpose === "download") {
						try {
							downloadCraftFile(file);
						} catch (cause) {
							reportError(fileSaveError(cause));
						} finally {
							finishSave();
						}
						break;
					}
					const onSave = latest.current.onSave;
					if (!onSave) {
						finishSave();
						break;
					}
					void (async () => {
						try {
							await onSave(file, (percent, detail) => {
								if (!active.current || pending.current !== request) return;
								setSaveProgress(storageSaveProgress(percent, detail));
							});
							if (!active.current || pending.current !== request) return;
							setDownload(file);
							setDownloadName(file.name);
							post({ type: "saved", revision: message.revision });
							setSaved(true);
						} catch (cause) {
							if (!active.current || pending.current !== request) return;
							setSaveFailed(true);
							reportError(fileSaveError(cause));
						} finally {
							if (active.current && pending.current === request) finishSave();
						}
					})();
					break;
				}
			}
		};
		ownerWindow.addEventListener("message", receive);
		void (async () => {
			try {
				if (create) {
					open();
					return;
				}
				if (!url)
					throw new Error(
						"Choose a document to open or create a new document.",
					);
				if (runtimeRequested && sourceBlob.current) {
					bytes = await sourceBlob.current.arrayBuffer();
					open();
					return;
				}
				const response = await fetch(url, {
					signal: controller.signal,
					cache: "no-store",
				});
				if (!response.ok)
					throw new Error("The document could not be downloaded.");
				if (Number(response.headers.get("content-length")) > viewer.maxBytes)
					throw new Error(
						`${viewer.label} supports files up to ${viewer.maxBytes / 1024 / 1024} MiB.`,
					);
				const blob = await response.blob();
				if (blob.size > viewer.maxBytes)
					throw new Error(
						`${viewer.label} supports files up to ${viewer.maxBytes / 1024 / 1024} MiB.`,
					);
				if (blob.size === 0) throw new Error("This document is empty.");
				if (controller.signal.aborted) return;
				sourceBlob.current = new Blob([blob], { type: viewer.inputMime });
				setDownload(sourceBlob.current);
				if (!runtimeRequested) {
					clearTimeout(timeout);
					setStatus("ready");
					latest.current.onReady?.();
					return;
				}
				bytes = await blob.arrayBuffer();
				open();
			} catch (cause) {
				if (controller.signal.aborted) return;
				clearTimeout(timeout);
				setStatus("failed");
				reportError(
					cause instanceof Error ? cause.message : "Could not load document.",
				);
			}
		})();
		return () => {
			active.current = false;
			controller.abort();
			clearTimeout(timeout);
			if (pending.current) clearTimeout(pending.current.timer);
			pending.current = null;
			ownerWindow.removeEventListener("message", receive);
			latest.current.onDirtyChange?.(false);
			latest.current.onBusyChange?.(false);
		};
	}, [
		url,
		create,
		filename,
		session,
		post,
		finishSave,
		reportError,
		viewer,
		outputName,
		runtimeRequested,
		runtimeAvailable,
	]);

	useEffect(() => {
		if (!editable) setEditing(false);
		if (runtimeRequested && status === "ready")
			post({ type: "set-read-only", readOnly: !editable || !editing });
	}, [editable, editing, status, post, runtimeRequested]);

	function exportDocument(purpose: "save" | "download") {
		if (pending.current || status !== "ready") return;
		if (purpose === "save" && (!editable || !editing || !onSave)) return;
		const id = crypto.randomUUID();
		setError("");
		reportedError.current = null;
		setSaveFailed(false);
		if (purpose === "save") setSaved(false);
		setSaving(true);
		setSaveProgress({ stage: "Preparing file", progress: null });
		latest.current.onBusyChange?.(true);
		const onTimeout = () => {
			if (pending.current?.id !== id) return;
			setSaveFailed(true);
			reportError("Preparing the file took too long.");
			finishSave();
		};
		pending.current = {
			id,
			persisting: false,
			purpose,
			onTimeout,
			timer: setTimeout(
				onTimeout,
				viewer.id === "filmcraft" ? 10 * 60_000 : 60_000,
			),
		};
		post({
			type: "export",
			requestId: id,
			format: viewer.outputExtension,
			...(purpose === "download" ? { purpose } : {}),
		});
	}
	function saveDocument() {
		exportDocument("save");
	}
	saveAction.current = onSave ? saveDocument : () => exportDocument("download");

	return (
		<div className="flex h-full min-h-0 flex-col bg-background">
			<div className="flex min-w-0 shrink-0 items-center gap-2 border-b px-3 py-3 pr-12">
				<div className="min-w-0 flex-1 truncate text-sm font-semibold">
					{title ?? filename}
				</div>
				<span
					className="hidden shrink-0 text-xs text-muted-foreground sm:inline"
					aria-live="polite"
				>
					{saving
						? pending.current?.purpose === "download"
							? "Preparing download..."
							: "Saving..."
						: dirty
							? "Unsaved changes"
							: saved
								? "Saved"
								: ""}
				</span>
				<ViewerAbout
					label={viewer.label}
					licensesUrl={`/craft/${viewer.id}/licenses/index.html`}
				>
					{editing ? (
						<>
							{viewer.label} editing is experimental. Formatting and unsupported
							features may change when saved.
							{viewer.extension !== viewer.outputExtension &&
								` This format is imported; saving creates a .${viewer.outputExtension} file.`}
						</>
					) : undefined}
				</ViewerAbout>
				{toolTarget &&
					status === "ready" &&
					runtimeRequested &&
					runtimeAvailable && (
						<CraftMcpTargetButton target={toolTarget} onError={reportError} />
					)}
				{showDownload && (
					<Button asChild variant="ghost" size="sm" className="shrink-0">
						<a
							href={downloadUrl || undefined}
							aria-disabled={status !== "ready" || saving}
							onClick={(event) => {
								if (status !== "ready" || saving) {
									event.preventDefault();
									return;
								}
								if (editing || dirty || create) {
									event.preventDefault();
									exportDocument("download");
								}
							}}
							download={downloadName}
							target="_blank"
							rel="noopener noreferrer"
							aria-label={`Download ${downloadName}`}
						>
							<DownloadIcon />{" "}
							<span className="hidden sm:inline">Download</span>
						</a>
					</Button>
				)}
				{editable && !editing && (
					<Button
						variant="outline"
						size="sm"
						className="shrink-0"
						aria-label="Edit file"
						disabled={status !== "ready"}
						onClick={() => {
							if (!runtimeRequested) {
								setStatus("loading");
								setRuntimeRequested(true);
							}
							setEditing(true);
						}}
					>
						<PencilIcon /> <span className="hidden sm:inline">Edit</span>
					</Button>
				)}
				{editable && editing && (
					<Button
						size="sm"
						className="shrink-0"
						aria-label={craftSaveLabel(viewer)}
						disabled={status !== "ready" || saving}
						onClick={saveDocument}
					>
						{saving ? <Loader2Icon className="animate-spin" /> : <SaveIcon />}{" "}
						{craftSaveLabel(viewer)}
					</Button>
				)}
			</div>
			{saving && saveProgress && <FileSaveProgress {...saveProgress} />}
			{viewer.id === "solidcraft" && editing && (
				<p className="border-b px-3 py-2 text-xs text-muted-foreground">
					Saved projects require SolidCraft 0.30 or later. Keep the original
					file if you need to use an older version.
				</p>
			)}
			{warnings.length > 0 && (
				<output className="max-h-20 overflow-auto border-b px-3 py-2 text-xs text-muted-foreground">
					{warnings.join(" ")}
				</output>
			)}
			{error && (
				<div
					role="alert"
					className="shrink-0 border-b px-3 py-2 text-sm text-destructive"
				>
					{saveFailed && <p className="font-medium">Could not save file</p>}
					<p>{error}</p>
					{saveFailed && (
						<>
							<p>Your changes are still open. Try saving again.</p>
							{editable && editing && status === "ready" && !saving && (
								<Button
									variant="outline"
									size="sm"
									className="mt-2"
									onClick={saveDocument}
								>
									Try saving again
								</Button>
							)}
						</>
					)}
				</div>
			)}
			<div className="relative flex-1 min-h-0">
				{runtimeRequested && runtimeAvailable && status !== "failed" && (
					<iframe
						ref={frame}
						src={`/craft/${viewer.id}/index.html?session=${encodeURIComponent(session)}`}
						title={`${viewer.label}: ${filename}`}
						className="h-full w-full border-0"
						allow="clipboard-read; clipboard-write"
					/>
				)}
				{!runtimeRequested && status === "ready" && downloadUrl !== url && (
					<NativePreview
						viewer={viewer}
						url={downloadUrl}
						filename={filename}
					/>
				)}
				{status === "loading" && (
					<output
						className={
							interactiveLoading
								? "sr-only"
								: "absolute inset-0 flex items-center justify-center gap-2 bg-background"
						}
					>
						<Loader2Icon className="size-4 animate-spin" />
						{runtimeRequested && !runtimeAvailable
							? `Waiting for ${viewer.label} to finish building...`
							: `Loading ${viewer.label}...`}
					</output>
				)}
				{status === "failed" && (
					<div className="flex h-full items-center justify-center">
						<Button variant="outline" onClick={onRetry}>
							Try again
						</Button>
					</div>
				)}
			</div>
		</div>
	);
}

function NativePreview({
	viewer,
	url,
	filename,
}: { viewer: CraftViewer; url: string; filename: string }) {
	switch (viewer.nativePreview) {
		case "pdf":
			return (
				<iframe
					src={`${url}#toolbar=1&view=FitH`}
					title={`PDF Preview: ${filename}`}
					className="h-full w-full border-0"
				/>
			);
		case "image":
			return (
				<img
					src={url}
					alt={filename}
					className="h-full w-full object-contain"
				/>
			);
		case "audio":
			return (
				<div className="flex h-full items-center justify-center">
					<audio src={url} controls aria-label={filename}>
						<track kind="captions" />
					</audio>
				</div>
			);
		case "video":
			return (
				<video
					src={url}
					controls
					className="h-full w-full object-contain"
					aria-label={filename}
				>
					<track kind="captions" />
				</video>
			);
		default:
			return null;
	}
}
