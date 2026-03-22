"use client";

import { Phone, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../../ui/button";

interface VoiceModeProps {
	open: boolean;
	onClose: () => void;
	onSend: (audioFile: File) => void;
}

// Orb visualizer responding to audio amplitude
function VoiceOrb({
	analyser,
	state,
}: {
	analyser: AnalyserNode | null;
	state: "listening" | "processing" | "idle";
}) {
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const animRef = useRef<number>(0);
	const phaseRef = useRef(0);

	useEffect(() => {
		const canvas = canvasRef.current;
		if (!canvas) return;

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		const dpr = window.devicePixelRatio || 1;
		canvas.width = 280 * dpr;
		canvas.height = 280 * dpr;
		ctx.scale(dpr, dpr);

		const draw = () => {
			animRef.current = requestAnimationFrame(draw);
			ctx.clearRect(0, 0, 280, 280);

			const cx = 140;
			const cy = 140;
			phaseRef.current += 0.02;

			let amplitude = 0;
			if (analyser && state === "listening") {
				const dataArray = new Uint8Array(analyser.frequencyBinCount);
				analyser.getByteTimeDomainData(dataArray);
				let sum = 0;
				for (let i = 0; i < dataArray.length; i++) {
					const v = (dataArray[i] - 128) / 128;
					sum += v * v;
				}
				amplitude = Math.sqrt(sum / dataArray.length);
			}

			const baseRadius = 60;
			const maxBulge = 35;
			const scaledAmplitude = Math.min(amplitude * 8, 1);

			// Multiple layered rings
			const layers = [
				{ radius: baseRadius + maxBulge * scaledAmplitude, alpha: 0.08, blur: 40 },
				{ radius: baseRadius + maxBulge * scaledAmplitude * 0.7, alpha: 0.12, blur: 25 },
				{ radius: baseRadius + maxBulge * scaledAmplitude * 0.4, alpha: 0.2, blur: 12 },
			];

			for (const layer of layers) {
				const gradient = ctx.createRadialGradient(
					cx,
					cy,
					0,
					cx,
					cy,
					layer.radius + layer.blur,
				);
				gradient.addColorStop(0, `rgba(139, 92, 246, ${layer.alpha})`);
				gradient.addColorStop(0.5, `rgba(59, 130, 246, ${layer.alpha * 0.7})`);
				gradient.addColorStop(1, "rgba(139, 92, 246, 0)");
				ctx.fillStyle = gradient;
				ctx.beginPath();
				ctx.arc(cx, cy, layer.radius + layer.blur, 0, Math.PI * 2);
				ctx.fill();
			}

			// Main orb with deformable edge
			const points = 128;
			ctx.beginPath();
			for (let i = 0; i <= points; i++) {
				const angle = (i / points) * Math.PI * 2;
				const wave1 = Math.sin(angle * 3 + phaseRef.current * 2) * maxBulge * scaledAmplitude * 0.3;
				const wave2 = Math.sin(angle * 5 - phaseRef.current * 1.5) * maxBulge * scaledAmplitude * 0.2;
				const wave3 = Math.sin(angle * 7 + phaseRef.current * 3) * maxBulge * scaledAmplitude * 0.1;
				const r = baseRadius + maxBulge * scaledAmplitude * 0.5 + wave1 + wave2 + wave3;

				const x = cx + Math.cos(angle) * r;
				const y = cy + Math.sin(angle) * r;
				if (i === 0) ctx.moveTo(x, y);
				else ctx.lineTo(x, y);
			}
			ctx.closePath();

			const orbGradient = ctx.createRadialGradient(cx - 20, cy - 20, 0, cx, cy, baseRadius + maxBulge);
			orbGradient.addColorStop(0, "rgba(167, 139, 250, 0.95)");
			orbGradient.addColorStop(0.4, "rgba(99, 102, 241, 0.85)");
			orbGradient.addColorStop(0.7, "rgba(59, 130, 246, 0.8)");
			orbGradient.addColorStop(1, "rgba(139, 92, 246, 0.75)");
			ctx.fillStyle = orbGradient;
			ctx.fill();

			// Inner highlight
			const innerGlow = ctx.createRadialGradient(cx - 15, cy - 15, 0, cx, cy, baseRadius * 0.6);
			innerGlow.addColorStop(0, "rgba(255, 255, 255, 0.25)");
			innerGlow.addColorStop(1, "rgba(255, 255, 255, 0)");
			ctx.fillStyle = innerGlow;
			ctx.beginPath();
			ctx.arc(cx, cy, baseRadius * 0.6, 0, Math.PI * 2);
			ctx.fill();

			// Processing spinner overlay
			if (state === "processing") {
				ctx.strokeStyle = "rgba(255, 255, 255, 0.4)";
				ctx.lineWidth = 3;
				ctx.lineCap = "round";
				const spinAngle = phaseRef.current * 4;
				ctx.beginPath();
				ctx.arc(cx, cy, baseRadius + 8, spinAngle, spinAngle + Math.PI * 1.2);
				ctx.stroke();
			}
		};

		draw();
		return () => cancelAnimationFrame(animRef.current);
	}, [analyser, state]);

	return (
		<canvas
			ref={canvasRef}
			width={280}
			height={280}
			className="w-70 h-70"
			style={{ imageRendering: "auto" }}
		/>
	);
}

export function VoiceMode({ open, onClose, onSend }: VoiceModeProps) {
	const [state, setState] = useState<"idle" | "listening" | "processing">("idle");
	const [duration, setDuration] = useState(0);
	const [analyserNode, setAnalyserNode] = useState<AnalyserNode | null>(null);

	const mediaRecorderRef = useRef<MediaRecorder | null>(null);
	const audioChunksRef = useRef<Blob[]>([]);
	const audioContextRef = useRef<AudioContext | null>(null);
	const analyserRef = useRef<AnalyserNode | null>(null);
	const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const silenceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const streamRef = useRef<MediaStream | null>(null);

	const SILENCE_THRESHOLD = 0.008;
	const SILENCE_MS = 2000;

	const cleanup = useCallback(() => {
		if (timerRef.current) {
			clearInterval(timerRef.current);
			timerRef.current = null;
		}
		if (silenceTimerRef.current) {
			clearTimeout(silenceTimerRef.current);
			silenceTimerRef.current = null;
		}
		if (streamRef.current) {
			streamRef.current.getTracks().forEach((t) => t.stop());
			streamRef.current = null;
		}
		if (audioContextRef.current) {
			audioContextRef.current.close().catch(() => {});
			audioContextRef.current = null;
		}
		setAnalyserNode(null);
		analyserRef.current = null;
	}, []);

	const handleClose = useCallback(() => {
		if (mediaRecorderRef.current?.state === "recording") {
			mediaRecorderRef.current.stop();
		}
		cleanup();
		setState("idle");
		setDuration(0);
		onClose();
	}, [cleanup, onClose]);

	const startListening = useCallback(async () => {
		try {
			const stream = await navigator.mediaDevices.getUserMedia({
				audio: true,
			});
			streamRef.current = stream;

			const audioContext = new AudioContext();
			const source = audioContext.createMediaStreamSource(stream);
			const analyser = audioContext.createAnalyser();
			analyser.fftSize = 2048;
			source.connect(analyser);
			audioContextRef.current = audioContext;
			analyserRef.current = analyser;
			setAnalyserNode(analyser);

			const mediaRecorder = new MediaRecorder(stream);
			mediaRecorderRef.current = mediaRecorder;
			audioChunksRef.current = [];

			mediaRecorder.ondataavailable = (event) => {
				if (event.data.size > 0) {
					audioChunksRef.current.push(event.data);
				}
			};

			mediaRecorder.onstop = () => {
				const audioBlob = new Blob(audioChunksRef.current, {
					type: "audio/webm",
				});

				if (audioBlob.size > 0) {
					setState("processing");
					const audioFile = new File(
						[audioBlob],
						`voice-mode-${Date.now()}.webm`,
						{ type: "audio/webm" },
					);
					onSend(audioFile);
				}

				cleanup();
				setState("idle");
				setDuration(0);
			};

			mediaRecorder.start();
			setState("listening");
			setDuration(0);

			timerRef.current = setInterval(() => {
				setDuration((prev) => prev + 1);
			}, 1000);

			// Silence detection
			let silentSince: number | null = null;
			const dataArray = new Float32Array(analyser.fftSize);

			const checkSilence = () => {
				if (
					!mediaRecorderRef.current ||
					mediaRecorderRef.current.state !== "recording"
				) {
					return;
				}

				analyser.getFloatTimeDomainData(dataArray);
				let sum = 0;
				for (let i = 0; i < dataArray.length; i++) {
					sum += dataArray[i] * dataArray[i];
				}
				const rms = Math.sqrt(sum / dataArray.length);

				if (rms < SILENCE_THRESHOLD) {
					if (!silentSince) silentSince = Date.now();
					if (Date.now() - silentSince > SILENCE_MS) {
						// Auto-stop on silence
						if (
							mediaRecorderRef.current?.state === "recording"
						) {
							mediaRecorderRef.current.stop();
						}
						return;
					}
				} else {
					silentSince = null;
				}

				silenceTimerRef.current = setTimeout(checkSilence, 80);
			};

			// Wait a moment before starting silence detection to allow initial silence
			setTimeout(checkSilence, 1500);
		} catch (err) {
			console.error("Voice mode: microphone access denied", err);
			handleClose();
		}
	}, [cleanup, handleClose, onSend]);

	// Auto-start when opened
	useEffect(() => {
		if (open && state === "idle") {
			startListening();
		}
	}, [open]);

	// Cleanup on unmount
	useEffect(() => {
		return () => {
			cleanup();
		};
	}, [cleanup]);

	// Keyboard shortcut to close
	useEffect(() => {
		if (!open) return;
		const handleKey = (e: KeyboardEvent) => {
			if (e.key === "Escape") handleClose();
		};
		window.addEventListener("keydown", handleKey);
		return () => window.removeEventListener("keydown", handleKey);
	}, [open, handleClose]);

	if (!open) return null;

	const formatTime = (seconds: number) => {
		const mins = Math.floor(seconds / 60);
		const secs = seconds % 60;
		return `${mins}:${secs.toString().padStart(2, "0")}`;
	};

	return (
		<div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-xl animate-in fade-in duration-300">
			{/* Radial gradient backdrop */}
			<div className="absolute inset-0 bg-gradient-radial from-violet-500/5 via-transparent to-transparent" />

			{/* Close button */}
			<Button
				variant="ghost"
				size="sm"
				className="absolute top-6 right-6 h-10 w-10 rounded-full"
				onClick={handleClose}
			>
				<X className="w-5 h-5" />
			</Button>

			{/* Center content */}
			<div className="flex flex-col items-center gap-8 relative">
				{/* Orb */}
				<VoiceOrb analyser={analyserNode} state={state} />

				{/* Status text */}
				<div className="flex flex-col items-center gap-2">
					{state === "listening" && (
						<>
							<p className="text-lg font-medium text-foreground">
								Listening...
							</p>
							<p className="text-sm text-muted-foreground font-mono">
								{formatTime(duration)}
							</p>
							<p className="text-xs text-muted-foreground/60 mt-2">
								Will auto-send when you stop talking
							</p>
						</>
					)}
					{state === "processing" && (
						<p className="text-lg font-medium text-foreground animate-pulse">
							Sending...
						</p>
					)}
					{state === "idle" && (
						<p className="text-sm text-muted-foreground">
							Initializing microphone...
						</p>
					)}
				</div>

				{/* End call button */}
				{state === "listening" && (
					<Button
						variant="destructive"
						size="lg"
						className="rounded-full h-14 w-14 shadow-lg shadow-red-500/20"
						onClick={handleClose}
					>
						<Phone className="w-5 h-5 rotate-135" />
					</Button>
				)}
			</div>
		</div>
	);
}
