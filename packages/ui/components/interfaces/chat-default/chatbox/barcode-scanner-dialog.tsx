"use client";

import { useTranslation } from "@flow-like/locales";
import Camera from "lucide-react/dist/esm/icons/camera.js";
import Loader2 from "lucide-react/dist/esm/icons/loader-2.js";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { captureFileName } from "../../../../lib/capture-file-name";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import {
	type BarcodeDetect,
	type BarcodeTrack,
	type DetectedBarcode,
	type LumaFrame,
	fitFrame,
	lumaFromRgba,
	mergeBarcodes,
	trackBarcodes,
	trackedBarcodes,
} from "./barcode-frames";

/**
 * Frames are read at the camera's own size. Scaled down, the bars of a shipping label fall
 * under two pixels and stop decoding.
 */
const FRAME_EDGE = 1920;
const FRAME_GAP_MS = 40;
/** How long an outline stays after the last frame that read its code. */
const HOLD_MS = 900;
const NO_CODES: BarcodeTrack = new Map();

export interface BarcodeCapture {
	readonly photo: File;
	readonly barcodes: readonly DetectedBarcode[];
}

interface LiveCodes {
	readonly width: number;
	readonly height: number;
	readonly track: BarcodeTrack;
}

function drawFrame(
	video: HTMLVideoElement,
	canvas: HTMLCanvasElement,
	reuse?: Uint8Array,
): LumaFrame {
	const { width, height } = fitFrame(
		video.videoWidth,
		video.videoHeight,
		FRAME_EDGE,
	);
	if (canvas.width !== width) canvas.width = width;
	if (canvas.height !== height) canvas.height = height;
	const context = canvas.getContext("2d", { willReadFrequently: true });
	if (!context) throw new Error("The camera frame could not be read");
	context.drawImage(video, 0, 0, width, height);
	const { data } = context.getImageData(0, 0, width, height);
	return { luma: lumaFromRgba(data, width * height, reuse), width, height };
}

function canvasToJpeg(canvas: HTMLCanvasElement): Promise<Blob> {
	return new Promise((resolve, reject) =>
		canvas.toBlob(
			(blob) =>
				blob ? resolve(blob) : reject(new Error("JPEG encoding failed")),
			"image/jpeg",
			0.9,
		),
	);
}

function CodeOutline({
	code,
	fontSize,
}: {
	readonly code: DetectedBarcode;
	readonly fontSize: number;
}) {
	if (code.points.length === 0) return null;
	const points = code.points.map((point) => `${point.x},${point.y}`).join(" ");
	const left = Math.min(...code.points.map((point) => point.x));
	const top = Math.min(...code.points.map((point) => point.y));
	return (
		<g>
			{code.points.length > 2 ? (
				<polygon
					points={points}
					className="fill-primary/20 stroke-primary"
					strokeWidth={3}
					strokeLinejoin="round"
					vectorEffect="non-scaling-stroke"
				/>
			) : (
				<polyline
					points={points}
					className="fill-none stroke-primary"
					strokeWidth={6}
					vectorEffect="non-scaling-stroke"
				/>
			)}
			<text
				x={left}
				y={top - fontSize * 0.4}
				fontSize={fontSize}
				className="fill-primary-foreground stroke-primary font-semibold"
				strokeWidth={fontSize * 0.2}
				paintOrder="stroke"
			>
				{code.text.length > 32 ? `${code.text.slice(0, 31)}…` : code.text}
			</text>
		</g>
	);
}

/** A live camera view that outlines every code it reads; the shutter hands back photo and codes. */
export function BarcodeScannerDialog({
	open,
	detect,
	onCapture,
	onCancel,
}: {
	readonly open: boolean;
	readonly detect: BarcodeDetect;
	readonly onCapture: (capture: BarcodeCapture) => void;
	readonly onCancel: () => void;
}) {
	const { t } = useTranslation();
	const videoRef = useRef<HTMLVideoElement>(null);
	const [ready, setReady] = useState(false);
	const [error, setError] = useState<string>();
	const [live, setLive] = useState<LiveCodes>();
	const [capturing, setCapturing] = useState(false);
	const liveRef = useRef(live);
	liveRef.current = live;

	useEffect(() => {
		if (!open) return;
		let stopped = false;
		let stream: MediaStream | undefined;
		const canvas = document.createElement("canvas");
		setReady(false);
		setError(undefined);
		setLive(undefined);
		void (async () => {
			try {
				stream = await navigator.mediaDevices.getUserMedia({
					video: {
						facingMode: { ideal: "environment" },
						width: { ideal: 1920 },
						height: { ideal: 1080 },
					},
					audio: false,
				});
				const video = videoRef.current;
				if (stopped || !video) return;
				video.srcObject = stream;
				await video.play();
				if (stopped) return;
				setReady(true);
				let luma: Uint8Array | undefined;
				while (!stopped) {
					if (video.videoWidth > 0) {
						const frame = drawFrame(video, canvas, luma);
						luma = frame.luma;
						const codes = await detect(frame, false).catch(() => []);
						if (stopped) break;
						setLive((previous) => ({
							width: frame.width,
							height: frame.height,
							track: trackBarcodes(
								previous?.track ?? NO_CODES,
								codes,
								performance.now(),
								HOLD_MS,
							),
						}));
					}
					await new Promise((resolve) => setTimeout(resolve, FRAME_GAP_MS));
				}
			} catch (cause) {
				if (!stopped)
					setError(
						t(
							"barcodeCameraUnavailableVal",
							"The camera could not start: {{val}}",
							{ val: cause instanceof Error ? cause.message : String(cause) },
						),
					);
			}
		})();
		return () => {
			stopped = true;
			for (const track of stream?.getTracks() ?? []) track.stop();
			if (videoRef.current) videoRef.current.srcObject = null;
		};
	}, [open, detect, t]);

	const capture = useCallback(async () => {
		const video = videoRef.current;
		if (!video || video.videoWidth === 0) return;
		setCapturing(true);
		try {
			const canvas = document.createElement("canvas");
			const frame = drawFrame(video, canvas);
			const photo = new File(
				[await canvasToJpeg(canvas)],
				captureFileName("barcodes", new Date(), ".jpg"),
				{ type: "image/jpeg" },
			);
			const thorough = await detect(frame, true).catch(() => []);
			onCapture({
				photo,
				barcodes: mergeBarcodes(
					thorough,
					trackedBarcodes(liveRef.current?.track ?? NO_CODES),
				),
			});
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setCapturing(false);
		}
	}, [detect, onCapture]);

	const codes = useMemo(
		() => trackedBarcodes(live?.track ?? NO_CODES),
		[live?.track],
	);
	const count = codes.length;
	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) onCancel();
			}}
		>
			<DialogContent className="h-[calc(100dvh-2rem-var(--fl-safe-top,0px)-var(--fl-safe-bottom,0px))] gap-0 overflow-hidden p-0 sm:max-w-3xl">
				<DialogHeader className="px-4 pt-4 pb-3 pr-12">
					<DialogTitle>{t("scanBarcodes", "Scan barcodes")}</DialogTitle>
					<DialogDescription>
						{t(
							"barcodeScannerHint",
							"Point the camera at the codes. Every code that is read gets outlined; press the shutter to add the photo and the codes to your message.",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="relative min-h-0 flex-1 bg-muted">
					<video
						ref={videoRef}
						muted
						playsInline
						autoPlay
						className="absolute inset-0 h-full w-full object-contain"
					/>
					{live && (
						<svg
							aria-hidden="true"
							viewBox={`0 0 ${live.width} ${live.height}`}
							preserveAspectRatio="xMidYMid meet"
							className="pointer-events-none absolute inset-0 h-full w-full"
						>
							{codes.map((code, index) => (
								<CodeOutline
									// Two equal labels share format and text; the index tells them apart.
									key={`${code.format}:${code.text}:${index}`}
									code={code}
									fontSize={Math.max(live.width, live.height) / 40}
								/>
							))}
						</svg>
					)}
					{!ready && !error && (
						<Loader2 className="absolute top-1/2 left-1/2 size-6 -translate-x-1/2 -translate-y-1/2 animate-spin text-muted-foreground" />
					)}
					{error && (
						<p
							role="alert"
							className="absolute inset-x-4 top-4 rounded-lg bg-background/90 p-3 text-sm text-destructive"
						>
							{error}
						</p>
					)}
				</div>
				<div className="flex items-center justify-between gap-3 px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
					<output aria-live="polite" className="text-sm text-muted-foreground">
						{t("barcodesFoundVal", "Codes found: {{val}}", { val: count })}
					</output>
					<Button
						type="button"
						size="icon"
						aria-label={t("captureBarcodes", "Capture photo and codes")}
						disabled={!ready || capturing}
						onClick={() => void capture()}
						className="size-14 rounded-full"
					>
						{capturing ? (
							<Loader2 className="size-6 animate-spin" />
						) : (
							<Camera className="size-6" />
						)}
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
