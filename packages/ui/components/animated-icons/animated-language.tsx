"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Globe that spins on hover.
 * Size locked to 16x16 to match the other sidebar icons.
 * Hover: a dashed orbit snaps in and rotates, the meridian sweeps around the
 * sphere and the glyph pair below the equator swaps latin -> han.
 */
export function AnimatedLanguageIcon({ className }: { className?: string }) {
	const [isHovered, setIsHovered] = useState(false);

	return (
		<div
			className={cn(
				"relative flex items-center justify-center w-4 h-4 cursor-pointer text-slate-800 dark:text-slate-200",
				className,
			)}
			onPointerEnter={() => setIsHovered(true)}
			onPointerLeave={() => setIsHovered(false)}
		>
			<motion.svg
				xmlns="http://www.w3.org/2000/svg"
				width="16"
				height="16"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				strokeWidth="2"
				strokeLinecap="round"
				strokeLinejoin="round"
				animate={isHovered ? "hover" : "initial"}
				initial="initial"
				aria-hidden="true"
				className="absolute w-full h-full"
			>
				{/* --- LAYER 1: ORBIT --- */}
				<motion.circle
					cx="12"
					cy="12"
					r="11"
					className="stroke-primary"
					strokeWidth="1.5"
					strokeDasharray="1.5 4"
					style={{ transformOrigin: "12px 12px" }}
					variants={{
						initial: { opacity: 0, scale: 0.5 },
						hover: {
							opacity: 1,
							scale: 1,
							rotate: 360,
							transition: {
								opacity: { duration: 0.2 },
								scale: { type: "spring", stiffness: 220, damping: 14 },
								rotate: {
									repeat: Number.POSITIVE_INFINITY,
									duration: 6,
									ease: "linear",
								},
							},
						},
					}}
				/>

				{/* --- LAYER 2: THE SPHERE --- */}
				<motion.g
					className={cn(
						"transition-colors duration-300",
						isHovered ? "stroke-primary" : "stroke-current",
					)}
					style={{ transformOrigin: "12px 12px" }}
					variants={{
						initial: { scale: 1 },
						hover: {
							scale: 1.06,
							transition: { type: "spring", stiffness: 400, damping: 12 },
						},
					}}
				>
					<circle cx="12" cy="12" r="9" />
					<path d="M3.2 9.5h17.6" />
					<motion.path
						d="M4.8 15.5h14.4"
						variants={{
							initial: { opacity: 1 },
							hover: { opacity: 0, transition: { duration: 0.15 } },
						}}
					/>
					{/* Meridian: squashing it horizontally reads as the globe turning. */}
					<motion.path
						d="M12 3c2.4 2.5 3.7 5.6 3.7 9S14.4 18.5 12 21c-2.4-2.5-3.7-5.6-3.7-9S9.6 5.5 12 3z"
						style={{ transformOrigin: "12px 12px" }}
						variants={{
							initial: { scaleX: 1 },
							hover: {
								scaleX: [1, 0.12, 1],
								transition: {
									repeat: Number.POSITIVE_INFINITY,
									duration: 2.6,
									ease: "easeInOut",
								},
							},
						}}
					/>
				</motion.g>

				{/* --- LAYER 3: GLYPH SWAP --- */}
				<motion.g
					className="fill-primary stroke-none"
					fontSize="7"
					fontWeight="700"
					textAnchor="middle"
					variants={{
						initial: { opacity: 0 },
						hover: { opacity: 1, transition: { duration: 0.2 } },
					}}
				>
					<motion.text
						x="12"
						y="18.4"
						variants={{
							initial: { opacity: 1 },
							hover: {
								opacity: [1, 1, 0, 0],
								transition: {
									repeat: Number.POSITIVE_INFINITY,
									duration: 2.6,
									times: [0, 0.4, 0.5, 1],
								},
							},
						}}
					>
						A
					</motion.text>
					<motion.text
						x="12"
						y="18.4"
						variants={{
							initial: { opacity: 0 },
							hover: {
								opacity: [0, 0, 1, 1],
								transition: {
									repeat: Number.POSITIVE_INFINITY,
									duration: 2.6,
									times: [0, 0.4, 0.5, 1],
								},
							},
						}}
					>
						文
					</motion.text>
				</motion.g>
			</motion.svg>
		</div>
	);
}
