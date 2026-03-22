"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Node Registry Hub Transformation
 * Base: A static line-art hexagon with a dormant core.
 * Hover: The core activates and spins. Two "satellite" data nodes materialize
 * and orbit the central hub on different elliptical paths, glowing cyan.
 */
export function AnimatedNodeRegistryIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    // Smooth physics for elements appearing/disappearing
    const spring = { type: "spring", stiffness: 300, damping: 20 };

    return (
        <div
            className={cn(
                "relative flex items-center justify-center w-5 h-5 cursor-pointer text-slate-800 dark:text-slate-200",
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
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: ORBITING SATELLITE NODES (Visible on hover) --- */}
                <motion.g
                    className="stroke-cyan-500 fill-cyan-100 dark:fill-cyan-900/30"
                    variants={{
                        initial: { opacity: 0, scale: 0 },
                        hover: { opacity: 1, scale: 1, transition: spring }
                    }}
                >
                    {/* Inner Orbit (Faster, Counter-Clockwise) */}
                    <motion.g
                        style={{ transformOrigin: "12px 12px" }}
                        variants={{
                            initial: { rotate: 0 },
                            hover: { rotate: -360, transition: { repeat: Infinity, duration: 3, ease: "linear" } }
                        }}
                    >
                        {/* Offset circle to create the orbit path */}
                        <circle cx="12" cy="5" r="1.5" strokeWidth="1.5" />
                    </motion.g>

                    {/* Outer Orbit (Slower, Clockwise, tilted axis) */}
                    <motion.g
                        style={{ transformOrigin: "12px 12px" }}
                        variants={{
                            initial: { rotate: 45 }, // Start at an angle
                            hover: { rotate: 405, transition: { repeat: Infinity, duration: 5, ease: "linear" } }
                        }}
                    >
                        <circle cx="12" cy="19" r="1.5" strokeWidth="1.5" />
                    </motion.g>
                </motion.g>

                {/* --- LAYER 2: THE CENTRAL REGISTRY HUB --- */}
                <motion.g
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-cyan-500 dark:stroke-cyan-400" : "stroke-currentColor"
                    )}
                >
                    {/* The outer hexagon shell */}
                    <motion.path
                        d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"
                        variants={{
                            initial: { scale: 1 },
                            // Subtle pulse to show it's active
                            hover: { scale: 1.05, transition: { repeat: Infinity, repeatType: "reverse", duration: 1 } }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    />

                    {/* The Inner Core (Database) */}
                    <motion.g
                        variants={{
                            initial: { scale: 0.5, rotate: 0 },
                            // Spins up and expands when activated
                            hover: { scale: 1, rotate: 180, transition: { scale: spring, rotate: { duration: 0.5, ease: "easeOut" } } }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    >
                        <motion.circle
                            cx="12" cy="12" r="4"
                            // Fills with active color on hover
                            className={cn(
                                "transition-colors duration-300 stroke-none",
                                isHovered ? "fill-cyan-500/20 dark:fill-cyan-400/20" : "fill-transparent"
                            )}
                        />
                        {/* Internal structure lines representing data */}
                        <path d="M12 8v8" />
                        <path d="M8 12h8" />
                    </motion.g>
                </motion.g>
            </motion.svg>
        </div>
    );
}