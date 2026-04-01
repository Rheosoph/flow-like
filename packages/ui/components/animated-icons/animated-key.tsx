"use client";
import { motion } from "framer-motion";
import { cn } from "../../lib/utils";

/** Lucide KeyRound – shaft draws, then handle circle pops in. Teal on hover. */
export function AnimatedKeyIcon({ className }: { className?: string }) {
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
			className={cn(
				"group-hover/icon:text-teal-400 transition-colors duration-150",
				className,
			)}
		>
			{/* Key body + shaft */}
			<motion.path
				d="M2 18v3c0 .6.4 1 1 1h4v-3h3v-3h2l1.4-1.4a6.5 6.5 0 1 0-4-4Z"
				variants={{
					initial: { pathLength: 1 },
					hover: {
						pathLength: [0, 1],
						transition: { duration: 0.5, ease: "easeInOut" },
					},
				}}
			/>
			{/* Handle circle */}
			<motion.circle
				cx="16.5"
				cy="7.5"
				r="2.5"
				variants={{
					initial: { scale: 1, opacity: 1 },
					hover: {
						scale: [0, 1.2, 1],
						opacity: [0, 1],
						transition: { duration: 0.35, delay: 0.4, type: "spring", stiffness: 300, damping: 12 },
					},
				}}
				style={{ transformOrigin: "16.5px 7.5px" }}
			/>
		</motion.svg>
	);
}
