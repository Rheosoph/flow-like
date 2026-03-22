"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Synchronized Data Flow Transformation
 * Base: Standard monochrome nodes and wire.
 * Hover: The wire dims, and a glowing data packet shoots along the path.
 * The Source node "pumps" when firing, and the Destination node "catches"
 * and pulses precisely when the data packet arrives.
 */
export function AnimatedFlowsIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    // The duration of one complete data transfer loop
    const loopDuration = 1.5;

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
                stroke="currentColor" // Ensures perfect base state visibility
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: THE WIRE --- */}
                {/* Base wire fades back slightly on hover to let the data glow */}
                <motion.path
                    d="M7 11v4a2 2 0 0 0 2 2h4"
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-slate-300 dark:stroke-slate-700" : "stroke-currentColor"
                    )}
                />

                {/* The Glowing Data Packet (Only visible on hover) */}
                <motion.path
                    d="M7 11v4a2 2 0 0 0 2 2h4"
                    className="stroke-cyan-500 dark:stroke-cyan-400"
                    strokeWidth="2.5"
                    variants={{
                        initial: { pathLength: 0.2, pathOffset: -0.2, opacity: 0 },
                        hover: {
                            // Shoots the dash from before the start to past the end
                            pathOffset: 1,
                            opacity: [0, 1, 1, 0], // Fades in on launch, out on arrival
                            transition: {
                                repeat: Infinity,
                                duration: loopDuration,
                                ease: "linear"
                            }
                        }
                    }}
                />

                {/* --- LAYER 2: SOURCE NODE (Top Left) --- */}
                <motion.g style={{ transformOrigin: "7px 7px" }}>
                    <motion.rect
                        width="8" height="8" x="3" y="3" rx="2"
                        className={cn(
                            "transition-colors duration-300",
                            isHovered
                                ? "stroke-cyan-500 fill-cyan-50 dark:stroke-cyan-400 dark:fill-cyan-950/40"
                                : "stroke-currentColor fill-transparent"
                        )}
                        variants={{
                            initial: { scale: 1 },
                            hover: {
                                // Pulses right at the start of the loop (0s) when data is "fired"
                                scale: [1, 1.2, 1, 1],
                                transition: {
                                    repeat: Infinity,
                                    duration: loopDuration,
                                    times: [0, 0.15, 0.3, 1], // Timing mapped to the data launch
                                    ease: "easeInOut"
                                }
                            }
                        }}
                    />
                    {/* Source Core (Revealed on hover) */}
                    <motion.circle
                        cx="7" cy="7" r="1.5"
                        className="fill-cyan-500 dark:fill-cyan-400 stroke-none"
                        variants={{ initial: { opacity: 0, scale: 0 }, hover: { opacity: 1, scale: 1, transition: { type: "spring" } } }}
                    />
                </motion.g>

                {/* --- LAYER 3: DESTINATION NODE (Bottom Right) --- */}
                <motion.g style={{ transformOrigin: "17px 17px" }}>
                    <motion.rect
                        width="8" height="8" x="13" y="13" rx="2"
                        className={cn(
                            "transition-colors duration-300",
                            // Indigo color to show processing/receiving
                            isHovered
                                ? "stroke-indigo-500 fill-indigo-50 dark:stroke-indigo-400 dark:fill-indigo-950/40"
                                : "stroke-currentColor fill-transparent"
                        )}
                        variants={{
                            initial: { scale: 1 },
                            hover: {
                                // Idles, then pulses at ~70% of the loop when the data packet physically hits it
                                scale: [1, 1, 1.2, 1],
                                transition: {
                                    repeat: Infinity,
                                    duration: loopDuration,
                                    times: [0, 0.65, 0.8, 1], // Timing mapped to the data arrival
                                    ease: "easeInOut"
                                }
                            }
                        }}
                    />
                    {/* Destination Core (Revealed on hover) */}
                    <motion.circle
                        cx="17" cy="17" r="1.5"
                        className="fill-indigo-500 dark:fill-indigo-400 stroke-none"
                        variants={{ initial: { opacity: 0, scale: 0 }, hover: { opacity: 1, scale: 1, transition: { type: "spring" } } }}
                    />
                </motion.g>

            </motion.svg>
        </div>
    );
}