"use client";

import { useEffect, useMemo, useState } from "react";
import { cn } from "../../lib";

interface LoadingScreenProps {
	message?: string;
	progress?: number;
	className?: string;
}

interface Tip {
	emoji: string;
	text: string;
}

const TIPS: Tip[] = [
	{
		emoji: "⌨️",
		text: "Press Ctrl+K to open the command palette from anywhere.",
	},
	{ emoji: "🔗", text: "Connect nodes by dragging from one pin to another." },
	{
		emoji: "📦",
		text: "Browse community packages in the Registry to extend your workflows.",
	},
	{ emoji: "💾", text: "Your flows auto-save — no need to hit save manually." },
	{
		emoji: "🔍",
		text: "Use the search bar in the node catalog to find nodes quickly.",
	},
	{
		emoji: "🎯",
		text: "Double-click the canvas to create a new node at that position.",
	},
	{
		emoji: "📋",
		text: "Select multiple nodes with Shift+Click to move them together.",
	},
	{
		emoji: "🧪",
		text: "Test individual nodes by right-clicking and selecting 'Run'.",
	},
	{ emoji: "🌙", text: "Toggle dark mode in Settings → Appearance." },
	{ emoji: "🔄", text: "Undo with Ctrl+Z — works for node connections too." },
	{ emoji: "📊", text: "Use the Data Viewer node to inspect values mid-flow." },
	{
		emoji: "⚡",
		text: "Pin frequently used nodes to the toolbar for faster access.",
	},
];

const HINTS: Tip[] = [
	{
		emoji: "🖥️",
		text: "Try Flow Like Studio — the desktop app for offline editing and local execution.",
	},
	{
		emoji: "🤖",
		text: "FlowPilot can generate entire workflows from a text description.",
	},
	{
		emoji: "🧩",
		text: "Build custom nodes with WASM — use any language that compiles to WebAssembly.",
	},
	{
		emoji: "☁️",
		text: "Deploy flows to the cloud with one click from the Studio.",
	},
	{
		emoji: "📱",
		text: "Flow Like Studio syncs your projects across all your devices.",
	},
	{
		emoji: "🔐",
		text: "Studio supports local-only mode — your data never leaves your machine.",
	},
];

function pickRandom<T>(arr: readonly T[], exclude?: number): number {
	let idx: number;
	do {
		idx = Math.floor(Math.random() * arr.length);
	} while (idx === exclude && arr.length > 1);
	return idx;
}

function FlowLogo() {
	return (
		<div className="relative flex items-center justify-center h-20 w-20">
			{/* outer breathing ring */}
			<div className="absolute inset-0 rounded-full border border-primary/12 ls-breathe" />

			{/* orbital dot */}
			<div className="absolute inset-0 ls-orbit">
				<div className="absolute top-0 left-1/2 -translate-x-1/2 -translate-y-1/2 h-1.5 w-1.5 rounded-full bg-primary/60" />
			</div>

			{/* glass container */}
			<div className="relative h-12 w-12 rounded-xl bg-muted/40 border border-border/60 backdrop-blur-sm flex items-center justify-center shadow-sm">
				<svg className="h-6 w-6 text-primary" viewBox="0 0 24 24" fill="none">
					<path
						d="M5 8h3a4 4 0 0 1 4 4v0a4 4 0 0 0 4 4h3"
						stroke="currentColor"
						strokeWidth="1.5"
						strokeLinecap="round"
						className="ls-draw"
					/>
					<path
						d="M5 16h3a4 4 0 0 0 4-4v0a4 4 0 0 1 4-4h3"
						stroke="currentColor"
						strokeWidth="1.5"
						strokeLinecap="round"
						className="ls-draw ls-draw--reverse"
					/>
					<circle cx="5" cy="8" r="1.5" fill="currentColor" opacity="0.5" />
					<circle cx="5" cy="16" r="1.5" fill="currentColor" opacity="0.5" />
					<circle
						cx="12"
						cy="12"
						r="2"
						fill="currentColor"
						className="ls-center-dot"
					/>
					<circle cx="19" cy="8" r="1.5" fill="currentColor" opacity="0.5" />
					<circle cx="19" cy="16" r="1.5" fill="currentColor" opacity="0.5" />
				</svg>
			</div>
		</div>
	);
}

function isHint(tip: Tip): boolean {
	return HINTS.some((h) => h.text === tip.text);
}

function TipCard({ tip, transitioning }: { tip: Tip; transitioning: boolean }) {
	const hint = isHint(tip);

	return (
		<div
			className={cn(
				"relative max-w-90 w-full rounded-xl border px-5 py-4 transition-all duration-500",
				hint
					? "border-primary/20 bg-primary/3"
					: "border-border/40 bg-muted/20",
				transitioning
					? "opacity-0 translate-y-3 scale-[0.98]"
					: "opacity-100 translate-y-0 scale-100",
			)}
		>
			{/* label */}
			<span
				className={cn(
					"text-[10px] font-medium uppercase tracking-wider mb-2 block",
					hint ? "text-primary/60" : "text-muted-foreground/40",
				)}
			>
				{hint ? "Did you know?" : "Tip"}
			</span>

			{/* body */}
			<div className="flex items-start gap-3">
				<span className="text-lg leading-none shrink-0" aria-hidden>
					{tip.emoji}
				</span>
				<p className="text-[13px] text-foreground/70 leading-relaxed">
					{tip.text}
				</p>
			</div>
		</div>
	);
}

export function LoadingScreen({
	message,
	progress = 0,
	className,
}: Readonly<LoadingScreenProps>) {
	const clamped = Math.min(Math.max(progress, 0), 100);

	const allCards = useMemo(() => [...TIPS, ...HINTS], []);
	const [cardIndex, setCardIndex] = useState(() => pickRandom(allCards));
	const [transitioning, setTransitioning] = useState(false);

	useEffect(() => {
		const id = setInterval(() => {
			setTransitioning(true);
			setTimeout(() => {
				setCardIndex((prev) => pickRandom(allCards, prev));
				setTransitioning(false);
			}, 500);
		}, 5500);
		return () => clearInterval(id);
	}, [allCards]);

	const currentCard = allCards[cardIndex];

	return (
		<div
			className={cn(
				"fixed inset-0 z-50 bg-background flex flex-col items-center justify-center overflow-hidden",
				className,
			)}
		>
			{/* soft radial glow */}
			<div className="pointer-events-none absolute inset-0">
				<div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 h-[60vh] w-[60vh] rounded-full bg-primary/3 blur-[160px]" />
			</div>

			{/* content */}
			<div className="relative flex flex-col items-center gap-10 ls-enter">
				{/* flow logo */}
				<FlowLogo />

				{/* status text */}
				<div className="text-center space-y-1.5">
					{message ? (
						<p className="text-sm text-foreground/70">{message}</p>
					) : (
						<p className="text-sm text-muted-foreground/60">
							Loading your workspace
						</p>
					)}
					{clamped > 0 && (
						<p className="text-xs tabular-nums text-muted-foreground/40">
							{Math.round(clamped)}%
						</p>
					)}
				</div>

				{/* progress bar */}
				{clamped > 0 && (
					<div className="w-48 h-px bg-border/40 rounded-full overflow-hidden">
						<div
							className="h-full bg-primary/50 transition-[width] duration-700 ease-out"
							style={{ width: `${clamped}%` }}
						/>
					</div>
				)}

				{/* tip / hint card */}
				<TipCard tip={currentCard} transitioning={transitioning} />
			</div>

			{/* minimal footer label */}
			<div
				className="absolute bottom-5 flex flex-col items-center gap-1 ls-enter"
				style={{ animationDelay: "0.3s" }}
			>
				<span className="text-[10px] tracking-widest uppercase text-muted-foreground/30">
					Flow Like
				</span>
			</div>

			<style>{`
				.ls-enter {
					animation: ls-enter 0.8s ease-out both;
				}
				@keyframes ls-enter {
					from { opacity: 0; transform: translateY(10px); }
					to   { opacity: 1; transform: translateY(0); }
				}

				/* breathing outer ring */
				.ls-breathe {
					animation: ls-breathe 4s ease-in-out infinite;
				}
				@keyframes ls-breathe {
					0%, 100% { transform: scale(1); opacity: 0.35; }
					50%      { transform: scale(1.12); opacity: 0.12; }
				}

				/* orbiting dot */
				.ls-orbit {
					animation: ls-orbit 6s linear infinite;
				}
				@keyframes ls-orbit {
					from { transform: rotate(0deg); }
					to   { transform: rotate(360deg); }
				}

				/* svg path draw */
				.ls-draw {
					stroke-dasharray: 48;
					stroke-dashoffset: 48;
					animation: ls-draw 2.8s ease-in-out infinite;
				}
				.ls-draw--reverse {
					animation-direction: reverse;
				}
				@keyframes ls-draw {
					0%   { stroke-dashoffset: 48; }
					50%  { stroke-dashoffset: 0; }
					100% { stroke-dashoffset: -48; }
				}

				/* center dot pulse */
				.ls-center-dot {
					transform-origin: center;
					animation: ls-center 2.8s ease-in-out infinite;
				}
				@keyframes ls-center {
					0%, 100% { transform: scale(1); opacity: 0.6; }
					50%      { transform: scale(1.3); opacity: 1; }
				}
			`}</style>
		</div>
	);
}
