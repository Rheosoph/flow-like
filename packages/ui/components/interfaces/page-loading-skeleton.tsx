"use client";

import { useEffect, useMemo, useState } from "react";
import { cn } from "../../lib";

/* ── Animated node graph ──────────────────────────────────────────── */

interface GraphNode {
	id: number;
	cx: number;
	cy: number;
	r: number;
	delay: number;
}

interface GraphEdge {
	from: number;
	to: number;
	delay: number;
}

function buildGraph(): { nodes: GraphNode[]; edges: GraphEdge[] } {
	const nodes: GraphNode[] = [
		{ id: 0, cx: 50, cy: 60, r: 6, delay: 0 },
		{ id: 1, cx: 130, cy: 35, r: 5, delay: 0.3 },
		{ id: 2, cx: 130, cy: 85, r: 5, delay: 0.5 },
		{ id: 3, cx: 210, cy: 60, r: 7, delay: 0.8 },
		{ id: 4, cx: 290, cy: 35, r: 5, delay: 1.1 },
		{ id: 5, cx: 290, cy: 85, r: 5, delay: 1.3 },
		{ id: 6, cx: 370, cy: 60, r: 6, delay: 1.6 },
	];
	const edges: GraphEdge[] = [
		{ from: 0, to: 1, delay: 0.15 },
		{ from: 0, to: 2, delay: 0.25 },
		{ from: 1, to: 3, delay: 0.55 },
		{ from: 2, to: 3, delay: 0.65 },
		{ from: 3, to: 4, delay: 0.95 },
		{ from: 3, to: 5, delay: 1.05 },
		{ from: 4, to: 6, delay: 1.35 },
		{ from: 5, to: 6, delay: 1.45 },
	];
	return { nodes, edges };
}

function NodeGraph() {
	const { nodes, edges } = useMemo(() => buildGraph(), []);

	return (
		<svg
			viewBox="0 0 420 120"
			className="w-full max-w-xs h-auto"
			fill="none"
			aria-hidden
		>
			{/* edges with traveling-dot animation */}
			{edges.map((e) => {
				const a = nodes[e.from];
				const b = nodes[e.to];
				const pathId = `e-${e.from}-${e.to}`;
				return (
					<g key={pathId}>
						<path
							id={pathId}
							d={`M${a.cx},${a.cy} C${(a.cx + b.cx) / 2},${a.cy} ${(a.cx + b.cx) / 2},${b.cy} ${b.cx},${b.cy}`}
							className="pls-edge"
							style={{ animationDelay: `${e.delay}s` }}
						/>
						<circle r="2.5" className="pls-particle">
							<animateMotion
								dur="2s"
								repeatCount="indefinite"
								begin={`${e.delay}s`}
							>
								<mpath href={`#${pathId}`} />
							</animateMotion>
						</circle>
					</g>
				);
			})}

			{/* nodes */}
			{nodes.map((n) => (
				<g key={n.id}>
					<circle
						cx={n.cx}
						cy={n.cy}
						r={n.r + 4}
						className="pls-ring"
						style={{ animationDelay: `${n.delay}s` }}
					/>
					<circle
						cx={n.cx}
						cy={n.cy}
						r={n.r}
						className="pls-node"
						style={{ animationDelay: `${n.delay}s` }}
					/>
				</g>
			))}
		</svg>
	);
}

/* ── Step labels ──────────────────────────────────────────────────── */

const STEPS = [
	"Initializing workflow",
	"Loading resources",
	"Processing data",
	"Preparing interface",
];

function StepIndicator() {
	const [step, setStep] = useState(0);

	useEffect(() => {
		const id = setInterval(() => {
			setStep((s) => (s + 1) % STEPS.length);
		}, 2200);
		return () => clearInterval(id);
	}, []);

	return (
		<div className="flex items-center gap-2">
			<div className="flex gap-1">
				{STEPS.map((_, i) => (
					<div
						key={i}
						className={cn(
							"h-1 rounded-full transition-all duration-500",
							i <= step ? "w-5 bg-primary/60" : "w-1.5 bg-muted-foreground/15",
						)}
					/>
				))}
			</div>
			<span className="text-xs text-muted-foreground/50 min-w-32.5 transition-all duration-300">
				{STEPS[step]}…
			</span>
		</div>
	);
}

/* ── Main component ───────────────────────────────────────────────── */

export function PageLoadingSkeleton({
	className,
	title = "Running workflow",
}: Readonly<{ className?: string; title?: string }>) {
	return (
		<div
			className={cn(
				"flex flex-col items-center justify-center h-full w-full gap-8 p-8",
				className,
			)}
		>
			{/* radial ambient glow */}
			<div className="pointer-events-none absolute inset-0 overflow-hidden">
				<div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 h-[50vh] w-[50vh] rounded-full bg-primary/3 blur-[120px] pls-breathe" />
			</div>

			{/* animated node graph */}
			<div className="relative pls-enter">
				<NodeGraph />
			</div>

			{/* status area */}
			<div
				className="flex flex-col items-center gap-3 pls-enter"
				style={{ animationDelay: "0.2s" }}
			>
				<p className="text-sm font-medium text-foreground/70">{title}</p>
				<StepIndicator />
			</div>

			<style>{`
				/* entry */
				.pls-enter {
					animation: pls-enter 0.7s ease-out both;
				}
				@keyframes pls-enter {
					from { opacity: 0; transform: translateY(12px) scale(0.97); }
					to   { opacity: 1; transform: translateY(0) scale(1); }
				}

				/* ambient glow breath */
				.pls-breathe {
					animation: pls-breathe 4s ease-in-out infinite;
				}
				@keyframes pls-breathe {
					0%, 100% { opacity: 0.6; transform: translate(-50%,-50%) scale(1); }
					50%      { opacity: 1;   transform: translate(-50%,-50%) scale(1.15); }
				}

				/* graph edges */
				.pls-edge {
					stroke: hsl(var(--primary) / 0.12);
					stroke-width: 1.5;
					stroke-dasharray: 200;
					stroke-dashoffset: 200;
					animation: pls-draw 1.2s ease-out forwards;
				}
				@keyframes pls-draw {
					to { stroke-dashoffset: 0; }
				}

				/* traveling particle */
				.pls-particle {
					fill: hsl(var(--primary) / 0.5);
				}

				/* graph nodes */
				.pls-node {
					fill: hsl(var(--primary) / 0.15);
					stroke: hsl(var(--primary) / 0.35);
					stroke-width: 1.5;
					animation: pls-pop 0.5s ease-out both;
				}
				@keyframes pls-pop {
					from { r: 0; opacity: 0; }
					to   { opacity: 1; }
				}

				/* outer ring pulse on nodes */
				.pls-ring {
					fill: none;
					stroke: hsl(var(--primary) / 0.08);
					stroke-width: 1;
					animation: pls-ring-pulse 2.5s ease-in-out infinite;
				}
				@keyframes pls-ring-pulse {
					0%, 100% { opacity: 0.4; r: inherit; }
					50%      { opacity: 0;   }
				}
			`}</style>
		</div>
	);
}
