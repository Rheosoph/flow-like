"use client";
import { motion } from "framer-motion";

interface UniversityHeroArtProps {
	readonly className?: string;
}

const NODES = [
	{ x: 30, y: 130, r: 3.5 },
	{ x: 80, y: 60, r: 4.5 },
	{ x: 110, y: 160, r: 3.5 },
	{ x: 170, y: 100, r: 4 },
	{ x: 225, y: 45, r: 5 },
] as const;

const LINKS = [
	[0, 1],
	[0, 2],
	[1, 3],
	[2, 3],
	[3, 4],
] as const;

// hop sequence for the traveling pulse — picks a meaningful route through the graph
const TRAVELER = [0, 1, 3, 4, 3, 2, 0] as const;

export function UniversityHeroArt({ className = "" }: UniversityHeroArtProps) {
	return (
		<div
			className={`pointer-events-none select-none text-amber-300 ${className}`}
			aria-hidden
		>
			<svg
				viewBox="0 0 250 200"
				className="h-full w-full"
				role="presentation"
			>
				<defs>
					<radialGradient id="hero-node-glow" cx="50%" cy="50%" r="50%">
						<stop offset="0%" stopColor="currentColor" stopOpacity="0.7" />
						<stop offset="55%" stopColor="currentColor" stopOpacity="0.15" />
						<stop offset="100%" stopColor="currentColor" stopOpacity="0" />
					</radialGradient>
					<linearGradient id="hero-link-grad" x1="0" y1="0" x2="1" y2="1">
						<stop offset="0%" stopColor="rgb(56 189 248)" stopOpacity="0.7" />
						<stop offset="55%" stopColor="rgb(167 139 250)" stopOpacity="0.6" />
						<stop offset="100%" stopColor="rgb(251 191 36)" stopOpacity="0.6" />
					</linearGradient>
					<radialGradient id="hero-traveler" cx="50%" cy="50%" r="50%">
						<stop offset="0%" stopColor="white" stopOpacity="1" />
						<stop offset="60%" stopColor="white" stopOpacity="0.4" />
						<stop offset="100%" stopColor="white" stopOpacity="0" />
					</radialGradient>
				</defs>

				{/* connecting lines — draw, hold, fade, restart with stagger */}
				{LINKS.map(([a, b], i) => {
					const from = NODES[a];
					const to = NODES[b];
					if (!from || !to) return null;
					return (
						<motion.line
							key={`${a}-${b}`}
							x1={from.x}
							y1={from.y}
							x2={to.x}
							y2={to.y}
							stroke="url(#hero-link-grad)"
							strokeWidth={1}
							strokeLinecap="round"
							initial={{ pathLength: 0, opacity: 0 }}
							animate={{
								pathLength: [0, 1, 1, 0],
								opacity: [0, 0.75, 0.75, 0],
							}}
							transition={{
								duration: 7,
								delay: i * 0.45,
								repeat: Number.POSITIVE_INFINITY,
								ease: "easeInOut",
								times: [0, 0.3, 0.75, 1],
							}}
						/>
					);
				})}

				{/* nodes — soft halo + solid dot, twinkle with staggered delays */}
				{NODES.map((n, i) => (
					<g key={`node-${n.x}-${n.y}`}>
						<motion.circle
							cx={n.x}
							cy={n.y}
							r={n.r * 4}
							fill="url(#hero-node-glow)"
							style={{ transformOrigin: `${n.x}px ${n.y}px` }}
							animate={{
								opacity: [0.25, 0.65, 0.25],
								scale: [0.9, 1.15, 0.9],
							}}
							transition={{
								duration: 3.5 + i * 0.6,
								delay: i * 0.4,
								repeat: Number.POSITIVE_INFINITY,
								ease: "easeInOut",
							}}
						/>
						<circle
							cx={n.x}
							cy={n.y}
							r={n.r}
							fill="currentColor"
							opacity={0.85}
						/>
					</g>
				))}

				{/* traveling pulse — hops through the network */}
				<motion.circle
					r={3}
					fill="url(#hero-traveler)"
					initial={{ opacity: 0 }}
					animate={{
						opacity: [0, 1, 1, 1, 1, 1, 1, 0],
						cx: TRAVELER.map((idx) => NODES[idx]?.x ?? 0),
						cy: TRAVELER.map((idx) => NODES[idx]?.y ?? 0),
					}}
					transition={{
						duration: 9,
						repeat: Number.POSITIVE_INFINITY,
						ease: "easeInOut",
					}}
				/>
			</svg>
		</div>
	);
}
