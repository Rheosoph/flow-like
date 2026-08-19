"use client";

import { useEffect, useRef, useState } from "react";
import type { IMessage } from "../../state/global-chat/global-chat-db";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";

/**
 * What the FlowPilot orb is showing.
 *
 * - `idle`     nothing happening
 * - `ready`    the orb wants something from you (an approval, a question, a queued review)
 * - `thinking` researching or loading — reasoning, searching, reading, inspecting
 * - `working`  applying changes or generating code — writing, emitting, committing, executing
 */
export type FlowPilotOrbState = "idle" | "ready" | "thinking" | "working";

export interface OrbStateParams {
	/** Multiplier on the orb's own clock. The shader derives all of its churn from u_time, so
	 *  this is what separates a resting orb from a thinking one. */
	rate: number;
	focus: number;
	/** Pointer-style bulge applied with no pointer present. */
	bulge: number;
	/** Radius of the idle bulge's travel. */
	reach: number;
	scale: number;
	breathe: number;
	/** 0 organic wobble … 1 perfect circle. */
	round: number;
	/** Cog-rim amplitude. */
	teeth: number;
	/** Orbiting satellites. */
	sat: number;
	/** Radians per second of rim rotation. */
	spin: number;
	/** How much the iridescence rides along with the spin. */
	spinMix: number;
}

export const ORB_STATE_PARAMS: Record<FlowPilotOrbState, OrbStateParams> = {
	idle: {
		rate: 0.22,
		focus: 0.05,
		bulge: 0,
		reach: 0,
		scale: 0.93,
		breathe: 0.01,
		round: 0,
		teeth: 0,
		sat: 0,
		spin: 0.05,
		spinMix: 0,
	},
	ready: {
		rate: 0.95,
		focus: 0.5,
		bulge: 0.4,
		reach: 0.32,
		scale: 1,
		breathe: 0.024,
		round: 0.25,
		teeth: 0,
		sat: 0,
		spin: 0.28,
		spinMix: 0.3,
	},
	thinking: {
		rate: 3.1,
		focus: 1.35,
		bulge: 0.1,
		reach: 0.1,
		scale: 1.02,
		breathe: 0.026,
		round: 1,
		teeth: 0,
		sat: 1,
		spin: 2.1,
		spinMix: 1,
	},
	working: {
		rate: 1.1,
		focus: 0.85,
		bulge: 0.14,
		reach: 0.12,
		scale: 0.97,
		breathe: 0.012,
		round: 0.97,
		teeth: 1,
		sat: 0,
		spin: 1.25,
		spinMix: 0.85,
	},
};

export const ORB_TEETH_COUNT = 8;

/**
 * The empty-state mark. Deliberately calmer than `idle` — it is rendered large and stared at
 * while you decide what to type, so the personality lives in its entrance, its pointer
 * attention, and its exit rather than in a loop.
 */
export const ORB_INVITING_PARAMS: OrbStateParams = {
	rate: 0.3,
	focus: 0.16,
	bulge: 0.14,
	reach: 0.3,
	scale: 1,
	breathe: 0.018,
	round: 0.2,
	teeth: 0,
	sat: 0,
	spin: 0.1,
	spinMix: 0.16,
};

/**
 * Where the empty-state mark travels while you are writing to it: rounder, a little fuller, and
 * churning at roughly walking pace. Never reached outright — the mark only gets this far while
 * you are typing steadily, and eases back to `ORB_INVITING_PARAMS` the moment you stop.
 */
export const ORB_COMPOSING_PARAMS: OrbStateParams = {
	rate: 0.85,
	focus: 0.44,
	bulge: 0.34,
	reach: 0.2,
	scale: 1.04,
	breathe: 0.03,
	round: 0.55,
	teeth: 0,
	sat: 0,
	spin: 0.5,
	spinMix: 0.42,
};

const ORB_PARAM_KEYS = Object.keys(
	ORB_INVITING_PARAMS,
) as (keyof OrbStateParams)[];

/** Scratch for `mixOrbParams`, reused so a render loop never allocates. */
const MIXED_PARAMS: OrbStateParams = { ...ORB_INVITING_PARAMS };

/**
 * Blends two poses. The result is a shared object valid only until the next call — render loops
 * use it to fill uniforms in the same frame, and nothing else should hold on to it.
 */
export function mixOrbParams(
	from: OrbStateParams,
	to: OrbStateParams,
	t: number,
): OrbStateParams {
	const k = Math.min(1, Math.max(0, t));
	for (const key of ORB_PARAM_KEYS) {
		MIXED_PARAMS[key] = from[key] + (to[key] - from[key]) * k;
	}
	return MIXED_PARAMS;
}

/**
 * Tools that mutate something or produce code/UI. Anything here is `working`; every other tool
 * is research or loading, so it falls through to `thinking`. Deliberately an allow-list of
 * mutations — a new read-only tool should default to thinking, not claim to be applying changes.
 */
const WORKING_TOOLS = new Set([
	"write_flowscript",
	"patch_flowscript",
	"edit_flowscript",
	"check_flowscript",
	"commit_flowscript",
	"emit_commands",
	"emit_surface",
	"emit_ui",
	"modify_component",
	"execute_event",
	"execute_node",
	"interact_app_page",
]);

/** Tools that hand control back to the user rather than continuing to work. */
const WAITING_TOOLS = new Set(["ask_user"]);

export function classifyOrbTool(
	toolName: string | undefined,
): FlowPilotOrbState {
	if (!toolName) return "thinking";
	const name = toolName.toLowerCase();
	if (WAITING_TOOLS.has(name)) return "ready";
	if (WORKING_TOOLS.has(name)) return "working";
	return "thinking";
}

/** The newest live message belongs to the run the shared composer and orb currently follow. */
export function selectActiveOrbTool(
	streamingMessages: readonly Pick<IMessage, "plan_steps">[],
): string | undefined {
	const steps = streamingMessages.at(-1)?.plan_steps;
	if (!steps?.length) return undefined;
	for (let i = steps.length - 1; i >= 0; i--) {
		if (steps[i].status === "progress") return steps[i].toolName;
	}
	return undefined;
}

/**
 * Derives the orb's state from live assistant activity.
 *
 * Precedence, highest first: a prompt waiting on the user, FlowScript being drafted, the tool
 * currently in progress, then plain streaming (the model is composing, i.e. thinking).
 */
export function useFlowPilotOrbState(): FlowPilotOrbState {
	const isStreaming = useGlobalChatStore((s) => s.isStreaming);
	const toolPrompt = useGlobalChatStore((s) => s.toolPrompt);
	const workspaceStatus = useGlobalChatStore(
		(s) => s.flowscriptWorkspace?.status,
	);
	const activeTool = useGlobalChatStore((s) =>
		selectActiveOrbTool(s.streamingMessages),
	);

	if (toolPrompt) return "ready";
	if (!isStreaming) return "idle";
	// The board agent streams FlowScript as it writes it — that is code generation regardless
	// of which tool frame happens to be open at this instant.
	if (workspaceStatus === "drafting") return "working";
	return classifyOrbTool(activeTool);
}

/**
 * Fires the acknowledge burst whenever the orb's state changes, so a transition is always
 * punctuated. Returns a nonce the orb watches.
 */
export function useOrbAckNonce(state: FlowPilotOrbState): number {
	const [nonce, setNonce] = useState(0);
	const previous = useRef(state);
	useEffect(() => {
		if (previous.current !== state) {
			previous.current = state;
			setNonce((n) => n + 1);
		}
	}, [state]);
	return nonce;
}
