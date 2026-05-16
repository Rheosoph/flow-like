"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

export function AnimatedStudyHatIcon({ className }: { className?: string }) {
	const [isHovered, setIsHovered] = useState(false);

	const spring = { type: "spring", stiffness: 420, damping: 18 };
	const pop = { type: "spring", stiffness: 520, damping: 14 };

	return (
		<motion.svg
			xmlns="http://www.w3.org/2000/svg"
			width="24"
			height="24"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			strokeWidth="1.9"
			strokeLinecap="round"
			strokeLinejoin="round"
			onPointerEnter={() => setIsHovered(true)}
			onPointerLeave={() => setIsHovered(false)}
			animate={isHovered ? "hover" : "initial"}
			initial="initial"
			className={cn(
				"cursor-pointer overflow-visible text-slate-800 dark:text-slate-200",
				className,
			)}
		>
			<title>University study hat</title>
			<motion.g
				className="stroke-amber-400 fill-amber-300/80 dark:stroke-amber-300 dark:fill-amber-300/70"
				strokeWidth="1.4"
			>
				<motion.path
					d="M5 4.2 5.9 6l2 .3-1.4 1.4.3 2-1.8-.9-1.8.9.3-2L2.1 6.3 4.1 6Z"
					variants={{
						initial: { opacity: 0, scale: 0.35, rotate: -20, x: 3, y: 4 },
						hover: {
							opacity: [0, 1, 1, 0],
							scale: [0.35, 1, 0.9],
							rotate: [-20, 8, 18],
							x: [3, 0, -1],
							y: [4, 0, -1],
							transition: { duration: 0.8, ease: "easeOut" },
						},
					}}
				/>
				<motion.path
					d="M20 3.8 20.6 5l1.4.2-1 .9.2 1.4-1.2-.7-1.2.7.2-1.4-1-.9 1.4-.2Z"
					variants={{
						initial: { opacity: 0, scale: 0.35, rotate: 18, x: -2, y: 4 },
						hover: {
							opacity: [0, 1, 1, 0],
							scale: [0.35, 1, 0.85],
							rotate: [18, -12, -26],
							x: [-2, 0, 1],
							y: [4, 0, -1],
							transition: { duration: 0.75, ease: "easeOut", delay: 0.08 },
						},
					}}
				/>
			</motion.g>

			<motion.g
				style={{ transformOrigin: "12px 10px" }}
				variants={{
					initial: { y: 0, rotate: 0, scale: 1, transition: spring },
					hover: {
						y: [0, -4, -1],
						rotate: [0, -9, 0],
						scale: [1, 1.08, 1.02],
						transition: { duration: 0.65, ease: "easeInOut" },
					},
				}}
			>
				<motion.path
					d="M2.8 8 12 3.8 21.2 8 12 12.2Z"
					className={cn(
						"transition-colors duration-300",
						isHovered
							? "fill-indigo-100 stroke-indigo-500 dark:fill-indigo-950/70 dark:stroke-indigo-300"
							: "fill-transparent stroke-currentColor",
					)}
				/>
				<motion.path
					d="M6.2 10.1v4.4c1.6 1.4 3.5 2.1 5.8 2.1s4.2-.7 5.8-2.1v-4.4"
					className={cn(
						"transition-colors duration-300",
						isHovered
							? "fill-indigo-50 stroke-indigo-500 dark:fill-indigo-950/50 dark:stroke-indigo-300"
							: "fill-transparent stroke-currentColor",
					)}
					variants={{
						initial: { pathLength: 1 },
						hover: {
							pathLength: [0.75, 1],
							transition: { duration: 0.35, ease: "easeOut", delay: 0.15 },
						},
					}}
				/>
				<motion.path
					d="M12 12.2v4.4"
					className={cn(
						"transition-colors duration-300",
						isHovered
							? "stroke-indigo-400 dark:stroke-indigo-200"
							: "stroke-currentColor",
					)}
					variants={{
						initial: { opacity: 0.55 },
						hover: { opacity: 1, transition: { duration: 0.2 } },
					}}
				/>
			</motion.g>

			<motion.g
				style={{ transformOrigin: "18.5px 8.5px" }}
				variants={{
					initial: { rotate: 0, transition: spring },
					hover: {
						rotate: [0, 24, -18, 10, 0],
						transition: { duration: 0.8, ease: "easeInOut" },
					},
				}}
			>
				<motion.path
					d="M18.5 8.8v5.5"
					className={cn(
						"transition-colors duration-300",
						isHovered
							? "stroke-amber-500 dark:stroke-amber-300"
							: "stroke-currentColor",
					)}
				/>
				<motion.circle
					cx="18.5"
					cy="15.7"
					r="1.15"
					className={cn(
						"transition-colors duration-300",
						isHovered
							? "fill-amber-400 stroke-amber-500 dark:fill-amber-300 dark:stroke-amber-300"
							: "fill-background stroke-currentColor",
					)}
					variants={{
						initial: { scale: 1, transition: spring },
						hover: { scale: [1, 1.35, 1], transition: pop },
					}}
				/>
			</motion.g>

			<motion.path
				d="M7.4 19.2c1.3.7 2.9 1 4.6 1s3.3-.3 4.6-1"
				className="stroke-emerald-500 dark:stroke-emerald-300"
				variants={{
					initial: { pathLength: 0, opacity: 0 },
					hover: {
						pathLength: 1,
						opacity: [0, 1, 0.8],
						transition: { duration: 0.45, ease: "easeOut", delay: 0.25 },
					},
				}}
			/>
		</motion.svg>
	);
}
