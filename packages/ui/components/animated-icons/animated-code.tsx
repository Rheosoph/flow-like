"use client";

import { motion } from "framer-motion";
import { useState, useId } from "react";
import { cn } from "../../lib/utils";

/**
 * Morphing Glowing Live Code Workstation
 * Base state: Exact visual match to standard code brackets and slash (</>).
 * Hover state:
 * 1. Direct d-attribute morph makes brackets become a monitor frame and
 * the slash melt into a detailed mechanical keyboard.
 * 2. An SVG blur filter creates a soft teal-cyan "bloom" emanating from the
 * screen's center.
 * 3. Colorful, glowing code lines fade in and pulse infinitely.
 */
export function AnimatedCodeIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);
    // Generates a unique ID for the SVG filter so multiple icons don't conflict
    const id = useId();

    // Highly fluid spring to make the topological morph look like liquid magic
    const morphSpring = { type: "spring", stiffness: 220, damping: 20 };
    const fadeSpring = { type: "spring", stiffness: 300, damping: 25 };

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
            // Crucial: allow the soft halo bloom to break the viewBox boundary
            style={{ overflow: "visible" }}
            className={cn("cursor-pointer text-slate-800 dark:text-slate-200", className)}
        >
            <defs>
                {/* Filter for monitor glow/bloom.
                  x,y,width,height define the canvas for the filter.
                  I make it much larger than the 24x24 viewBox to allow the bloom to "blur out".
                */}
                <filter id={`bloom-${id}`} x="-10" y="-10" width="44" height="44" filterUnits="userSpaceOnUse">
                    {/* The Blur: Creates the soft halo */}
                    <feGaussianBlur in="SourceGraphic" stdDeviation="2.5" result="blur" />
                    {/* The Composite: Merges the sharp original over the blurred halo */}
                    <feMerge>
                        <feMergeNode in="blur" />
                        <feMergeNode in="SourceGraphic" />
                    </feMerge>
                </filter>
            </defs>

            {/* --- LAYER 1: DATA SPARKS (Compiling effect) --- */}
            <motion.circle
                cx="12" cy="12" r="1.5"
                className="fill-purple-400 stroke-none dark:fill-purple-500"
                variants={{
                    initial: { y: 0, scale: 0, opacity: 0 },
                    hover: {
                        y: -14, x: -8, scale: 1, opacity: [0, 1, 0],
                        transition: { duration: 1.5, ease: "easeOut", repeat: Infinity, delay: 0.3 }
                    }
                }}
            />
            <motion.circle
                cx="12" cy="12" r="1"
                className="fill-blue-400 stroke-none dark:fill-blue-500"
                variants={{
                    initial: { y: 0, scale: 0, opacity: 0 },
                    hover: {
                        y: -16, x: 6, scale: 1.5, opacity: [0, 1, 0],
                        transition: { duration: 1.8, ease: "easeOut", repeat: Infinity, delay: 0.6 }
                    }
                }}
            />

            {/* --- LAYER 2: THE MORPHING MONITOR --- */}

            {/* 2.1 Inner Dark Screen Base (Fades in first for depth) */}
            <motion.rect
                x="3" y="4" width="18" height="11" rx="0.5"
                className="fill-slate-900 stroke-none dark:fill-slate-950"
                variants={{
                    initial: { opacity: 0, scale: 0.8 },
                    hover: { opacity: 1, scale: 1, transition: { delay: 0.1, ...fadeSpring } }
                }}
            />

            {/* 2.2 Glow/Bloom layer.
              Matches screen shape, uses blur filter and soft teal-cyan color.
            */}
            <motion.rect
                x="3" y="4" width="18" height="11" rx="0.5"
                filter={`url(#bloom-${id})`}
                className="fill-cyan-400/60 stroke-none dark:fill-cyan-500/70"
                variants={{
                    initial: { opacity: 0, scale: 0.8 },
                    hover: { opacity: 1, scale: 1, transition: { delay: 0.2, ...fadeSpring } }
                }}
            />

            {/* 2.3 LEFT BRACKET (<) morphs into TOP & LEFT Monitor Borders */}
            <motion.path
                variants={{
                    initial: { d: "M 6 8 L 2 12 L 6 16" },
                    hover: { d: "M 22 3 L 2 3 L 2 16" }
                }}
                transition={morphSpring}
                className="fill-none"
            />

            {/* 2.4 RIGHT BRACKET (>) morphs into BOTTOM & RIGHT Monitor Borders */}
            <motion.path
                variants={{
                    initial: { d: "M 18 16 L 22 12 L 18 8" },
                    hover: { d: "M 2 16 L 22 16 L 22 3" }
                }}
                transition={morphSpring}
                className="fill-none"
            />

            {/* 2.5 Glowing Code Lines inside Monitor (Pulses infinitely) */}
            <motion.g
                variants={{
                    initial: { opacity: 0 },
                    // Stagger delay after glow. Adds energy with a subtle pulse.
                    hover: { opacity: [1, 0.7, 1], transition: { delay: 0.4, duration: 2, repeat: Infinity } }
                }}
                strokeWidth="1.2"
                strokeLinecap="round"
            >
                <line x1="5" y1="6.5" x2="10" y2="6.5" className="stroke-pink-500" />
                <line x1="5" y1="9" x2="16" y2="9" className="stroke-emerald-400" />
                <line x1="7" y1="11.5" x2="13" y2="11.5" className="stroke-blue-400" />
            </motion.g>

            {/* --- LAYER 3: MONITOR STAND --- */}
            <motion.g
                style={{ transformOrigin: "12px 16px" }}
                variants={{
                    initial: { scaleY: 0, opacity: 0 },
                    hover: { scaleY: 1, opacity: 1, transition: { delay: 0.15, ...fadeSpring } }
                }}
            >
                <path d="M 10 16 V 19 M 14 16 V 19" className="stroke-slate-400 dark:stroke-slate-500" />
                <path d="M 8 19 H 16" className="stroke-slate-800 dark:stroke-slate-300" strokeWidth="2" />
            </motion.g>

            {/* --- LAYER 4: THE MORPHING KEYBOARD --- */}
            {/* THE SLASH (/) morphs into the KEYBOARD OUTER FRAME
                We use 5 points (M + 3 L + Z) for both states so Framer Motion maps them perfectly.
            */}
            <motion.path
                variants={{
                    // Standard angled line coordinates
                    initial: { d: "M 14.5 4 L 14.5 4 L 9.5 20 L 9.5 20 Z" },
                    // Folds wide open. Points map to top and bottom corners.
                    hover: { d: "M 2 20 L 22 20 L 22 24 L 2 24 Z" }
                }}
                transition={morphSpring}
                className="fill-none"
            />

            {/* Keyboard Inner Keys (Fades in once keyboard morphs) */}
            <motion.path
                d="M 4 21.5 h 16 M 4 23 h 16"
                strokeDasharray="2 1.5"
                className="stroke-slate-400 dark:stroke-slate-500"
                strokeWidth="1"
                variants={{
                    initial: { pathLength: 0, opacity: 0 },
                    hover: { pathLength: 1, opacity: 1, transition: { delay: 0.25, duration: 0.4 } }
                }}
            />
        </motion.svg>
    );
}