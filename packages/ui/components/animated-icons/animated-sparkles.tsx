"use client";
import { motion } from "framer-motion";
import { cn } from "../../lib/utils";

/** Stars pop in sequentially and scale. Turns yellow on hover. */
export function AnimatedSparklesIcon({ className }: { className?: string }) {
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
			className={cn("group-hover/icon:text-yellow-400 transition-colors duration-150", className)}
		>
			{/* Large star */}
			<motion.path
				d="M12 3 9.5 9.5 3 12l6.5 2.5L12 21l2.5-6.5L21 12l-6.5-2.5z"
				variants={{
					initial: { scale: 1, opacity: 1 },
					hover: {
						scale: [0.5, 1.2, 1],
						opacity: [0, 1],
						transition: { duration: 0.4, type: "spring", stiffness: 300, damping: 12 },
					},
				}}
				style={{ transformOrigin: "12px 12px" }}
			/>
			{/* Small star top-right */}
			<motion.path
				d="M19 3l-.8 2L16 6l2.2.8L19 9l.8-2.2L22 6l-2.2-.8z"
				variants={{
					initial: { scale: 1, opacity: 1 },
					hover: {
						scale: [0, 1.3, 1],
						opacity: [0, 1],
						transition: { duration: 0.35, delay: 0.2, type: "spring", stiffness: 350, damping: 10 },
					},
				}}
				style={{ transformOrigin: "19px 6px" }}
			/>
			{/* Small star bottom-left */}
			<motion.path
				d="M5 15l-.8 2L2 18l2.2.8L5 21l.8-2.2L8 18l-2.2-.8z"
				variants={{
					initial: { scale: 1, opacity: 1 },
					hover: {
						scale: [0, 1.3, 1],
						opacity: [0, 1],
						transition: { duration: 0.35, delay: 0.35, type: "spring", stiffness: 350, damping: 10 },
					},
				}}
				style={{ transformOrigin: "5px 18px" }}
			/>
		</motion.svg>
	);
}
