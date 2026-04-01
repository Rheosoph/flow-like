"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Dynamic Sidebar Toggle
 * Accepts an `isOpen` prop to seamlessly transition between Open and Closed states.
 * Features a collapsing partition, a shifting center of gravity, and an arrow
 * that dynamically draws a stem and nudges in the direction of the action on hover.
 */
export function AnimatedSidebarIcon({
    className,
    isOpen,
    onClick
}: {
    className?: string;
    isOpen?: boolean;
    onClick?: () => void;
}) {
    // Fallback internal state just in case it's used standalone without a parent controller
    const [internalIsOpen, setInternalIsOpen] = useState(true);
    const [isHovered, setIsHovered] = useState(false);

    const isSidebarOpen = isOpen !== undefined ? isOpen : internalIsOpen;

    const handleToggle = () => {
        if (onClick) onClick();
        else setInternalIsOpen(!internalIsOpen);
    };

    // Physics
    const spring = { type: "spring", stiffness: 400, damping: 25 };
    const snappySpring = { type: "spring", stiffness: 500, damping: 15 };

    // Determine the precise animation state to pass down to the SVG elements
    const currentState = isSidebarOpen
        ? isHovered ? "openHover" : "open"
        : isHovered ? "closedHover" : "closed";

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
                stroke="currentColor" // The base container respects the parent text color
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                animate={currentState}
                initial={isSidebarOpen ? "open" : "closed"}
                className="absolute w-full h-full"
            >
                {/* --- LAYER 1: OUTER FRAME --- */}
                <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />

                {/* --- LAYER 2: THE SIDEBAR FILL --- */}
                {/* Represents the actual sidebar panel. Scales down to 0 when closed. */}
                <motion.path
                    d="M9 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h4z"
                    className={cn(
                        "transition-colors duration-300 stroke-none",
                        isHovered ? "fill-indigo-500/20 dark:fill-indigo-400/20" : "fill-slate-500/10 dark:fill-slate-400/10"
                    )}
                    style={{ transformOrigin: "3px 12px" }}
                    variants={{
                        open: { scaleX: 1, opacity: 1, transition: spring },
                        openHover: { scaleX: 1, opacity: 1, transition: spring },
                        closed: { scaleX: 0, opacity: 0, transition: spring },
                        closedHover: { scaleX: 0, opacity: 0, transition: spring }
                    }}
                />

                {/* --- LAYER 3: THE DIVIDER LINE --- */}
                {/* Slides left to merge seamlessly with the frame border when closed */}
                <motion.line
                    x1="9" y1="3" x2="9" y2="21"
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-indigo-500 dark:stroke-indigo-400" : "stroke-currentColor"
                    )}
                    variants={{
                        open: { x: 0, transition: spring },
                        openHover: { x: 0, transition: spring },
                        closed: { x: -6, opacity: 0, transition: spring },
                        closedHover: { x: -6, opacity: 0, transition: spring }
                    }}
                />

                {/* --- LAYER 4: THE ACTION ARROW --- */}
                <motion.g
                    style={{ transformOrigin: "13.5px 12px" }} // Perfectly centers the 180-deg flip
                    className={cn(
                        "transition-colors duration-300",
                        isHovered ? "stroke-indigo-500 dark:stroke-indigo-400" : "stroke-currentColor"
                    )}
                    variants={{
                        // Default open state (Arrow points Left)
                        open: { x: 0, rotate: 0, transition: spring },
                        // Nudges left to hint at closing
                        openHover: { x: -2, rotate: 0, transition: snappySpring },

                        // Closed state (Arrow translates to true center, flips Right)
                        closed: { x: -1.5, rotate: 180, transition: spring },
                        // Nudges right to hint at opening (-1.5 + 2 = 0.5)
                        closedHover: { x: 0.5, rotate: 180, transition: snappySpring }
                    }}
                >
                    {/* The Base Chevron */}
                    <path d="m15 15-3-3 3-3" />

                    {/* The Dynamic Stem - Draws in on hover to create a full arrow */}
                    <motion.path
                        d="M12 12h5"
                        variants={{
                            open: { pathLength: 0, opacity: 0 },
                            openHover: { pathLength: 1, opacity: 1, transition: { duration: 0.2, ease: "easeOut" } },
                            closed: { pathLength: 0, opacity: 0 },
                            closedHover: { pathLength: 1, opacity: 1, transition: { duration: 0.2, ease: "easeOut" } }
                        }}
                    />
                </motion.g>
            </motion.svg>
        </div>
    );
}