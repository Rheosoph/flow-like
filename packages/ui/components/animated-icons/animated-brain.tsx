"use client";
import { motion } from "framer-motion";
import { useState } from "react";
import { cn } from "../../lib/utils";

/** * Biological Brain to AI Network Transformation
 * On hover: Base outline dims and turns pink. A purple neural network
 * draws outward from the spine, ending in pulsing cyan data nodes.
 */
export function AnimatedBrainIcon({ className }: { className?: string }) {
    const [isHovered, setIsHovered] = useState(false);

    // Reusable variants for the synapse lines
    const synapseVariants = {
        initial: { pathLength: 0, opacity: 0, transition: { duration: 0.2 } },
        hover: { pathLength: 1, opacity: 1, transition: { duration: 0.5, ease: "easeOut", delay: 0.1 } }
    };

    // Reusable variants for the pulsing data nodes
    const nodeVariants = {
        initial: { scale: 0, opacity: 0, transition: { duration: 0.2 } },
        hover: { scale: [0, 1.5, 1], opacity: 1, transition: { type: "spring", stiffness: 300, damping: 12, delay: 0.4 } }
    };

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
            {/* --- LAYER 1: BASE BRAIN OUTLINE --- */}

            {/* Standard outline fades out slightly on hover */}
            <motion.g
                variants={{
                    initial: { opacity: 1, transition: { duration: 0.3 } },
                    hover: { opacity: 0, transition: { duration: 0.3 } }
                }}
            >
                {/* Left hemisphere */}
                <path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18" />
                {/* Right hemisphere */}
                <path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18" />
            </motion.g>

            {/* Glowing Pink outline cross-fades in on hover */}
            <motion.g
                className="stroke-pink-500"
                variants={{
                    initial: { opacity: 0, transition: { duration: 0.3 } },
                    hover: { opacity: 0.4, transition: { duration: 0.3 } }
                }}
            >
                <path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18" />
                <path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18" />
            </motion.g>

            {/* --- LAYER 2: INNER NEURAL NETWORK (Draws on hover) --- */}

            <g className="stroke-purple-500" strokeWidth="1.5">
                {/* Center Stem */}
                <motion.line x1="12" y1="5" x2="12" y2="18" variants={synapseVariants} />

                {/* Left Top Branch */}
                <motion.path d="M12 10 L8 12 L4 12" variants={synapseVariants} />

                {/* Right Top Branch */}
                <motion.path d="M12 8 L17 10 L20 10" variants={synapseVariants} />

                {/* Left Bottom Branch */}
                <motion.path d="M12 14 L8 17" variants={synapseVariants} />

                {/* Right Bottom Branch */}
                <motion.path d="M12 15 L16 18" variants={synapseVariants} />
            </g>

            {/* --- LAYER 3: DATA NODES (Pops in after pathways draw) --- */}

            <g className="fill-cyan-400 stroke-none">
                {/* Terminal Nodes */}
                <motion.circle cx="4" cy="12" r="1.5" variants={nodeVariants} />
                <motion.circle cx="20" cy="10" r="1.5" variants={nodeVariants} />
                <motion.circle cx="8" cy="17" r="1.5" variants={nodeVariants} />
                <motion.circle cx="16" cy="18" r="1.5" variants={nodeVariants} />

                {/* Core/Stem Node (Pink to stand out) */}
                <motion.circle cx="12" cy="5" r="2" className="fill-pink-400" variants={nodeVariants} />
            </g>
        </motion.svg>
    );
}