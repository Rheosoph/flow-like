"use client";
import { motion } from "framer-motion";

/**
 * Decorative artwork for the University catalog hero. Pure SVG + framer-motion,
 * no image asset required. Floating layered "course cards" with sparkles and
 * orbiting orbs that ambient-animate forever.
 */
export function CatalogHeroArt({
	className = "",
}: {
	readonly className?: string;
}) {
	return (
		<div
			className={`relative pointer-events-none select-none ${className}`}
			aria-hidden
		>
			<svg viewBox="0 0 320 240" className="w-full h-full">
				<defs>
					<linearGradient id="card-violet" x1="0" y1="0" x2="1" y2="1">
						<stop offset="0%" stopColor="#a855f7" stopOpacity="0.85" />
						<stop offset="100%" stopColor="#ec4899" stopOpacity="0.6" />
					</linearGradient>
					<linearGradient id="card-cyan" x1="0" y1="0" x2="1" y2="1">
						<stop offset="0%" stopColor="#22d3ee" stopOpacity="0.85" />
						<stop offset="100%" stopColor="#3b82f6" stopOpacity="0.6" />
					</linearGradient>
					<linearGradient id="card-amber" x1="0" y1="0" x2="1" y2="1">
						<stop offset="0%" stopColor="#fbbf24" stopOpacity="0.85" />
						<stop offset="100%" stopColor="#f97316" stopOpacity="0.6" />
					</linearGradient>
					<radialGradient id="orb-glow" cx="50%" cy="50%" r="50%">
						<stop offset="0%" stopColor="white" stopOpacity="0.3" />
						<stop offset="100%" stopColor="white" stopOpacity="0" />
					</radialGradient>
				</defs>

				{/* ambient orbs */}
				<motion.circle
					cx="40"
					cy="60"
					r="48"
					fill="url(#card-violet)"
					initial={{ opacity: 0.2 }}
					animate={{ opacity: [0.2, 0.4, 0.2], y: [0, -8, 0] }}
					transition={{
						duration: 7,
						repeat: Number.POSITIVE_INFINITY,
						ease: "easeInOut",
					}}
					style={{ filter: "blur(28px)" }}
				/>
				<motion.circle
					cx="260"
					cy="180"
					r="56"
					fill="url(#card-cyan)"
					initial={{ opacity: 0.15 }}
					animate={{ opacity: [0.15, 0.35, 0.15], y: [0, 10, 0] }}
					transition={{
						duration: 9,
						repeat: Number.POSITIVE_INFINITY,
						ease: "easeInOut",
					}}
					style={{ filter: "blur(32px)" }}
				/>

				{/* floating course cards */}
				<motion.g
					initial={{ y: 0 }}
					animate={{ y: [0, -4, 0] }}
					transition={{
						duration: 6,
						repeat: Number.POSITIVE_INFINITY,
						ease: "easeInOut",
					}}
				>
					<g transform="translate(60 110) rotate(-12)">
						<rect
							x="0"
							y="0"
							width="120"
							height="78"
							rx="14"
							fill="url(#card-cyan)"
							opacity="0.85"
						/>
						<rect
							x="14"
							y="46"
							width="58"
							height="6"
							rx="3"
							fill="white"
							opacity="0.5"
						/>
						<rect
							x="14"
							y="58"
							width="38"
							height="6"
							rx="3"
							fill="white"
							opacity="0.3"
						/>
						<circle cx="98" cy="22" r="10" fill="white" opacity="0.6" />
					</g>
				</motion.g>

				<motion.g
					initial={{ y: 0 }}
					animate={{ y: [0, -7, 0] }}
					transition={{
						duration: 7,
						repeat: Number.POSITIVE_INFINITY,
						ease: "easeInOut",
						delay: 0.5,
					}}
				>
					<g transform="translate(140 70) rotate(6)">
						<rect
							x="0"
							y="0"
							width="130"
							height="84"
							rx="16"
							fill="url(#card-violet)"
						/>
						<rect
							x="16"
							y="50"
							width="64"
							height="6"
							rx="3"
							fill="white"
							opacity="0.55"
						/>
						<rect
							x="16"
							y="62"
							width="42"
							height="6"
							rx="3"
							fill="white"
							opacity="0.35"
						/>
						<g transform="translate(96 18)">
							<path d="M0 8 L8 0 L16 8 L8 16 Z" fill="white" opacity="0.7" />
						</g>
					</g>
				</motion.g>

				<motion.g
					initial={{ y: 0 }}
					animate={{ y: [0, -5, 0] }}
					transition={{
						duration: 8,
						repeat: Number.POSITIVE_INFINITY,
						ease: "easeInOut",
						delay: 1,
					}}
				>
					<g transform="translate(190 130) rotate(-4)">
						<rect
							x="0"
							y="0"
							width="100"
							height="70"
							rx="12"
							fill="url(#card-amber)"
						/>
						<rect
							x="12"
							y="42"
							width="50"
							height="5"
							rx="2.5"
							fill="white"
							opacity="0.6"
						/>
						<rect
							x="12"
							y="52"
							width="32"
							height="5"
							rx="2.5"
							fill="white"
							opacity="0.4"
						/>
						<circle cx="78" cy="20" r="9" fill="white" opacity="0.7" />
					</g>
				</motion.g>

				{/* sparkles */}
				{[
					{ cx: 48, cy: 38, d: 0, r: 1.6 },
					{ cx: 282, cy: 56, d: 0.8, r: 2.2 },
					{ cx: 246, cy: 94, d: 1.6, r: 1.4 },
					{ cx: 28, cy: 196, d: 2.4, r: 1.8 },
					{ cx: 304, cy: 218, d: 0.4, r: 1.4 },
				].map((s, i) => (
					<motion.circle
						key={i}
						cx={s.cx}
						cy={s.cy}
						r={s.r}
						fill="white"
						initial={{ opacity: 0 }}
						animate={{ opacity: [0, 1, 0], scale: [0.8, 1.4, 0.8] }}
						transition={{
							duration: 2.4,
							repeat: Number.POSITIVE_INFINITY,
							delay: s.d,
							ease: "easeInOut",
						}}
					/>
				))}
			</svg>
		</div>
	);
}
