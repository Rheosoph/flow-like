"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Cascading Docs & Holographic Typing
 * Base: Standard minimalist file-text icon.
 * Hover: Main file lifts and glows amber. Two background translucent files
 * fan out for 3D depth. A glowing cursor sequentially types out the text lines.
 */
export function AnimatedDocsIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    // Physics for snapping the files back into a single stack
    const returnTransition = { type: "spring", stiffness: 300, damping: 25 };
    const cascadeTransition = { type: "spring", stiffness: 400, damping: 15 };

    return (
        <div
            className={cn(
                "relative flex items-center justify-center w-4 h-4 cursor-pointer text-slate-800 dark:text-slate-200",
                className
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
                stroke="currentColor" // Ensures the base state is visible and matches standard text
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: BACK HOLOGRAPHIC FILE --- */}
                <motion.g
                    // Hardcoded styling because it is completely hidden in base state
                    className="stroke-amber-600/30 dark:stroke-amber-500/30 fill-amber-500/10 dark:fill-amber-500/5"
                    variants={{
                        initial: { x: 0, y: 0, opacity: 0, scale: 0.9, transition: { duration: 0.15 } },
                        hover: { x: 4, y: 4, opacity: 1, scale: 1, transition: { ...cascadeTransition, delay: 0.1 } }
                    }}
                >
                    <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
                    <path d="M14 2v4a2 2 0 0 0 2 2h4" />
                </motion.g>

                {/* --- LAYER 2: MIDDLE HOLOGRAPHIC FILE --- */}
                <motion.g
                    className="stroke-amber-500/50 dark:stroke-amber-400/50 fill-amber-400/10 dark:fill-amber-400/5"
                    variants={{
                        initial: { x: 0, y: 0, opacity: 0, scale: 0.95, transition: { duration: 0.15 } },
                        hover: { x: 2, y: 2, opacity: 1, scale: 1, transition: { ...cascadeTransition, delay: 0.05 } }
                    }}
                >
                    <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />
                    <path d="M14 2v4a2 2 0 0 0 2 2h4" />
                </motion.g>

                {/* --- LAYER 3: MAIN FOREGROUND FILE --- */}
                <motion.g
                    // Tailwind handles color transitions smoothly to prevent interpolation bugs
                    className={cn(
                        "transition-colors duration-300",
                        isHovered
                            ? "stroke-amber-500 fill-amber-50 dark:stroke-amber-400 dark:fill-amber-950/50"
                            : "stroke-currentColor fill-transparent"
                    )}
                    variants={{
                        initial: { x: 0, y: 0, transition: returnTransition },
                        // Lifts up and left while the others fan out down and right
                        hover: { x: -2, y: -2, transition: cascadeTransition }
                    }}
                >
                    {/* Document Outline */}
                    <path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z" />

                    {/* Folded Corner */}
                    <motion.path
                        d="M14 2v4a2 2 0 0 0 2 2h4"
                        className={cn(
                            "transition-colors duration-300",
                            isHovered ? "fill-amber-200/50 dark:fill-amber-900/50" : "fill-transparent"
                        )}
                    />

                    {/* BASE STATE LINES: Standard text lines that fade away on hover */}
                    <motion.g variants={{ initial: { opacity: 1 }, hover: { opacity: 0, transition: { duration: 0.1 } } }}>
                        {/* Drawn left to right (x=8 -> x=16) */}
                        <path d="M8 9h2" />
                        <path d="M8 13h8" />
                        <path d="M8 17h8" />
                    </motion.g>

                    {/* HOVER STATE LINES: Glowing lines that type themselves out */}
                    <motion.g
                        className="stroke-amber-600 dark:stroke-amber-400"
                        variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
                    >
                        <motion.path d="M8 9h2" variants={{ initial: { pathLength: 0 }, hover: { pathLength: 1, transition: { duration: 0.15, delay: 0.1, ease: "linear" } } }} />
                        <motion.path d="M8 13h8" variants={{ initial: { pathLength: 0 }, hover: { pathLength: 1, transition: { duration: 0.25, delay: 0.25, ease: "linear" } } }} />
                        <motion.path d="M8 17h8" variants={{ initial: { pathLength: 0 }, hover: { pathLength: 1, transition: { duration: 0.25, delay: 0.5, ease: "linear" } } }} />
                    </motion.g>

                    {/* THE GLOWING CURSORS: Physically traces the lines as they type */}
                    <motion.rect
                        y="8" width="1.5" height="2"
                        className="fill-amber-500 dark:fill-amber-300 stroke-none"
                        variants={{
                            initial: { x: 8, opacity: 0 },
                            hover: { x: [8, 10], opacity: [0, 1, 1, 0], transition: { duration: 0.15, delay: 0.1, times: [0, 0.1, 0.9, 1], ease: "linear" } }
                        }}
                    />
                    <motion.rect
                        y="12" width="1.5" height="2"
                        className="fill-amber-500 dark:fill-amber-300 stroke-none"
                        variants={{
                            initial: { x: 8, opacity: 0 },
                            hover: { x: [8, 16], opacity: [0, 1, 1, 0], transition: { duration: 0.25, delay: 0.25, times: [0, 0.1, 0.9, 1], ease: "linear" } }
                        }}
                    />
                    <motion.rect
                        y="16" width="1.5" height="2"
                        className="fill-amber-500 dark:fill-amber-300 stroke-none"
                        variants={{
                            initial: { x: 8, opacity: 0 },
                            hover: { x: [8, 16], opacity: [0, 1, 1, 0], transition: { duration: 0.25, delay: 0.5, times: [0, 0.1, 0.9, 1], ease: "linear" } }
                        }}
                    />
                </motion.g>
            </motion.svg>
        </div>
    );
}