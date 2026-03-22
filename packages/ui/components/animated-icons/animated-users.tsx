"use client";
import { motion } from "framer-motion";
import { cn } from "../../lib/utils";

/** Two people - front person fades in, back person follows. Indigo on hover. */
export function AnimatedUsersIcon({ className }: { className?: string }) {
	return (
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
			className={cn("group-hover/icon:text-indigo-400 transition-colors duration-150", className)}
		>
			{/* Front person head */}
			<motion.path
				d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"
				variants={{
					initial: { pathLength: 1 },
					hover: {
						pathLength: [0, 1],
						transition: { duration: 0.4, ease: "easeOut" },
					},
				}}
			/>
			<motion.circle
				cx="9"
				cy="7"
				r="4"
				variants={{
					initial: { scale: 1, opacity: 1 },
					hover: {
						scale: [0.6, 1.1, 1],
						opacity: [0, 1],
						transition: { duration: 0.35, type: "spring", stiffness: 300, damping: 14 },
					},
				}}
				style={{ transformOrigin: "9px 7px" }}
			/>
			{/* Back person */}
			<motion.path
				d="M22 21v-2a4 4 0 0 0-3-3.87"
				variants={{
					initial: { pathLength: 1 },
					hover: {
						pathLength: [0, 1],
						transition: { duration: 0.3, ease: "easeOut", delay: 0.3 },
					},
				}}
			/>
			<motion.path
				d="M16 3.13a4 4 0 0 1 0 7.75"
				variants={{
					initial: { pathLength: 1 },
					hover: {
						pathLength: [0, 1],
						transition: { duration: 0.3, ease: "easeOut", delay: 0.35 },
					},
				}}
			/>
		</motion.svg>
	);
}
