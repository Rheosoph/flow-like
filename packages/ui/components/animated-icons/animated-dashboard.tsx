"use client";
import { motion } from "framer-motion";
import { cn } from "../../lib/utils";

/** Four grid panels flip into place in sequence. Turns sky on hover. */
export function AnimatedDashboardIcon({ className }: { className?: string }) {
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
			className={cn("group-hover/icon:text-sky-400 transition-colors duration-150", className)}
		>
			{/* Top-left panel */}
			<motion.rect
				x="3"
				y="3"
				width="7"
				height="9"
				rx="1"
				variants={{
					initial: { scaleY: 1, opacity: 1 },
					hover: {
						scaleY: [0, 1.1, 1],
						opacity: [0, 1],
						transition: { duration: 0.3, type: "spring", stiffness: 280, damping: 12 },
					},
				}}
				style={{ transformOrigin: "6.5px 3px" }}
			/>
			{/* Top-right panel */}
			<motion.rect
				x="14"
				y="3"
				width="7"
				height="5"
				rx="1"
				variants={{
					initial: { scaleY: 1, opacity: 1 },
					hover: {
						scaleY: [0, 1.1, 1],
						opacity: [0, 1],
						transition: { duration: 0.3, type: "spring", stiffness: 280, damping: 12, delay: 0.1 },
					},
				}}
				style={{ transformOrigin: "17.5px 3px" }}
			/>
			{/* Bottom-left panel */}
			<motion.rect
				x="3"
				y="14"
				width="7"
				height="7"
				rx="1"
				variants={{
					initial: { scaleY: 1, opacity: 1 },
					hover: {
						scaleY: [0, 1.1, 1],
						opacity: [0, 1],
						transition: { duration: 0.3, type: "spring", stiffness: 280, damping: 12, delay: 0.2 },
					},
				}}
				style={{ transformOrigin: "6.5px 14px" }}
			/>
			{/* Bottom-right panel */}
			<motion.rect
				x="14"
				y="12"
				width="7"
				height="9"
				rx="1"
				variants={{
					initial: { scaleY: 1, opacity: 1 },
					hover: {
						scaleY: [0, 1.1, 1],
						opacity: [0, 1],
						transition: { duration: 0.3, type: "spring", stiffness: 280, damping: 12, delay: 0.3 },
					},
				}}
				style={{ transformOrigin: "17.5px 12px" }}
			/>
		</motion.svg>
	);
}
