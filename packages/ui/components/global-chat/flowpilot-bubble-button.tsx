"use client";

import { AnimatePresence, motion } from "framer-motion";
import { usePathname } from "next/navigation";
import { createPortal } from "react-dom";
import { useFabBubbleSuppressed } from "../../state/fab-suppression";
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

// Routes that already surface FlowPilot themselves, so the floating launcher would be redundant.
// The board/widget builders deliberately use the bubble instead of their own in-interface button, so
// they are NOT listed here — only the full chat view is.
const HIDDEN_ROUTE_PREFIXES = ["/chat"];

/**
 * The small round FlowPilot launcher docked bottom-right on every page except the start page (which
 * hosts the full hero bubble) and /chat (which IS the assistant). Clicking it opens the docked
 * overlay — the same conversation the hero bubble and /chat share — which balloons out of this corner
 * so the bubble reads as morphing into the chat. Desktop only.
 */
export function FlowPilotBubbleButton() {
	const pathname = usePathname();
	const mode = useGlobalChatStore((state) => state.mode);
	const openOverlay = useGlobalChatStore((state) => state.openOverlay);
	const suppressed = useFabBubbleSuppressed();
	// The launcher is the only FlowPilot surface visible while you work elsewhere in the app,
	// so it is where the assistant's activity has to be legible.
	const orbState = useFlowPilotOrbState();
	const ackNonce = useOrbAckNonce(orbState);

	// Hide where a FlowPilot entry point already exists: the hero bubble (start page), the full chat
	// view, and while the docked overlay itself is open. Also hide while a component claims the
	// bottom-right corner (e.g. the FlowScript panel's Apply button) so they don't overlap.
	const hidden =
		pathname === "/" ||
		mode === "overlay" ||
		suppressed ||
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
