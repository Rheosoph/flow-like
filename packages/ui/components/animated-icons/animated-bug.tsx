"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

export function AnimatedBugIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

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
                stroke="currentColor" // <-- RESTORED: This makes it visible!
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: CYAN CONTAINMENT RINGS --- */}
                <motion.g
                    className="stroke-cyan-400 dark:stroke-cyan-500"
                    strokeWidth="1.5"
                    variants={{
                        initial: { opacity: 0, scale: 0 },
                        hover: { opacity: 1, scale: 1, transition: { type: "spring", stiffness: 200, damping: 12 } }
                    }}
                >
                    <motion.ellipse
                        cx="12" cy="12" rx="5" ry="9"
                        variants={{
                            initial: { rotate: 45 },
                            hover: { rotate: [45, -315], transition: { repeat: Infinity, duration: 4, ease: "linear" } }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    />
                    <motion.ellipse
                        cx="12" cy="12" rx="5" ry="9"
                        variants={{
                            initial: { rotate: -45 },
                            hover: { rotate: [-45, 315], transition: { repeat: Infinity, duration: 4, ease: "linear" } }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    />
                </motion.g>

                {/* --- LAYER 2: THE BUG BODY --- */}
                <motion.g
                    className={cn(
                        "transition-colors duration-300",
                        isHovered
                            ? "stroke-red-500 fill-red-900/40 dark:fill-red-950/60"
                            : "stroke-currentColor fill-transparent"
                    )}
                >
                    {/* Body Shell */}
                    <motion.path
                        d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6"
                        variants={{
                            initial: { scaleX: 1, scaleY: 1 },
                            hover: { scaleX: 1.15, scaleY: 1.05, transition: { type: "spring", stiffness: 400, damping: 10 } }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    />
                    {/* Head */}
                    <path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1" />

                    {/* Center Spine */}
                    <motion.path
                        d="M12 20v-9"
                        className={cn("transition-colors duration-300", isHovered ? "stroke-red-300 dark:stroke-red-400" : "stroke-currentColor")}
                        variants={{
                            initial: { strokeWidth: 2 },
                            hover: { strokeWidth: 3 }
                        }}
                    />
                </motion.g>

                {/* --- LAYER 3: LEGS & ANTENNAE --- */}
                <motion.g
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-red-500" : "stroke-currentColor"
                    )}
                >
                    {/* Antennae */}
                    <motion.path d="m8 2 1.88 1.88" style={{ transformOrigin: "9.88px 3.88px" }} variants={{ initial: { rotate: 0 }, hover: { rotate: -15, scale: 1.2, transition: { type: "spring" } } }} />
                    <motion.path d="M14.12 3.88 16 2" style={{ transformOrigin: "14.12px 3.88px" }} variants={{ initial: { rotate: 0 }, hover: { rotate: 15, scale: 1.2, transition: { type: "spring" } } }} />

                    {/* Top Legs */}
                    <motion.path d="M6.53 9C4.6 8.8 3 7.1 3 5" style={{ transformOrigin: "6.53px 9px" }} variants={{ initial: { rotate: 0 }, hover: { rotate: -25, scaleX: 1.3, transition: { type: "spring" } } }} />
                    <motion.path d="M17.47 9c1.93-.2 3.53-1.9 3.53-4" style={{ transformOrigin: "17.47px 9px" }} variants={{ initial: { rotate: 0 }, hover: { rotate: 25, scaleX: 1.3, transition: { type: "spring" } } }} />

                    {/* Middle Legs */}
                    <motion.path d="M8 14H4" style={{ transformOrigin: "8px 14px" }} variants={{ initial: { scaleX: 1 }, hover: { scaleX: 1.4, transition: { type: "spring" } } }} />
                    <motion.path d="M16 14h4" style={{ transformOrigin: "16px 14px" }} variants={{ initial: { scaleX: 1 }, hover: { scaleX: 1.4, transition: { type: "spring" } } }} />

                    {/* Bottom Legs */}
                    <motion.path d="M9.5 19c-1.7 1.5-3 2-5 2" style={{ transformOrigin: "9.5px 19px" }} variants={{ initial: { rotate: 0 }, hover: { rotate: 20, scaleX: 1.3, transition: { type: "spring" } } }} />
                    <motion.path d="M14.5 19c1.7 1.5 3 2 5 2" style={{ transformOrigin: "14.5px 19px" }} variants={{ initial: { rotate: 0 }, hover: { rotate: -20, scaleX: 1.3, transition: { type: "spring" } } }} />
                </motion.g>
            </motion.svg>
        </div>
    );
}