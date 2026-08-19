"use client";

import { AnimatePresence, motion } from "framer-motion";
import { usePathname } from "next/navigation";
import { createPortal } from "react-dom";
import { useFabBubbleVisible } from "../../state/fab-bubble";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { FlowPilotBubbleOrb } from "./flowpilot-bubble-orb";
import { useFlowPilotOrbState, useOrbAckNonce } from "./flowpilot-orb-state";

const MotionFlowPilotBubbleOrb = motion.create(FlowPilotBubbleOrb);

// The film says what it is doing; screen readers need the same information in words.
const ORB_STATE_LABEL: Record<string, string> = {
	idle: "Ask FlowPilot",
	ready: "FlowPilot needs your input",
	thinking: "FlowPilot is researching",
	working: "FlowPilot is applying changes",
};

// Routes that already surface FlowPilot themselves, so the floating launcher would be redundant
// even if something they embed requests it (the chat renders inline app surfaces, and the start
// page hosts the full hero bubble).
const HIDDEN_ROUTE_PREFIXES = ["/chat"];

/**
 * The small round FlowPilot launcher docked bottom-right. Clicking it opens the docked overlay —
 * the same conversation the hero bubble and /chat share — which balloons out of this corner so the
 * bubble reads as morphing into the chat. Desktop only.
 *
 * Visibility is opt-in: only surfaces that call `useRequestFabBubble()` get it (see fab-bubble.ts),
 * so it no longer floats over screens it has nothing to do with.
 */
export function FlowPilotBubbleButton() {
	const pathname = usePathname();
	const mode = useGlobalChatStore((state) => state.mode);
	const openOverlay = useGlobalChatStore((state) => state.openOverlay);
	const visible = useFabBubbleVisible();
	// On the surfaces that request it the launcher is the only FlowPilot surface on screen, so it is
	// where the assistant's activity has to be legible.
	const orbState = useFlowPilotOrbState();
	const ackNonce = useOrbAckNonce(orbState);

	// Hide unless a mounted surface asked for it, while the docked overlay itself is open, and on the
	// routes that are their own FlowPilot entry point.
	const hidden =
		!visible ||
		mode === "overlay" ||
		pathname === "/" ||
		HIDDEN_ROUTE_PREFIXES.some(
			(prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`),
		);

	const content = (
		<AnimatePresence>
			{!hidden && (
				<MotionFlowPilotBubbleOrb
					onClick={openOverlay}
					orbState={orbState}
					ackNonce={ackNonce}
					aria-label={ORB_STATE_LABEL[orbState]}
					title={ORB_STATE_LABEL[orbState]}
					initial={{ opacity: 0, scale: 0.3 }}
					animate={{ opacity: 1, scale: 1 }}
					exit={{ opacity: 0, scale: 0.5, transition: { duration: 0.12 } }}
					transition={{ type: "spring", stiffness: 360, damping: 22 }}
					whileHover={{ scale: 1.08 }}
					whileTap={{ scale: 0.92 }}
					className="fixed bottom-6 right-6 z-9998 hidden rounded-full outline-none focus-visible:ring-2 focus-visible:ring-primary/40 md:block"
				/>
			)}
		</AnimatePresence>
	);

	if (typeof document === "undefined") return content;
	return createPortal(content, document.body);
}
