"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Day/Night Theme Transformation
 * Size perfectly locked to 16x16 to match the sidebar grid.
 * Light Mode Hover: Core pulses yellow, rays shoot outward and spin 180deg.
 * Dark Mode Hover: Moon tilts, fills with blue, magical stars twinkle in.
 */
export function AnimatedThemeIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    return (
        <div
            // Locked exactly to w-4 h-4 (16px) to match the other icons perfectly
            className={cn(
                "relative flex items-center justify-center w-4 h-4 cursor-pointer text-slate-800 dark:text-slate-200",
                className
            )}
            onPointerEnter={() => setIsHovered(true)}
            onPointerLeave={() => setIsHovered(false)}
        >
            {/* --- SUN (Visible in Light Mode) --- */}
            <motion.svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                // Forces the unhovered state to precisely match text color
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute transition-all duration-500 scale-100 rotate-0 dark:scale-0 dark:-rotate-90"
            >
                <motion.g style={{ transformOrigin: "12px 12px" }}>
                    {/* Sun Core */}
                    <motion.circle
                        cx="12" cy="12" r="4"
                        variants={{
                            initial: { scale: 1, fill: "transparent", stroke: "currentColor" },
                            hover: {
                                scale: 1.1,
                                fill: "#fde047",
                                stroke: "#f59e0b",
                                transition: { type: "spring", stiffness: 300 }
                            }
                        }}
                    />
                    {/* Sun Rays - Dynamic Burst & Spin */}
                    <motion.g
                        style={{ transformOrigin: "12px 12px" }}
                        variants={{
                            initial: { rotate: 0, scale: 1, stroke: "currentColor" },
                            hover: {
                                rotate: 180,
                                scale: 1.3, // Explodes outward
                                stroke: "#f59e0b",
                                transition: { type: "spring", stiffness: 200, damping: 12 }
                            }
                        }}
                    >
                        <path d="M12 2v2" />
                        <path d="M12 20v2" />
                        <path d="m4.93 4.93 1.41 1.41" />
                        <path d="m17.66 17.66 1.41 1.41" />
                        <path d="M2 12h2" />
                        <path d="M20 12h2" />
                        <path d="m6.34 17.66-1.41 1.41" />
                        <path d="m19.07 4.93-1.41 1.41" />
                    </motion.g>
                </motion.g>
            </motion.svg>

            {/* --- MOON (Visible in Dark Mode) --- */}
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
                className="absolute transition-all duration-500 scale-0 rotate-90 dark:scale-100 dark:rotate-0"
            >
                <motion.g style={{ transformOrigin: "12px 12px" }}>
                    {/* The Crescent Moon */}
                    <motion.path
                        d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"
                        variants={{
                            initial: { rotate: 0, scale: 1, fill: "transparent", stroke: "currentColor" },
                            hover: {
                                rotate: -15,
                                scale: 1.05,
                                fill: "#c7d2fe",
                                stroke: "#818cf8",
                                transition: { type: "spring" }
                            }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    />

                    {/* Big Star (Top Left) */}
                    <motion.path
                        d="M4 4l1 3 3 1-3 1-1 3-1-3-3-1 3-1z"
                        className="fill-indigo-300 stroke-none"
                        variants={{
                            initial: { scale: 0, opacity: 0, rotate: 0 },
                            hover: {
                                scale: [0, 1.2, 0.9],
                                opacity: 1,
                                rotate: 45,
                                transition: { duration: 0.5, delay: 0.1 }
                            }
                        }}
                        style={{ transformOrigin: "5px 6px" }}
                    />

                    {/* Small Star (Top Right) */}
                    <motion.path
                        d="M20 4l.5 1.5 1.5.5-1.5.5-.5 1.5-.5-1.5-1.5-.5 1.5-.5z"
                        className="fill-indigo-200 stroke-none"
                        variants={{
                            initial: { scale: 0, opacity: 0, rotate: 0 },
                            hover: {
                                scale: [0, 1.3, 1],
                                opacity: 1,
                                rotate: -45,
                                transition: { duration: 0.5, delay: 0.2 }
                            }
                        }}
                        style={{ transformOrigin: "20px 5px" }}
                    />
                </motion.g>
            </motion.svg>
        </div>
    );
}