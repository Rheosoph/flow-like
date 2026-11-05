"use client";

import {
	AnimatePresence,
	motion,
	useMotionValue,
	useReducedMotion,
	useSpring,
	useTransform,
	useVelocity,
} from "framer-motion";
import { usePathname } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useFabBubbleVisible } from "../../state/fab-bubble";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { BOTS_ROUTE } from "../bots-tab/dock-selection";
import { FlowPilotBubbleOrb } from "./flowpilot-bubble-orb";
import { useFlowPilotOrbState, useOrbAckNonce } from "./flowpilot-orb-state";

const MotionFlowPilotBubbleOrb = motion.create(FlowPilotBubbleOrb);
const POSITION_KEY = "flowpilot-bubble-position";
const BUBBLE_SIZE = 72;
const SCREEN_INSET = 24;
const WOBBLE_SPRING = { stiffness: 380, damping: 15, mass: 0.6 };
const clamp = (value: number, min: number, max: number) =>
	Math.min(max, Math.max(min, value));

type BubbleBounds = { right: number; bottom: number };

function relativePosition(x: number, y: number, bounds: BubbleBounds) {
	return {
		x: bounds.right ? clamp(x / bounds.right, 0, 1) : 1,
		y: bounds.bottom ? clamp(y / bounds.bottom, 0, 1) : 1,
	};
}

// The film says what it is doing; screen readers need the same information in words.
const ORB_STATE_LABEL: Record<string, string> = {
	idle: "Ask FlowPilot",
	ready: "FlowPilot needs your input",
	thinking: "FlowPilot is researching",
	working: "FlowPilot is applying changes",
};

// The Bots tab already provides FlowPilot. Home requests the launcher when its layout has no assistant widget.
const HIDDEN_ROUTE_PREFIXES = [BOTS_ROUTE];

/**
 * The movable FlowPilot launcher. Clicking opens the same conversation as the hero bubble and
 * the Bots tab in the docked overlay. Its position is remembered across visits. Desktop only.
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
	const reducedMotion = useReducedMotion();
	const x = useMotionValue(0);
	const y = useMotionValue(0);
	const velocityX = useVelocity(x);
	const velocityY = useVelocity(y);
	const stretch = useSpring(
		useTransform(() =>
			reducedMotion
				? 0
				: clamp(
						(Math.abs(velocityX.get()) - Math.abs(velocityY.get())) / 6000,
						-0.14,
						0.14,
					),
		),
		WOBBLE_SPRING,
	);
	const scaleX = useTransform(stretch, (value) => 1 + value);
	const scaleY = useTransform(stretch, (value) => 1 / (1 + value));
	const rotate = useSpring(
		useTransform(() =>
			reducedMotion ? 0 : clamp(velocityX.get() / 180, -10, 10),
		),
		WOBBLE_SPRING,
	);
	const [bounds, setBounds] = useState<BubbleBounds | null>(null);
	const boundsRef = useRef<BubbleBounds>({ right: 0, bottom: 0 });
	const dragBounds = useRef(boundsRef.current);
	const [dragging, setDragging] = useState(false);
	const dragged = useRef(false);
	const pointerOrigin = useRef({ x: 0, y: 0 });

	const rememberPosition = useCallback(() => {
		try {
			localStorage.setItem(
				POSITION_KEY,
				JSON.stringify(relativePosition(x.get(), y.get(), boundsRef.current)),
			);
		} catch {
			// The launcher remains movable when browser storage is unavailable.
		}
	}, [x, y]);

	const constrainAfterResize = () => {
		// Motion caches constraints for a gesture. A window resize invalidates that cache.
		if (dragBounds.current === boundsRef.current) return;
		x.jump(clamp(x.get(), 0, boundsRef.current.right));
		y.jump(clamp(y.get(), 0, boundsRef.current.bottom));
	};

	useEffect(() => {
		let position = { x: 1, y: 1 };
		try {
			const saved = JSON.parse(localStorage.getItem(POSITION_KEY) ?? "null");
			if (saved && Number.isFinite(saved.x) && Number.isFinite(saved.y)) {
				position = { x: clamp(saved.x, 0, 1), y: clamp(saved.y, 0, 1) };
			}
		} catch {
			// Invalid or unavailable storage falls back to the bottom-right corner.
		}
		const fitToScreen = () => {
			const nextBounds = {
				right: Math.max(0, window.innerWidth - BUBBLE_SIZE - SCREEN_INSET * 2),
				bottom: Math.max(
					0,
					window.innerHeight - BUBBLE_SIZE - SCREEN_INSET * 2,
				),
			};
			x.jump(position.x * nextBounds.right);
			y.jump(position.y * nextBounds.bottom);
			boundsRef.current = nextBounds;
			setBounds(nextBounds);
		};
		const onResize = () => {
			position = relativePosition(x.get(), y.get(), boundsRef.current);
			fitToScreen();
			rememberPosition();
		};
		fitToScreen();
		window.addEventListener("resize", onResize);
		return () => window.removeEventListener("resize", onResize);
	}, [x, y, rememberPosition]);

	// Hide unless a mounted surface asked for it, while the docked overlay itself is open, and on the
	// routes that are their own FlowPilot entry point.
	const hidden =
		!visible ||
		mode === "overlay" ||
		HIDDEN_ROUTE_PREFIXES.some(
			(prefix) => pathname === prefix || pathname.startsWith(`${prefix}/`),
		);

	const content = (
		<AnimatePresence>
			{!hidden && bounds && (
				<motion.div
					className="fixed left-6 top-6 z-9998 hidden size-[72px] md:block"
					style={{ x, y, touchAction: "none", userSelect: "none" }}
					drag
					dragConstraints={{ left: 0, top: 0, ...bounds }}
					dragElastic={reducedMotion ? 0 : 0.08}
					dragMomentum={!reducedMotion}
					dragTransition={{
						power: 0.12,
						timeConstant: 180,
						bounceStiffness: 420,
						bounceDamping: 22,
					}}
					onDragStart={() => {
						dragged.current = true;
						setDragging(true);
					}}
					onDragEnd={() => {
						setDragging(false);
						constrainAfterResize();
						rememberPosition();
					}}
					onDrag={constrainAfterResize}
					onDragTransitionEnd={rememberPosition}
					exit={{ opacity: 0, transition: { duration: 0.12 } }}
				>
					<MotionFlowPilotBubbleOrb
						onPointerDownCapture={(event) => {
							if (event.button !== 0 || !event.isPrimary) return;
							dragBounds.current = boundsRef.current;
							dragged.current = false;
							pointerOrigin.current = { x: event.clientX, y: event.clientY };
							event.currentTarget.setPointerCapture(event.pointerId);
						}}
						onPointerMoveCapture={(event) => {
							if (!event.currentTarget.hasPointerCapture(event.pointerId))
								return;
							if (
								Math.hypot(
									event.clientX - pointerOrigin.current.x,
									event.clientY - pointerOrigin.current.y,
								) >= 3
							) {
								dragged.current = true;
							}
						}}
						onClick={(event) => {
							if (dragged.current && event.detail !== 0) {
								event.preventDefault();
								return;
							}
							openOverlay();
						}}
						onKeyDown={(event) => {
							if (event.altKey || event.ctrlKey || event.metaKey) return;
							const step = event.shiftKey ? 64 : 24;
							const offsets: Record<string, [number, number]> = {
								ArrowLeft: [-step, 0],
								ArrowRight: [step, 0],
								ArrowUp: [0, -step],
								ArrowDown: [0, step],
							};
							const offset = offsets[event.key];
							if (!offset) return;
							event.preventDefault();
							event.stopPropagation();
							x.jump(clamp(x.get() + offset[0], 0, bounds.right));
							y.jump(clamp(y.get() + offset[1], 0, bounds.bottom));
							rememberPosition();
						}}
						orbState={orbState}
						ackNonce={ackNonce}
						aria-label={ORB_STATE_LABEL[orbState]}
						aria-description="Drag to move, or use the arrow keys while focused."
						title={`${ORB_STATE_LABEL[orbState]}. Drag to move`}
						initial={{ opacity: 0, scale: reducedMotion ? 1 : 0.3 }}
						animate={{ opacity: 1, scale: 1 }}
						transition={{ type: "spring", stiffness: 360, damping: 22 }}
						whileHover={{ scale: reducedMotion ? 1 : 1.08 }}
						whileTap={{ scale: reducedMotion ? 1 : 0.92 }}
						style={{
							scaleX: reducedMotion ? 1 : scaleX,
							scaleY: reducedMotion ? 1 : scaleY,
							rotate: reducedMotion ? 0 : rotate,
							cursor: dragging ? "grabbing" : "grab",
						}}
					/>
				</motion.div>
			)}
		</AnimatePresence>
	);

	if (typeof document === "undefined") return content;
	return createPortal(content, document.body);
}
