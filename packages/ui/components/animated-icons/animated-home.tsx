"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * House to "Cozy Home" Transformation
 * Base state: A wider, grounded house silhouette.
 * Hover state: Roof lifts, warm light turns on, door appears, smoke loops.
 */
export function AnimatedHomeIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    const springTransition = { type: "spring", stiffness: 300, damping: 15 };
    const returnTransition = { type: "spring", stiffness: 300, damping: 25 };

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
            className={cn("cursor-pointer text-slate-800 dark:text-slate-200", className)}
        >
            {/* --- LAYER 1: WALLS & CHIMNEY --- */}
            <motion.g
                variants={{
                    initial: { y: 0, transition: returnTransition },
                    hover: { y: 1, transition: springTransition }
                }}
            >
                {/* Outline Walls (Wider footprint) */}
                <path d="M4 11 V20 A2 2 0 0 0 6 22 H18 A2 2 0 0 0 20 20 V11" />

                {/* Filled Walls */}
                <motion.path
                    d="M4 11 V20 A2 2 0 0 0 6 22 H18 A2 2 0 0 0 20 20 V11 Z"
                    className="fill-blue-100 stroke-none dark:fill-blue-900/40"
                    variants={{
                        initial: { opacity: 0, transition: { duration: 0.2 } },
                        hover: { opacity: 1, transition: { duration: 0.3 } }
                    }}
                />

                {/* Outline Chimney (Adjusted for wider roof) */}
                <path d="M16 5.5 V2 H18 V7.5" />

                {/* Filled Chimney */}
                <motion.path
                    d="M16 5.5 V2 H18 V7.5 Z"
                    className="fill-blue-200 stroke-none dark:fill-blue-800"
                    variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
                />

                {/* Looping Smoke Puff 1 */}
                <motion.circle cx="17" cy="0" r="1.5" className="fill-slate-300 stroke-none dark:fill-slate-500"
                    variants={{
                        initial: { y: 5, scale: 0, opacity: 0 },
                        hover: {
                            y: -5, scale: 1.5, opacity: [0, 1, 0],
                            transition: { duration: 1.5, ease: "easeOut", repeat: Infinity, delay: 0.2 }
                        }
                    }}
                />

                {/* Looping Smoke Puff 2 */}
                <motion.circle cx="18.5" cy="-2" r="1" className="fill-slate-300 stroke-none dark:fill-slate-500"
                    variants={{
                        initial: { y: 5, scale: 0, opacity: 0 },
                        hover: {
                            y: -7, scale: 1.5, opacity: [0, 1, 0],
                            transition: { duration: 1.5, ease: "easeOut", repeat: Infinity, delay: 0.8 }
                        }
                    }}
                />
            </motion.g>

            {/* --- LAYER 2: THE ROOF --- */}
            <motion.g
                variants={{
                    initial: { y: 0, transition: returnTransition },
                    hover: { y: -2, transition: springTransition }
                }}
            >
                {/* Outline Roof (Wider span) */}
                <path d="M2 12 L12 3 L22 12" />

                {/* Filled Roof */}
                <motion.path
                    d="M2 12 L12 3 L22 12 Z"
                    className="fill-blue-500 stroke-blue-500 dark:fill-blue-600 dark:stroke-blue-600"
                    variants={{
                        initial: { opacity: 0, transition: { duration: 0.2 } },
                        hover: { opacity: 1, transition: { duration: 0.3 } }
                    }}
                />

                {/* Glowing Attic Window */}
                <motion.circle
                    cx="12" cy="7.5" r="1.5"
                    className="fill-amber-300 stroke-amber-500 dark:fill-amber-400 dark:stroke-amber-600"
                    variants={{
                        initial: { scale: 0, opacity: 0, transition: returnTransition },
                        hover: { scale: 1, opacity: 1, transition: { type: "spring", stiffness: 400, damping: 12, delay: 0.15 } }
                    }}
                />
            </motion.g>

            {/* --- LAYER 3: THE DOOR --- */}
            <motion.g
                style={{ transformOrigin: "12px 22px" }}
                variants={{
                    initial: { scaleY: 0, opacity: 0, transition: returnTransition },
                    hover: { scaleY: 1, opacity: 1, transition: { ...springTransition, delay: 0.1 } }
                }}
            >
                <path
                    d="M10 22 V16 A1 1 0 0 1 11 15 H13 A1 1 0 0 1 14 16 V22 Z"
                    className="fill-blue-600 stroke-blue-700 dark:fill-blue-500 dark:stroke-blue-400"
                />
                {/* Tiny Doorknob */}
                <circle cx="13" cy="19" r="0.5" className="fill-white stroke-none" />
            </motion.g>
        </motion.svg>
    );
}