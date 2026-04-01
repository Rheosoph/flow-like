"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Library to Open Book Transformation
 * Base state: 3 books on a shelf (right book leaning).
 * Hover state: Shelf falls, books tumble, center book flies into a colorful open book.
 */
export function AnimatedLibraryIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

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
            {/* --- STATE 1: THE BOOKSHELF --- */}

            {/* The Shelf */}
            <motion.line
                x1="2" y1="21" x2="22" y2="21"
                variants={{
                    initial: { opacity: 1, y: 0, transition: { duration: 0.3, delay: 0.1 } },
                    hover: { opacity: 0, y: 4, transition: { duration: 0.2 } }
                }}
            />

            {/* Left Book - Standing tall */}
            <motion.rect
                x="4" y="8" width="4" height="13" rx="1"
                style={{ transformOrigin: "6px 21px" }}
                variants={{
                    initial: { rotate: 0, x: 0, opacity: 1, transition: { duration: 0.3, delay: 0.1 } },
                    hover: { rotate: -25, x: -6, opacity: 0, transition: { duration: 0.2 } }
                }}
            />

            {/* Center Book - The one that transforms */}
            <motion.rect
                x="10" y="5" width="4" height="16" rx="1"
                variants={{
                    initial: { y: 0, scale: 1, opacity: 1, transition: { duration: 0.3, delay: 0.1 } },
                    hover: { y: -10, scale: 0.8, opacity: 0, transition: { duration: 0.2 } }
                }}
            />

            {/* Right Book - LEANING to sell the "bookshelf" look */}
            <motion.rect
                x="16" y="8" width="4" height="13" rx="1"
                style={{ transformOrigin: "16px 21px" }}
                variants={{
                    // Rotated -15deg so it rests on the center book
                    initial: { rotate: -15, x: -1, y: 0, opacity: 1, transition: { duration: 0.3, delay: 0.1 } },
                    hover: { rotate: 25, x: 6, y: 0, opacity: 0, transition: { duration: 0.2 } }
                }}
            />


            {/* --- STATE 2: THE FLYING OPEN BOOK --- */}
            <motion.g
                style={{ transformOrigin: "12px 12px" }}
                variants={{
                    initial: {
                        y: 10, scale: 0.5, opacity: 0,
                        transition: { duration: 0.15, ease: "easeIn" }
                    },
                    hover: {
                        y: 0, scale: 1, opacity: 1,
                        transition: { type: "spring", stiffness: 280, damping: 15, delay: 0.1 }
                    }
                }}
            >
                {/* Left Page (Orange) */}
                <path
                    d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"
                    className="fill-orange-100 stroke-orange-500 dark:fill-orange-950 dark:stroke-orange-400"
                    strokeWidth="1.5"
                />

                {/* Right Page (Amber) */}
                <path
                    d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"
                    className="fill-amber-100 stroke-amber-500 dark:fill-amber-950 dark:stroke-amber-400"
                    strokeWidth="1.5"
                />

                {/* Center Spine Line */}
                <line
                    x1="12" y1="7" x2="12" y2="21"
                    className="stroke-orange-600 dark:stroke-orange-400"
                    strokeWidth="1.5"
                />
            </motion.g>
        </motion.svg>
    );
}