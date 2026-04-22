"use client";

import { motion } from "framer-motion";
import { useId, useState } from "react";
import { cn } from "../../lib/utils";

/**
 * GIANT Overgrown Blooming Package Icon
 * Base state: Pixel-perfect match to standard Lucide 'Package 2' outline.
 * Hover state: The box drops to the floor. A MASSIVE bouquet blooms,
 * intentionally breaking the top boundaries of the 24x24 canvas for a huge,
 * dramatic pop-out effect. Front vines wrap the bottom for 3D depth.
 */
export function AnimatedPackageIcon({ className }: { className?: string }) {
	const [isHovered, setIsHovered] = useState(false);
	const id = useId();

	const outerBoxPath =
		"M16.76 3a2 2 0 0 1 1.8 1.1l2.23 4.479a2 2 0 0 1 .21.891V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V9.472a2 2 0 0 1 .211-.894L5.45 4.1A2 2 0 0 1 7.24 3z";
	const centerCrease = "M12 3v6";
	const flapLine = "M3.054 9.013h17.893";

	const BoxStrokes = () => (
		<>
			<path d={centerCrease} />
			<path d={outerBoxPath} />
			<path d={flapLine} />
		</>
	);

	const springFlaps = { type: "spring", stiffness: 220, damping: 15 };
	const springFlower = { type: "spring", stiffness: 220, damping: 14 }; // Slightly looser spring for huge growth
	const returnTransition = { type: "spring", stiffness: 350, damping: 25 };

	return (
		<motion.svg
			xmlns="http://www.w3.org/2000/svg"
			width="24"
			height="24"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			strokeWidth="2"
			strokeLinecap="round"
			strokeLinejoin="round"
			onPointerEnter={() => setIsHovered(true)}
			onPointerLeave={() => setIsHovered(false)}
			animate={isHovered ? "hover" : "initial"}
			initial="initial"
			// ESSENTIAL: Allows the giant flowers to render above y=0
			style={{ overflow: "visible" }}
			className={cn(
				"cursor-pointer text-slate-800 dark:text-slate-200",
				className,
			)}
		>
			<defs>
				<clipPath id={`${id}-bottom`}>
					<rect x="-5" y="9.013" width="34" height="20" />
				</clipPath>
				<clipPath id={`${id}-left-flap`}>
					<rect x="-5" y="-10" width="17" height="19.013" />
				</clipPath>
				<clipPath id={`${id}-right-flap`}>
					<rect x="12" y="-10" width="17" height="19.013" />
				</clipPath>
			</defs>

			{/* --- SHADOW --- */}
			<motion.ellipse
				cx="12"
				cy="20.5"
				rx="7"
				ry="1"
				className="fill-slate-200 stroke-none dark:fill-slate-800"
				variants={{
					initial: {
						scaleX: 0.8,
						opacity: 0,
						y: 0,
						transition: returnTransition,
					},
					hover: { scaleX: 1.3, opacity: 1, y: 2.5, transition: springFlaps },
				}}
			/>

			{/* --- MASTER "DROP" GROUP ---
                Drops 2.5px so the box bottom hits the physical SVG floor.
            */}
			<motion.g
				variants={{
					initial: { y: 0, transition: returnTransition },
					hover: { y: 2.5, transition: springFlaps },
				}}
			>
				{/* --- 1. BOX INTERIOR --- */}
				<motion.polygon
					points="5.45,4.1 18.55,4.1 20.946,9.013 3.054,9.013"
					className="fill-amber-900 stroke-none dark:fill-amber-950"
					variants={{
						initial: { opacity: 0, transition: { duration: 0.1 } },
						hover: { opacity: 1, transition: { duration: 0.2 } },
					}}
				/>

				{/* --- 2. THE GIANT BOUQUET --- */}
				<motion.g
					variants={{
						initial: {
							y: 2,
							scale: 0.3,
							opacity: 0,
							transition: { duration: 0.2 },
						},
						hover: {
							y: 0,
							scale: 1,
							opacity: 1,
							transition: { delay: 0.1, ...springFlower },
						},
					}}
					style={{ transformOrigin: "12px 9px" }}
				>
					{/* Glowing Sparkles pushed further out */}
					<Sparkle cx={2} cy={0} delay={0.3} />
					<Sparkle cx={22} cy={2} delay={0.4} />
					<Sparkle cx={12} cy={-5} delay={0.5} />

					{/* Main Thick Stem */}
					<motion.path
						d="M 12 9 Q 9 5, 12 2.5"
						className="stroke-emerald-500 dark:stroke-emerald-400"
						strokeWidth="2"
						fill="none"
						variants={{
							initial: { pathLength: 0 },
							hover: {
								pathLength: 1,
								transition: { duration: 0.5, delay: 0.1 },
							},
						}}
					/>

					{/* Side Stems spread wide */}
					<motion.path
						d="M 12 9 Q 7 6, 4 5"
						className="stroke-emerald-600 dark:stroke-emerald-500"
						strokeWidth="1.5"
						fill="none"
						variants={{
							initial: { pathLength: 0 },
							hover: {
								pathLength: 1,
								transition: { duration: 0.4, delay: 0.15 },
							},
						}}
					/>
					<motion.path
						d="M 12 9 Q 17 6, 20 5"
						className="stroke-emerald-600 dark:stroke-emerald-500"
						strokeWidth="1.5"
						fill="none"
						variants={{
							initial: { pathLength: 0 },
							hover: {
								pathLength: 1,
								transition: { duration: 0.4, delay: 0.15 },
							},
						}}
					/>

					{/* CENTRAL GIANT FLOWER (Massive petals, bursts past y=0) */}
					<motion.g
						variants={{
							initial: { scale: 0 },
							hover: { scale: 1, transition: { type: "spring", delay: 0.3 } },
						}}
						style={{ transformOrigin: "12px 2.5px" }}
					>
						{/* 8 thick petals for a full look */}
						{[0, 45, 90, 135, 180, 225, 270, 315].map((angle) => (
							<ellipse
								key={angle}
								cx="12"
								cy="2.5"
								rx="2.5"
								ry="6.5"
								className="fill-pink-400 stroke-none dark:fill-pink-500"
								style={{
									transformOrigin: "12px 2.5px",
									transform: `rotate(${angle}deg)`,
								}}
							/>
						))}
						<circle
							cx="12"
							cy="2.5"
							r="3"
							className="fill-yellow-300 stroke-none dark:fill-yellow-400"
						/>
					</motion.g>

					{/* LEFT SIDE FLOWER (Pushed far left) */}
					<motion.g
						variants={{
							initial: { scale: 0 },
							hover: { scale: 1, transition: { type: "spring", delay: 0.35 } },
						}}
						style={{ transformOrigin: "4px 5px" }}
					>
						{[0, 60, 120, 180, 240, 300].map((angle) => (
							<ellipse
								key={angle}
								cx="4"
								cy="5"
								rx="1.5"
								ry="4"
								className="fill-purple-400 stroke-none dark:fill-purple-500"
								style={{
									transformOrigin: "4px 5px",
									transform: `rotate(${angle}deg)`,
								}}
							/>
						))}
						<circle
							cx="4"
							cy="5"
							r="2"
							className="fill-yellow-300 stroke-none dark:fill-yellow-400"
						/>
					</motion.g>

					{/* RIGHT SIDE FLOWER (Pushed far right) */}
					<motion.g
						variants={{
							initial: { scale: 0 },
							hover: { scale: 1, transition: { type: "spring", delay: 0.4 } },
						}}
						style={{ transformOrigin: "20px 5px" }}
					>
						{[30, 90, 150, 210, 270, 330].map((angle) => (
							<ellipse
								key={angle}
								cx="20"
								cy="5"
								rx="1.5"
								ry="4"
								className="fill-orange-400 stroke-none dark:fill-orange-500"
								style={{
									transformOrigin: "20px 5px",
									transform: `rotate(${angle}deg)`,
								}}
							/>
						))}
						<circle
							cx="20"
							cy="5"
							r="2"
							className="fill-yellow-300 stroke-none dark:fill-yellow-400"
						/>
					</motion.g>
				</motion.g>

				{/* --- 3. THE BOX FRONT BODY --- */}
				<g clipPath={`url(#${id}-bottom)`}>
					<motion.path
						d={outerBoxPath}
						className="fill-amber-300 stroke-none dark:fill-amber-800"
						variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
					/>
					<BoxStrokes />
				</g>

				{/* --- 4. LEFT FLAP --- */}
				<motion.g
					style={{ transformOrigin: "3.054px 9.013px" }}
					variants={{
						initial: { rotate: 0 },
						hover: { rotate: -115, transition: springFlaps },
					}}
				>
					<g clipPath={`url(#${id}-left-flap)`}>
						<motion.path
							d={outerBoxPath}
							className="fill-amber-200 stroke-none dark:fill-amber-700"
							variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
						/>
						<BoxStrokes />
					</g>
				</motion.g>

				{/* --- 5. RIGHT FLAP --- */}
				<motion.g
					style={{ transformOrigin: "20.946px 9.013px" }}
					variants={{
						initial: { rotate: 0 },
						hover: { rotate: 115, transition: springFlaps },
					}}
				>
					<g clipPath={`url(#${id}-right-flap)`}>
						<motion.path
							d={outerBoxPath}
							className="fill-amber-200 stroke-none dark:fill-amber-700"
							variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
						/>
						<BoxStrokes />
					</g>
				</motion.g>

				{/* --- 6. OVERGROWTH VINES --- */}
				<motion.g
					variants={{
						initial: { opacity: 0 },
						hover: { opacity: 1, transition: { delay: 0.2 } },
					}}
				>
					<motion.path
						d="M 6.5 9.1 Q 3 11, 4 15 T 7 16"
						className="stroke-emerald-500 dark:stroke-emerald-400"
						strokeWidth="1.5"
						fill="none"
						variants={{
							initial: { pathLength: 0 },
							hover: {
								pathLength: 1,
								transition: { duration: 0.6, delay: 0.25, ease: "easeOut" },
							},
						}}
					/>
					<motion.path
						d="M 4 15 Q 1 14, 2 12 Q 5 13, 4 15 Z"
						className="fill-emerald-500 stroke-none dark:fill-emerald-400"
						variants={{
							initial: { scale: 0 },
							hover: { scale: 1, transition: { delay: 0.5 } },
						}}
						style={{ transformOrigin: "4px 15px" }}
					/>

					<motion.path
						d="M 17.5 9.1 Q 21 11, 20 15 T 17 16"
						className="stroke-emerald-600 dark:stroke-emerald-500"
						strokeWidth="1.5"
						fill="none"
						variants={{
							initial: { pathLength: 0 },
							hover: {
								pathLength: 1,
								transition: { duration: 0.6, delay: 0.3, ease: "easeOut" },
							},
						}}
					/>
					<motion.path
						d="M 20 15 Q 23 14, 22 12 Q 19 13, 20 15 Z"
						className="fill-emerald-600 stroke-none dark:fill-emerald-500"
						variants={{
							initial: { scale: 0 },
							hover: { scale: 1, transition: { delay: 0.6 } },
						}}
						style={{ transformOrigin: "20px 15px" }}
					/>
				</motion.g>
			</motion.g>
		</motion.svg>
	);
}

function Sparkle({ cx, cy, delay }: { cx: number; cy: number; delay: number }) {
	return (
		<motion.path
			d={`M ${cx} ${cy - 2} L ${cx + 0.5} ${cy - 0.5} L ${cx + 2} ${cy} L ${cx + 0.5} ${cy + 0.5} L ${cx} ${cy + 2} L ${cx - 0.5} ${cy + 0.5} L ${cx - 2} ${cy} L ${cx - 0.5} ${cy - 0.5} Z`}
			className="fill-yellow-400 stroke-none dark:fill-yellow-300"
			variants={{
				initial: { scale: 0, opacity: 0, rotate: -45 },
				hover: {
					scale: [0, 1, 0],
					opacity: [0, 1, 0],
					rotate: 0,
					transition: { duration: 0.8, delay, ease: "easeInOut" },
				},
			}}
			style={{ transformOrigin: `${cx}px ${cy}px` }}
		/>
	);
}
