"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Authentic Lucide Pin with Floating Anticipation
 * Fix 1: Added overflow="visible" to the SVG so the hover lift doesn't clip.
 * Fix 2: Added a permanent vibrant tint to the needle tip.
 */
export function AnimatedPinIcon({ className, onPinToggle }: { className?: string, onPinToggle?: (isPinned: boolean) => void }) {
    const [isPinned, setIsPinned] = useState(false);
    const [isHovered, setIsHovered] = useState(false);

    const handleToggle = () => {
        const newState = !isPinned;
        setIsPinned(newState);
        if (onPinToggle) onPinToggle(newState);
    };

    return (
        <div
            className={cn(
                "relative flex items-center justify-center w-5 h-5 cursor-pointer text-slate-800 dark:text-slate-200",
                className
            )}
            onPointerEnter={() => setIsHovered(true)}
            onPointerLeave={() => setIsHovered(false)}
            onClick={handleToggle}
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
                animate={isPinned ? "pinned" : isHovered ? "hover" : "initial"}
                initial="initial"
                // THIS FIXES THE CLIPPING BUG
                style={{ overflow: "visible" }}
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: IMPACT SHOCKWAVE --- */}
                <motion.circle
                    cx="12" cy="20" r="6"
                    className="stroke-violet-500 dark:stroke-violet-400"
                    strokeWidth="1.5"
                    variants={{
                        initial: { scale: 0, opacity: 0 },
                        hover: { scale: 0, opacity: 0 },
                        pinned: {
                            scale: [0.3, 2],
                            opacity: [0, 1, 0],
                            transition: { duration: 0.5, ease: "easeOut" }
                        }
                    }}
                />

                {/* --- LAYER 2: THE LUCIDE PIN --- */}
                <motion.g
                    style={{ transformOrigin: "12px 20px" }} // Pivot point at the needle tip
                    variants={{
                        // 1. Base State: Resting at an angle
                        initial: {
                            y: 0,
                            rotate: 40,
                            scaleY: 1,
                            transition: { type: "spring", stiffness: 300, damping: 20 }
                        },
                        // 2. Hover State: Lifted and floating (won't clip anymore)
                        hover: {
                            y: [-4, -6, -4],
                            rotate: [-5, 5, -5],
                            scaleY: 1,
                            transition: {
                                y: { repeat: Infinity, duration: 2, ease: "easeInOut" },
                                rotate: { repeat: Infinity, duration: 2.5, ease: "easeInOut" },
                                scaleY: { type: "spring" }
                            }
                        },
                        // 3. Pinned State: The Heavy Strike
                        pinned: {
                            y: 2,
                            rotate: 0,
                            scaleY: [1, 0.6, 1], // The aggressive impact squash
                            transition: { type: "spring", stiffness: 600, damping: 12 }
                        }
                    }}
                >
                    {/* The Needle Tip - Permanently colored so it stands out */}
                    <motion.g className="stroke-violet-500 dark:stroke-violet-400">
                        <path d="M12 17v5" />
                        {/* Tiny glowing point at the very bottom of the needle */}
                        <circle cx="12" cy="22" r="0.5" className="fill-violet-500 dark:fill-violet-400 stroke-none" />
                    </motion.g>

                    {/* The Authentic Chunky Lucide Head */}
                    <motion.path
                        d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"
                        className={cn(
                            "transition-all duration-300",
                            isPinned
                                ? "fill-violet-500 stroke-violet-600 dark:fill-violet-400 dark:stroke-violet-400"
                                : "fill-transparent stroke-currentColor"
                        )}
                    />
                </motion.g>
            </motion.svg>
        </div>
    );
}