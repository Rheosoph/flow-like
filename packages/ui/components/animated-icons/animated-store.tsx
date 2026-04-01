"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * App Grid to "App Cosmos" Transformation
 * On hover: The standard app grid scatters outward, rotates playfully,
 * and lights up with vibrant colors. A compass star blooms in the center
 * to represent "exploration" and discovery.
 */
export function AnimatedExploreAppsIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    // Physics for a snappy but smooth return to the grid
    const returnTransition = { type: "spring", stiffness: 300, damping: 25 };
    // Physics for the playful outward scatter
    const scatterTransition = { type: "spring", stiffness: 300, damping: 15 };

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
            {/* --- THE EXPLORATION STAR (Revealed on hover) --- */}
            <motion.path
                d="M12 3 L13.5 10.5 L21 12 L13.5 13.5 L12 21 L10.5 13.5 L3 12 L10.5 10.5 Z"
                className="fill-indigo-400 stroke-none dark:fill-indigo-500"
                style={{ transformOrigin: "12px 12px" }}
                variants={{
                    // Shrinks and spins away on unhover
                    initial: { scale: 0, rotate: -90, opacity: 0, transition: returnTransition },
                    // Blooms and spins in on hover
                    hover: { scale: 0.65, rotate: 0, opacity: 1, transition: { ...scatterTransition, delay: 0.05 } }
                }}
            />

            {/* --- TOP LEFT APP BLOCK --- */}
            <motion.g
                style={{ transformOrigin: "6.5px 6.5px" }}
                variants={{
                    initial: { x: 0, y: 0, rotate: 0, transition: returnTransition },
                    hover: { x: -2, y: -2, rotate: -12, transition: scatterTransition }
                }}
            >
                <rect x="3" y="3" width="7" height="7" rx="1.5" />
                <motion.rect x="3" y="3" width="7" height="7" rx="1.5"
                    className="fill-cyan-100 stroke-cyan-500 dark:fill-cyan-950 dark:stroke-cyan-400"
                    variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
                />
            </motion.g>

            {/* --- TOP RIGHT APP BLOCK --- */}
            <motion.g
                style={{ transformOrigin: "17.5px 6.5px" }}
                variants={{
                    initial: { x: 0, y: 0, rotate: 0, transition: returnTransition },
                    hover: { x: 2, y: -2, rotate: 12, transition: scatterTransition }
                }}
            >
                <rect x="14" y="3" width="7" height="7" rx="1.5" />
                <motion.rect x="14" y="3" width="7" height="7" rx="1.5"
                    className="fill-fuchsia-100 stroke-fuchsia-500 dark:fill-fuchsia-950 dark:stroke-fuchsia-400"
                    variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
                />
            </motion.g>

            {/* --- BOTTOM LEFT APP BLOCK --- */}
            <motion.g
                style={{ transformOrigin: "6.5px 17.5px" }}
                variants={{
                    initial: { x: 0, y: 0, rotate: 0, transition: returnTransition },
                    hover: { x: -2, y: 2, rotate: 12, transition: scatterTransition }
                }}
            >
                <rect x="3" y="14" width="7" height="7" rx="1.5" />
                <motion.rect x="3" y="14" width="7" height="7" rx="1.5"
                    className="fill-amber-100 stroke-amber-500 dark:fill-amber-950 dark:stroke-amber-400"
                    variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
                />
            </motion.g>

            {/* --- BOTTOM RIGHT APP BLOCK --- */}
            <motion.g
                style={{ transformOrigin: "17.5px 17.5px" }}
                variants={{
                    initial: { x: 0, y: 0, rotate: 0, transition: returnTransition },
                    hover: { x: 2, y: 2, rotate: -12, transition: scatterTransition }
                }}
            >
                <rect x="14" y="14" width="7" height="7" rx="1.5" />
                <motion.rect x="14" y="14" width="7" height="7" rx="1.5"
                    className="fill-emerald-100 stroke-emerald-500 dark:fill-emerald-950 dark:stroke-emerald-400"
                    variants={{ initial: { opacity: 0 }, hover: { opacity: 1 } }}
                />
            </motion.g>

        </motion.svg>
    );
}