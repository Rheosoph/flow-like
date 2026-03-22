"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

export function AnimatedNewProjectIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    const dockTransition = { type: "spring", stiffness: 400, damping: 14 };
    const retreatTransition = { type: "spring", stiffness: 300, damping: 20 };

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
                stroke="currentColor" // <-- RESTORED: This makes the base plus visible!
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: THE SHOCKWAVE --- */}
                <motion.circle
                    cx="12" cy="12" r="10"
                    className="stroke-emerald-400 dark:stroke-emerald-500"
                    strokeWidth="1.5"
                    variants={{
                        initial: { scale: 0, opacity: 0 },
                        hover: {
                            scale: 1.2,
                            opacity: [0, 0.8, 0],
                            transition: { duration: 0.5, delay: 0.25, ease: "easeOut" }
                        }
                    }}
                />

                {/* --- LAYER 2: THE PROJECT MODULES --- */}
                <g strokeWidth="1.5">
                    {/* Indigo Diamond */}
                    <motion.g
                        className="stroke-indigo-500 fill-indigo-500/20 dark:stroke-indigo-400 dark:fill-indigo-400/20"
                        variants={{
                            initial: { x: -10, y: -10, scale: 0.5, opacity: 0, transition: retreatTransition },
                            hover: { x: 0, y: 0, scale: 1, opacity: 1, transition: { ...dockTransition, delay: 0.05 } }
                        }}
                    >
                        <path d="M 6.5 4 L 9 6.5 L 6.5 9 L 4 6.5 Z" />
                    </motion.g>

                    {/* Cyan Circle */}
                    <motion.g
                        className="stroke-cyan-500 fill-cyan-500/20 dark:stroke-cyan-400 dark:fill-cyan-400/20"
                        variants={{
                            initial: { x: 10, y: -10, scale: 0.5, opacity: 0, transition: retreatTransition },
                            hover: { x: 0, y: 0, scale: 1, opacity: 1, transition: { ...dockTransition, delay: 0.1 } }
                        }}
                    >
                        <circle cx="17.5" cy="6.5" r="2.5" />
                    </motion.g>

                    {/* Amber Square */}
                    <motion.g
                        className="stroke-amber-500 fill-amber-500/20 dark:stroke-amber-400 dark:fill-amber-400/20"
                        variants={{
                            initial: { x: 10, y: 10, scale: 0.5, opacity: 0, transition: retreatTransition },
                            hover: { x: 0, y: 0, scale: 1, opacity: 1, transition: { ...dockTransition, delay: 0.15 } }
                        }}
                    >
                        <rect x="15" y="15" width="5" height="5" rx="1" />
                    </motion.g>

                    {/* Fuchsia Triangle */}
                    <motion.g
                        className="stroke-fuchsia-500 fill-fuchsia-500/20 dark:stroke-fuchsia-400 dark:fill-fuchsia-400/20"
                        variants={{
                            initial: { x: -10, y: 10, scale: 0.5, opacity: 0, transition: retreatTransition },
                            hover: { x: 0, y: 0, scale: 1, opacity: 1, transition: { ...dockTransition, delay: 0.2 } }
                        }}
                    >
                        <path d="M 6.5 14 L 9 19 H 4 Z" />
                    </motion.g>
                </g>

                {/* --- LAYER 3: THE CORE PLUS SIGN --- */}
                <motion.g
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-emerald-500 dark:stroke-emerald-400" : "stroke-currentColor"
                    )}
                    variants={{
                        initial: { rotate: 0, scale: 1, transition: retreatTransition },
                        hover: { rotate: 180, scale: 1.1, transition: { type: "spring", stiffness: 300, damping: 15 } }
                    }}
                    style={{ transformOrigin: "12px 12px" }}
                >
                    <path d="M5 12h14" />
                    <path d="M12 5v14" />
                </motion.g>
            </motion.svg>
        </div>
    );
}