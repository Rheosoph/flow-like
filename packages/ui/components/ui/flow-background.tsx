"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../lib";

interface FlowBackgroundProps {
	className?: string;
	intensity?: "subtle" | "medium" | "full";
	interactive?: boolean;
	/**
	 * Skips the animated canvas while keeping the wrapper DOM identical. Routes
	 * that paint an opaque editor over the whole window would otherwise run a
	 * permanent rAF loop and a window `mousemove` listener under it — and
	 * swapping the wrapper element instead would remount `{children}`, i.e. the
	 * entire router outlet.
	 */
	active?: boolean;
	children?: React.ReactNode;
}

interface Node {
	id: number;
	x: number;
	y: number;
	vx: number;
	vy: number;
	radius: number;
	pulse: number;
	color: string;
}

interface Connection {
	from: number;
	to: number;
	progress: number;
	speed: number;
	active: boolean;
}

const NODE_COLORS = [
	"rgba(59, 130, 246, 0.6)", // blue
	"rgba(168, 85, 247, 0.6)", // purple
	"rgba(236, 72, 153, 0.6)", // pink
	"rgba(34, 197, 94, 0.6)", // green
	"rgba(251, 146, 60, 0.6)", // orange
];

function seededRandom(seed: number) {
	const x = Math.sin(seed) * 10000;
	return x - Math.floor(x);
}

function FlowCanvas({
	intensity,
	interactive,
}: { intensity: "subtle" | "medium" | "full"; interactive: boolean }) {
	const { t } = useTranslation("common");
	const canvasRef = useRef<HTMLCanvasElement>(null);
	const containerRef = useRef<HTMLDivElement>(null);
	const animationRef = useRef<number>(0);
	const nodesRef = useRef<Node[]>([]);
	const connectionsRef = useRef<Connection[]>([]);
	const mouseRef = useRef({ x: -1000, y: -1000 });
	const [isReady, setIsReady] = useState(false);

	const opacityMultiplier =
		intensity === "subtle" ? 0.3 : intensity === "medium" ? 0.5 : 0.7;
	const nodeCountMultiplier =
		intensity === "subtle" ? 0.5 : intensity === "medium" ? 0.75 : 1;

	const initializeGraph = useCallback(
		(width: number, height: number) => {
			if (width <= 0 || height <= 0) return;

			const baseCount = Math.min(Math.floor((width * height) / 35000), 15);
			const nodeCount = Math.max(
				5,
				Math.floor(baseCount * nodeCountMultiplier),
			);
			const nodes: Node[] = [];

			for (let i = 0; i < nodeCount; i++) {
				nodes.push({
					id: i,
					x: seededRandom(i * 7 + 1) * width,
					y: seededRandom(i * 13 + 2) * height,
					vx: (seededRandom(i * 17 + 3) - 0.5) * 0.3,
					vy: (seededRandom(i * 23 + 4) - 0.5) * 0.3,
					radius: 3 + seededRandom(i * 29 + 5) * 3,
					pulse: seededRandom(i * 31 + 6) * Math.PI * 2,
					color: NODE_COLORS[i % NODE_COLORS.length],
				});
			}

			const connections: Connection[] = [];
			for (let i = 0; i < nodes.length; i++) {
				const connectionCount = 1 + Math.floor(seededRandom(i * 37 + 7) * 2);
				for (let j = 0; j < connectionCount; j++) {
					const target = Math.floor(
						seededRandom(i * 41 + j * 43 + 8) * nodes.length,
					);
					if (target !== i) {
						connections.push({
							from: i,
							to: target,
							progress: seededRandom(i * 47 + j * 53 + 9),
							speed: 0.001 + seededRandom(i * 59 + j * 61 + 10) * 0.002,
							active: seededRandom(i * 67 + j * 71 + 11) > 0.4,
						});
					}
				}
			}

			nodesRef.current = nodes;
			connectionsRef.current = connections;
			setIsReady(true);
		},
		[nodeCountMultiplier],
	);

	useEffect(() => {
		const canvas = canvasRef.current;
		const container = containerRef.current;
		if (!canvas || !container) return;

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		let isSetup = false;

		const setupCanvas = () => {
			const rect = container.getBoundingClientRect();
			// Use window dimensions as fallback if container hasn't sized yet
			const width = rect.width > 0 ? rect.width : window.innerWidth;
			const height = rect.height > 0 ? rect.height : window.innerHeight;

			if (width <= 0 || height <= 0) return;

			const dpr = window.devicePixelRatio || 1;
			canvas.width = width * dpr;
			canvas.height = height * dpr;
			canvas.style.width = `${width}px`;
			canvas.style.height = `${height}px`;
			ctx.setTransform(1, 0, 0, 1, 0, 0); // Reset transform before scaling
			ctx.scale(dpr, dpr);
			initializeGraph(width, height);
			isSetup = true;
		};

		// Try setup immediately, then retry after a frame if needed
		setupCanvas();
		const initTimeout = requestAnimationFrame(() => {
			if (!isSetup) setupCanvas();
		});
		// Additional retry after a short delay for slow renders
		const retryTimeout = setTimeout(() => {
			if (!isSetup) setupCanvas();
		}, 100);

		const handleResize = () => {
			setupCanvas();
		};

		const handleMouseMove = (e: MouseEvent) => {
			if (interactive) {
				mouseRef.current = { x: e.clientX, y: e.clientY };
			}
		};

		window.addEventListener("resize", handleResize);
		if (interactive) {
			window.addEventListener("mousemove", handleMouseMove);
		}

		const animate = () => {
			const rect = container.getBoundingClientRect();
			const width = rect.width || window.innerWidth;
			const height = rect.height || window.innerHeight;
			const nodes = nodesRef.current;
			const connections = connectionsRef.current;
			const mouse = mouseRef.current;

			if (nodes.length === 0) {
				animationRef.current = requestAnimationFrame(animate);
				return;
			}

			ctx.clearRect(0, 0, width, height);

			// Draw connections
			for (const conn of connections) {
				const fromNode = nodes[conn.from];
				const toNode = nodes[conn.to];
				if (!fromNode || !toNode) continue;

				const dx = toNode.x - fromNode.x;
				const dy = toNode.y - fromNode.y;
				const dist = Math.sqrt(dx * dx + dy * dy);

				if (dist > 350) continue;

				const alpha = Math.max(0, 1 - dist / 350) * 0.1 * opacityMultiplier;
				ctx.beginPath();
				ctx.moveTo(fromNode.x, fromNode.y);
				ctx.lineTo(toNode.x, toNode.y);
				ctx.strokeStyle = `rgba(148, 163, 184, ${alpha})`;
				ctx.lineWidth = 1;
				ctx.stroke();

				if (conn.active) {
					conn.progress += conn.speed;
					if (conn.progress > 1) conn.progress = 0;

					const px = fromNode.x + dx * conn.progress;
					const py = fromNode.y + dy * conn.progress;

					const gradient = ctx.createRadialGradient(px, py, 0, px, py, 4);
					gradient.addColorStop(
						0,
						`rgba(59, 130, 246, ${0.6 * opacityMultiplier})`,
					);
					gradient.addColorStop(1, "rgba(59, 130, 246, 0)");

					ctx.beginPath();
					ctx.arc(px, py, 4, 0, Math.PI * 2);
					ctx.fillStyle = gradient;
					ctx.fill();
				}
			}

			// Draw and update nodes
			for (const node of nodes) {
				if (interactive) {
					const mdx = mouse.x - node.x;
					const mdy = mouse.y - node.y;
					const mouseDist = Math.sqrt(mdx * mdx + mdy * mdy);
					if (mouseDist < 120 && mouseDist > 0) {
						const force = (120 - mouseDist) / 120;
						node.vx -= (mdx / mouseDist) * force * 0.05;
						node.vy -= (mdy / mouseDist) * force * 0.05;
					}
				}

				node.x += node.vx;
				node.y += node.vy;
				node.vx *= 0.995;
				node.vy *= 0.995;
				node.pulse += 0.015;

				if (node.x < 0 || node.x > width) node.vx *= -1;
				if (node.y < 0 || node.y > height) node.vy *= -1;
				node.x = Math.max(0, Math.min(width, node.x));
				node.y = Math.max(0, Math.min(height, node.y));

				const pulseScale = 1 + Math.sin(node.pulse) * 0.15;
				const glowRadius = node.radius * 2.5 * pulseScale;

				const glow = ctx.createRadialGradient(
					node.x,
					node.y,
					0,
					node.x,
					node.y,
					glowRadius,
				);
				glow.addColorStop(
					0,
					node.color.replace("0.6", String(0.2 * opacityMultiplier)),
				);
				glow.addColorStop(1, "transparent");

				ctx.beginPath();
				ctx.arc(node.x, node.y, glowRadius, 0, Math.PI * 2);
				ctx.fillStyle = glow;
				ctx.fill();

				ctx.beginPath();
				ctx.arc(node.x, node.y, node.radius * pulseScale, 0, Math.PI * 2);
				ctx.fillStyle = node.color.replace(
					"0.6",
					String(0.4 * opacityMultiplier),
				);
				ctx.fill();
			}

			animationRef.current = requestAnimationFrame(animate);
		};

		animate();

		return () => {
			cancelAnimationFrame(initTimeout);
			clearTimeout(retryTimeout);
			window.removeEventListener("resize", handleResize);
			if (interactive) {
				window.removeEventListener("mousemove", handleMouseMove);
			}
			cancelAnimationFrame(animationRef.current);
		};
	}, [initializeGraph, interactive, opacityMultiplier]);

	return (
		<div ref={containerRef} className="absolute inset-0">
			<canvas
				ref={canvasRef}
				className="absolute inset-0 pointer-events-none"
				style={{ opacity: isReady ? 1 : 0, transition: "opacity 0.3s ease-in" }}
				aria-hidden="true"
			/>
		</div>
	);
}

export function FlowBackground({
	className,
	intensity = "subtle",
	interactive = true,
	active = true,
	children,
}: Readonly<FlowBackgroundProps>) {
	return (
		<div className={cn("relative w-full h-full", className)}>
			{/* Gradient overlays */}
			<div className="absolute inset-0 bg-gradient-to-br from-background via-background to-background pointer-events-none">
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_left,rgba(59,130,246,0.08),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,rgba(168,85,247,0.08),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,rgba(236,72,153,0.04),transparent_60%)]" />
			</div>

			{/* Flow graph canvas */}
			{active && <FlowCanvas intensity={intensity} interactive={interactive} />}

			{/* Subtle grid */}
			<div
				className="absolute inset-0 pointer-events-none opacity-[0.02]"
				style={{
					backgroundImage: `linear-gradient(rgba(255,255,255,0.1) 1px, transparent 1px),
                                    linear-gradient(90deg, rgba(255,255,255,0.1) 1px, transparent 1px)`,
					backgroundSize: "80px 80px",
				}}
				aria-hidden="true"
			/>

			{/* Content - flex layout to allow proper scrolling inside children */}
			{children && (
				<div className="relative z-10 w-full h-full flex flex-col flex-1 min-h-0 overflow-auto">
					{children}
				</div>
			)}
		</div>
	);
}

export function FlowGradientOverlay({
	className,
	variant = "default",
}: Readonly<{ className?: string; variant?: "default" | "accent" | "muted" }>) {
	const gradients = {
		default: (
			<>
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top_left,rgba(59,130,246,0.1),transparent_50%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom_right,rgba(168,85,247,0.1),transparent_50%)]" />
			</>
		),
		accent: (
			<>
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,rgba(236,72,153,0.12),transparent_60%)]" />
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_bottom,rgba(59,130,246,0.12),transparent_60%)]" />
			</>
		),
		muted: (
			<>
				<div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,rgba(148,163,184,0.06),transparent_70%)]" />
			</>
		),
	};

	return (
		<div
			className={cn("absolute inset-0 pointer-events-none", className)}
			aria-hidden="true"
		>
			{gradients[variant]}
		</div>
	);
}

interface FloatingOrbsProps {
	className?: string;
	count?: number;
	colors?: string[];
}

export function FloatingOrbs({
	className,
	count = 5,
	colors = [
		"bg-blue-500/20",
		"bg-purple-500/20",
		"bg-pink-500/20",
		"bg-green-500/20",
		"bg-orange-500/20",
	],
}: FloatingOrbsProps) {
	const orbs = Array.from({ length: count }, (_, i) => {
		const size = 40 + seededRandom(i * 7) * 60;
		const x = seededRandom(i * 11) * 100;
		const y = seededRandom(i * 13) * 100;
		const duration = 15 + seededRandom(i * 17) * 10;
		const delay = seededRandom(i * 19) * -20;

		return {
			id: i,
			size,
			x,
			y,
			duration,
			delay,
			color: colors[i % colors.length],
		};
	});

	return (
		<div
			className={cn(
				"absolute inset-0 overflow-hidden pointer-events-none",
				className,
			)}
			aria-hidden="true"
		>
			{orbs.map((orb) => (
				<div
					key={orb.id}
					className={cn("absolute rounded-full blur-2xl", orb.color)}
					style={{
						width: orb.size,
						height: orb.size,
						left: `${orb.x}%`,
						top: `${orb.y}%`,
						animation: `float-orb ${orb.duration}s ease-in-out infinite`,
						animationDelay: `${orb.delay}s`,
						transform: "translate(-50%, -50%)",
					}}
				/>
			))}
		</div>
	);
}
