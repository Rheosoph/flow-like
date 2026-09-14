"use client";

import { useTranslation } from "@flow-like/locales";
import { Camera, Mic, Pause, Play, Square } from "lucide-react";
import { usePathname } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { useActionContext, useComponentEventTrigger } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import {
	type CameraEffects,
	CameraError,
	type CameraOverlay,
	cameraFilter,
	cameraImageRect,
	cameraOverlayX,
	parseCameraEffects,
} from "../camera-overlays";
import { CameraSession, type CameraSessionState } from "../camera-session";
import { firstEventAction } from "../event-handlers";
import type { CameraViewComponent } from "../types";

const EXACT = { legacyFallback: false, wildcardFallback: false };
function remotelyReadable(url: string) {
	try {
		const parsed = new URL(url);
		return (
			["http:", "https:"].includes(parsed.protocol) &&
			parsed.hostname.toLowerCase() !== "asset.localhost"
		);
	} catch {
		return false;
	}
}
const stopped: CameraSessionState = {
	status: "stopped",
	width: 0,
	height: 0,
	busy: false,
};

export function A2UICameraView({
	component,
	componentId,
	surfaceId,
	appId: propAppId,
	elementRef,
	style,
}: ComponentProps<CameraViewComponent>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const actionContext = useActionContext();
	const appId = actionContext.appId ?? propAppId;
	const pathname = usePathname();
	const { resolve, setByPath } = useData();
	const triggerEvent = useComponentEventTrigger(componentId);
	const value = <T,>(key: keyof CameraViewComponent, fallback: T): T =>
		component[key]
			? ((resolve(component[key] as never) as T) ?? fallback)
			: fallback;
	// In A2UI, preview mode is the executing runtime; the editing canvas uses false.
	const disabled = value("disabled", false) || !actionContext.isPreviewMode;
	const intervalMs = value("intervalMs", 0);
	const maxWidth = value("maxWidth", 1280);
	const quality = value("quality", 0.85);
	const audioEnabled = value("audioEnabled", false);
	const audioBufferSeconds = value("audioBufferSeconds", 60);
	const facingMode = value<string>("facingMode", "environment");
	const deviceId = value("deviceId", "");
	const mirrored = value("mirrored", facingMode === "user");
	const fit = value<string>("fit", "contain") === "cover" ? "cover" : "contain";
	const label = value("label", t("camera", "Camera"));
	const videoRef = useRef<HTMLVideoElement>(null);
	const containerRef = useRef<HTMLDivElement>(null);
	const sessionRef = useRef<CameraSession | null>(null);
	const visibleRef = useRef(true);
	const dispatchingRef = useRef(false);
	const dispatchRevision = useRef(0);
	const captureEventRef = useRef("capture");
	const latest = useRef({
		backend,
		actionContext,
		component,
		disabled,
		triggerEvent,
		appId,
	});
	latest.current = {
		backend,
		actionContext,
		component,
		disabled,
		triggerEvent,
		appId,
	};
	const [state, setState] = useState<CameraSessionState>(stopped);
	const [error, setError] = useState<string>();
	const [eventBusy, setEventBusy] = useState(false);
	const [containerSize, setContainerSize] = useState({
		width: 640,
		height: 360,
	});
	const rootRef = useCallback(
		(node: HTMLDivElement | null) => {
			containerRef.current = node;
			elementRef?.(node);
		},
		[elementRef],
	);

	// biome-ignore lint/correctness/useExhaustiveDependencies: A route change ends the mounted camera's permission scope.
	useEffect(() => {
		let mounted = true;
		setState(stopped);
		setError(undefined);
		setEventBusy(false);
		dispatchingRef.current = false;
		++dispatchRevision.current;
		const isVisible = () => {
			if (
				!mounted ||
				latest.current.disabled ||
				!visibleRef.current ||
				document.visibilityState === "hidden" ||
				!containerRef.current?.isConnected ||
				containerRef.current.closest('[hidden], [aria-hidden="true"]')
			)
				return false;
			for (
				let node: HTMLElement | null = containerRef.current;
				node;
				node = node.parentElement
			) {
				const computed = getComputedStyle(node);
				if (computed.display === "none" || computed.visibility === "hidden")
					return false;
			}
			return true;
		};
		const session = new CameraSession({
			appId,
			surfaceId,
			componentId,
			video: () => videoRef.current,
			isVisible,
			onState: (next) => {
				if (mounted) setState(next);
			},
			upload: async (file, context) => {
				const current = latest.current;
				const executionTarget =
					context?.executionTarget ??
					(await current.actionContext.resolveTemporaryUploadTarget?.(
						firstEventAction(
							current.component.eventHandlers,
							captureEventRef.current,
							undefined,
							EXACT,
						),
					));
				if (!isVisible() || context?.signal?.aborted || session.signal.aborted)
					throw new CameraError(
						"screen_closed",
						"The camera screen closed before its media could be shared.",
					);
				const signal = AbortSignal.any([
					session.signal,
					...(context?.signal ? [context.signal] : []),
				]);
				const helper = current.backend.helperState;
				const result = helper.filesToTemporaryFiles
					? (
							await helper.filesToTemporaryFiles([file], {
								offline: false,
								appId,
								executionTarget,
								signal,
							})
						)[0]
					: undefined;
				if (helper.filesToTemporaryFiles && !result?.uploaded)
					throw new CameraError(
						"upload_failed",
						result?.error || "The camera media could not be uploaded.",
					);
				const temporary = result?.uploaded ??
					(await helper.fileToTemporaryFile?.(
						file,
						false,
						appId,
						executionTarget,
					)) ?? {
						url: await helper.fileToUrl(file, false, appId, executionTarget),
					};
				if (!isVisible() || signal.aborted)
					throw new CameraError(
						"screen_closed",
						"The camera screen closed while its media was being shared.",
					);
				if (executionTarget === "remote" && !remotelyReadable(temporary.url))
					throw new CameraError(
						"upload_unavailable",
						"This Event needs a remote temporary media upload. Sign in and try again.",
					);
				return {
					name: file.name,
					type: file.type,
					size: file.size,
					url: temporary.url,
					backendUrl: temporary.url,
					flowPath: temporary.flowPath,
				};
			},
		});
		sessionRef.current = session;
		const hide = () => {
			if (document.visibilityState === "hidden") session.stop();
		};
		const stop = () => session.stop();
		document.addEventListener("visibilitychange", hide);
		window.addEventListener("pagehide", stop);
		window.addEventListener("flow-like:device-inactive", stop);
		const hiddenObserver = new MutationObserver(() => {
			if (!isVisible() && session.state.status !== "stopped") session.stop();
		});
		for (
			let node: HTMLElement | null = containerRef.current;
			node;
			node = node.parentElement
		)
			hiddenObserver.observe(node, {
				attributes: true,
				attributeFilter: ["hidden", "aria-hidden", "style", "class"],
			});
		const observer =
			typeof IntersectionObserver === "undefined"
				? null
				: new IntersectionObserver(([entry]) => {
						visibleRef.current = entry?.isIntersecting ?? false;
						if (!visibleRef.current) session.stop();
					});
		if (containerRef.current) observer?.observe(containerRef.current);
		const resize =
			typeof ResizeObserver === "undefined"
				? null
				: new ResizeObserver(([entry]) => {
						if (entry)
							setContainerSize({
								width: entry.contentRect.width,
								height: entry.contentRect.height,
							});
					});
		if (containerRef.current) resize?.observe(containerRef.current);
		return () => {
			mounted = false;
			session.dispose();
			if (sessionRef.current === session) sessionRef.current = null;
			document.removeEventListener("visibilitychange", hide);
			window.removeEventListener("pagehide", stop);
			window.removeEventListener("flow-like:device-inactive", stop);
			observer?.disconnect();
			resize?.disconnect();
			hiddenObserver.disconnect();
		};
	}, [appId, surfaceId, componentId, pathname]);

	useEffect(() => {
		if (disabled) sessionRef.current?.stop();
	}, [disabled]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Input configuration changes require another explicit Start.
	useEffect(() => {
		sessionRef.current?.stop();
	}, [deviceId, facingMode, audioEnabled, audioBufferSeconds]);
	// Buffer progress updates bindings without replacing the most recently captured frame.
	useEffect(() => {
		const bound = latest.current.component.value;
		if (!bound || !("path" in bound) || state.audioEnabled === undefined)
			return;
		for (const [key, item] of Object.entries({
			audioEnabled: state.audioEnabled,
			audioActive: state.audioActive ?? false,
			audioBufferSeconds: state.audioBufferSeconds ?? 60,
			audioAvailableMs: state.audioAvailableMs ?? 0,
		}))
			setByPath(`${bound.path}/${key}`, item);
	}, [
		state.audioEnabled,
		state.audioActive,
		state.audioBufferSeconds,
		state.audioAvailableMs,
		setByPath,
	]);
	const overlays = component.overlays ? resolve(component.overlays) : undefined;
	// literalJson resolves to a fresh object; use its value so applying an update
	// does not schedule itself again or keep extending the overlay lifetime.
	const overlayJson = overlays ? JSON.stringify(overlays) : undefined;
	useEffect(() => {
		if (!overlayJson) return;
		try {
			sessionRef.current?.updateOverlays(JSON.parse(overlayJson));
		} catch (reason) {
			if (!(reason instanceof CameraError && reason.code === "stale_frame"))
				setError(
					reason instanceof Error ? reason.message : "Invalid camera overlays.",
				);
		}
	}, [overlayJson]);
	let effects: CameraEffects = {};
	try {
		effects = parseCameraEffects(value("effects", {}));
	} catch {
		/* Invalid effects never reach CSS. */
	}

	const reportError = useCallback(
		(reason: unknown) => {
			const message =
				reason instanceof Error
					? reason.message
					: t(
							"cameraActionFailed",
							"The camera action could not be completed.",
						);
			setError(message);
			if (sessionRef.current?.signal.aborted) return;
			void latest.current.triggerEvent(
				"error",
				latest.current.component,
				{
					code: reason instanceof CameraError ? reason.code : "camera_error",
					message,
				},
				EXACT,
			);
		},
		[t],
	);

	const capture = useCallback(
		async (eventName: "capture" | "frame") => {
			const session = sessionRef.current;
			if (!session || dispatchingRef.current || latest.current.disabled) return;
			// An interval without an explicitly bound Event never exports images.
			if (
				eventName === "frame" &&
				!firstEventAction(
					latest.current.component.eventHandlers,
					"frame",
					undefined,
					EXACT,
				)
			)
				return;
			dispatchingRef.current = true;
			const revision = ++dispatchRevision.current;
			setEventBusy(true);
			captureEventRef.current = eventName;
			const signal = session.signal;
			try {
				setError(undefined);
				const frame = await session.capture(
					{ appId, signal },
					maxWidth,
					quality,
				);
				if (signal.aborted) return;
				const bound = latest.current.component.value;
				if (bound && "path" in bound)
					setByPath(bound.path, {
						...frame,
						audioEnabled: session.state.audioEnabled ?? false,
						audioActive: session.state.audioActive ?? false,
						audioBufferSeconds: session.state.audioBufferSeconds ?? 60,
						audioAvailableMs: session.state.audioAvailableMs ?? 0,
					});
				await latest.current.triggerEvent(
					eventName,
					latest.current.component,
					{ ...frame, frame, value: frame },
					EXACT,
				);
			} catch (reason) {
				if (!signal.aborted) reportError(reason);
			} finally {
				if (revision === dispatchRevision.current) {
					dispatchingRef.current = false;
					if (!signal.aborted) setEventBusy(false);
				}
			}
		},
		[appId, maxWidth, quality, reportError, setByPath],
	);

	useEffect(() => {
		if (
			disabled ||
			state.status !== "live" ||
			!Number.isFinite(intervalMs) ||
			intervalMs <= 0
		)
			return;
		const timer = setInterval(
			() => {
				void capture("frame");
			},
			Math.max(250, intervalMs),
		);
		return () => clearInterval(timer);
	}, [capture, disabled, intervalMs, state.status]);
	useEffect(() => {
		if (state.status === "stopped") setEventBusy(false);
	}, [state.status]);

	const start = async () => {
		setError(undefined);
		if (typeof navigator.mediaDevices?.getUserMedia !== "function") {
			reportError(
				new CameraError(
					"unsupported",
					"Camera access requires a supported browser and HTTPS or localhost.",
				),
			);
			return;
		}
		try {
			const current = sessionRef.current;
			await current?.start(
				deviceId
					? { deviceId: { exact: deviceId }, width: { ideal: 1280 } }
					: { facingMode: { ideal: facingMode }, width: { ideal: 1280 } },
				audioEnabled,
				audioBufferSeconds,
			);
			if (current === sessionRef.current && current?.state.status === "live") {
				const session = {
					sessionId: current.state.sessionId,
					surfaceId,
					componentId,
					status: "live",
					audioEnabled,
					audioActive: current.state.audioActive ?? false,
					audioBufferSeconds,
					audioAvailableMs: current.state.audioAvailableMs ?? 0,
				};
				const bound = latest.current.component.value;
				if (bound && "path" in bound) setByPath(bound.path, session);
				await triggerEvent("ready", component, session, EXACT);
			}
		} catch (reason) {
			reportError(reason);
		}
	};
	const running = state.status === "live" || state.status === "frozen";
	const busy = state.busy || eventBusy || state.status === "starting";
	const rect = cameraImageRect(
		containerSize,
		{ width: state.width, height: state.height },
		fit,
	);
	const clickOverlay = (overlay: CameraOverlay) => {
		void triggerEvent(
			"overlayClick",
			component,
			{
				overlay,
				sessionId: state.sessionId,
				frameId: state.frameId,
				surfaceId,
				componentId,
			},
			EXACT,
		);
	};

	return (
		<section
			className={cn("flex min-w-0 flex-col gap-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
			aria-label={label}
		>
			<div
				ref={rootRef}
				className="relative isolate aspect-video min-h-32 overflow-hidden rounded-lg bg-black"
				data-camera-status={state.status}
			>
				<video
					ref={videoRef}
					autoPlay
					muted
					playsInline
					aria-label={label}
					className="h-full w-full"
					style={{
						objectFit: fit,
						transform: mirrored ? "scaleX(-1)" : undefined,
						filter: cameraFilter(state.overlays?.effects ?? effects),
					}}
				/>
				{running && state.overlays && (
					<div
						className="pointer-events-none absolute"
						style={{
							left: rect.x,
							top: rect.y,
							width: rect.width,
							height: rect.height,
						}}
					>
						{state.overlays.overlays.map((overlay) => (
							<CameraAnnotation
								key={overlay.id}
								overlay={overlay}
								mirrored={mirrored}
								onClick={() => clickOverlay(overlay)}
								interactive={Boolean(
									component.eventHandlers?.overlayClick?.length,
								)}
							/>
						))}
					</div>
				)}
				{!running && (
					<div className="pointer-events-none absolute inset-0 flex items-center justify-center gap-2 text-sm text-white/80">
						<Camera size={20} />
						{state.status === "starting"
							? t("startingCamera", "Starting camera…")
							: t("cameraIsOff", "Camera is off")}
					</div>
				)}
				{running && (
					<output className="absolute left-2 top-2 rounded bg-black/65 px-2 py-1 text-xs text-white">
						{state.status === "frozen"
							? t("cameraFrozen", "Frozen")
							: t("cameraLive", "Live")}
						{intervalMs > 0
							? ` · ${t("automaticCapture", "Automatic capture")}`
							: ""}
					</output>
				)}
				{state.audioActive && (
					<span
						data-camera-microphone="active"
						className="absolute bottom-2 left-2 flex items-center gap-1 rounded bg-black/75 px-2 py-1 text-xs text-white"
					>
						<Mic size={14} className="text-red-400" aria-hidden="true" />
						{t("cameraMicrophoneOn", "Microphone on")}
						<span aria-hidden="true">
							{" "}
							· {Math.floor((state.audioAvailableMs ?? 0) / 1000)} s
						</span>
					</span>
				)}
			</div>
			<div className="flex flex-wrap items-center gap-2">
				{!running ? (
					<Button
						type="button"
						size="sm"
						disabled={disabled || busy}
						onClick={() => void start()}
					>
						<Camera size={16} />
						{audioEnabled
							? t("startCameraAndMicrophone", "Start camera and microphone")
							: t("startCamera", "Start camera")}
					</Button>
				) : (
					<>
						<Button
							type="button"
							size="sm"
							disabled={disabled || busy}
							onClick={() => void capture("capture")}
						>
							<Camera size={16} />
							{t("captureImage", "Capture image")}
						</Button>
						<Button
							type="button"
							size="sm"
							variant="outline"
							disabled={busy}
							onClick={() =>
								void (
									state.status === "frozen"
										? sessionRef.current?.resume()
										: sessionRef.current?.freeze()
								)?.catch(reportError)
							}
						>
							{state.status === "frozen" ? (
								<Play size={16} />
							) : (
								<Pause size={16} />
							)}
							{state.status === "frozen"
								? t("resumeCamera", "Resume")
								: t("freezeCamera", "Freeze")}
						</Button>
						<Button
							type="button"
							size="sm"
							variant="outline"
							onClick={() => sessionRef.current?.stop()}
						>
							<Square size={16} />
							{t("stopCamera", "Stop")}
						</Button>
					</>
				)}
			</div>
			{(error || state.error) && (
				<p role="alert" className="text-sm text-destructive">
					{error || state.error}
				</p>
			)}
		</section>
	);
}

function CameraAnnotation({
	overlay,
	mirrored,
	interactive,
	onClick,
}: {
	overlay: CameraOverlay;
	mirrored: boolean;
	interactive: boolean;
	onClick: () => void;
}) {
	const left =
		cameraOverlayX(overlay.x ?? 0, overlay.width ?? 0, mirrored) * 100;
	const color = overlay.color ?? "#22c55e";
	const label = overlay.text ?? overlay.id;
	if (overlay.type === "polygon")
		return (
			<svg
				role="img"
				className="absolute inset-0 h-full w-full overflow-visible"
				viewBox="0 0 1 1"
				preserveAspectRatio="none"
				aria-label={label}
			>
				<polygon
					points={overlay.points
						?.map(([x, y]) => `${cameraOverlayX(x, 0, mirrored)},${y}`)
						.join(" ")}
					fill={overlay.background ?? "transparent"}
					stroke={color}
					strokeWidth={2}
					vectorEffect="non-scaling-stroke"
					opacity={overlay.opacity ?? 1}
					style={{
						pointerEvents: interactive ? "auto" : "none",
						cursor: interactive ? "pointer" : undefined,
					}}
					role={interactive ? "button" : undefined}
					tabIndex={interactive ? 0 : undefined}
					onClick={onClick}
					onKeyDown={(event) => {
						if (interactive && (event.key === "Enter" || event.key === " ")) {
							event.preventDefault();
							onClick();
						}
					}}
				/>
			</svg>
		);
	return (
		<button
			type="button"
			disabled={!interactive}
			aria-label={label}
			onClick={onClick}
			className="absolute appearance-none text-left disabled:opacity-100"
			style={{
				left: `${left}%`,
				top: `${(overlay.y ?? 0) * 100}%`,
				width:
					overlay.width === undefined ? undefined : `${overlay.width * 100}%`,
				height:
					overlay.height === undefined ? undefined : `${overlay.height * 100}%`,
				pointerEvents: interactive ? "auto" : "none",
				opacity: overlay.opacity ?? 1,
				color,
				background:
					overlay.type === "dim"
						? "rgba(0,0,0,0.65)"
						: (overlay.background ?? "transparent"),
				backdropFilter: overlay.type === "blur" ? "blur(12px)" : undefined,
				border: overlay.type === "box" ? `2px solid ${color}` : undefined,
				fontSize: overlay.fontSize ?? 14,
				lineHeight: 1.25,
				transform:
					overlay.type === "point" ? "translate(-50%, -50%)" : undefined,
			}}
		>
			{overlay.type === "point" && (
				<span
					style={{
						display: "block",
						width: 12,
						height: 12,
						borderRadius: "50%",
						background: color,
					}}
				/>
			)}
			{overlay.text && (
				<span
					style={{
						display: "block",
						padding: "2px 4px",
						background: overlay.background ?? "rgba(0,0,0,0.7)",
						position: overlay.type === "box" ? "absolute" : undefined,
						bottom: overlay.type === "box" ? "100%" : undefined,
						whiteSpace: "pre-wrap",
					}}
				>
					{overlay.text}
					{overlay.confidence === undefined
						? ""
						: ` ${Math.round(overlay.confidence * 100)}%`}
				</span>
			)}
		</button>
	);
}
