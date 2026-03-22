"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Mechanical Settings Gear Transformation
 * Fix: Replaced linear spin with a robotic, 45-degree ratcheting "tick".
 * Outer gear ticks clockwise; inner core shrinks and ticks counter-clockwise.
 */
export function AnimatedSettingsIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    // This creates the heavy, robotic "snap and lock" movement
    const ratchetTransition = {
        duration: 0.4,
        ease: "backInOut", // Pulls back slightly, snaps forward, and locks
        repeat: Infinity,
        repeatDelay: 0.3 // The mechanical pause between ticks
    };

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
                stroke="currentColor" // Ensures visibility in base state
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={isHovered ? "hover" : "initial"}
                initial="initial"
                className="absolute w-full h-full"
            >
                {/* --- OUTER GEAR (Ticks Clockwise) --- */}
                <motion.g
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-slate-500 dark:stroke-slate-400" : "stroke-currentColor"
                    )}
                    variants={{
                        initial: { rotate: 0 },
                        hover: {
                            // 45 degrees is exactly one tooth. This guarantees a seamless loop.
                            rotate: [0, 45],
                            transition: ratchetTransition
                        }
                    }}
                    style={{ transformOrigin: "12px 12px" }}
                >
                    <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
                </motion.g>

                {/* --- INNER CORE (Ticks Counter-Clockwise) --- */}
                <motion.g
                    className={cn(
                        "transition-colors duration-300",
                        isHovered
                            ? "stroke-cyan-500 fill-cyan-500/20 dark:stroke-cyan-400 dark:fill-cyan-400/20"
                            : "stroke-currentColor fill-transparent"
                    )}
                    variants={{
                        initial: { rotate: 0, scale: 1 },
                        hover: {
                            // Ticks in the opposite direction
                            rotate: [0, -45],
                            scale: 0.6,
                            transition: {
                                rotate: ratchetTransition,
                                // Scale happens once and stays there, doesn't loop
                                scale: { type: "spring", stiffness: 300, damping: 20 }
                            }
                        }
                    }}
                    style={{ transformOrigin: "12px 12px" }}
                >
                    <circle cx="12" cy="12" r="3" />

                    {/* Inner Axle Dot (Pops in on hover) */}
                    <motion.circle
                        cx="12" cy="12" r="1.5"
                        className="fill-cyan-500 dark:fill-cyan-400 stroke-none"
                        variants={{
                            initial: { scale: 0, opacity: 0 },
                            hover: { scale: 1, opacity: 1, transition: { type: "spring", delay: 0.1 } }
                        }}
                        style={{ transformOrigin: "12px 12px" }}
                    />
                </motion.g>
            </motion.svg>
        </div>
    );
}