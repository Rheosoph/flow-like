"use client";

import { useCallback, useMemo } from "react";
import {
	evaluateGate,
	evaluateGates,
} from "../../../../lib/device-management/model/gates";
import type {
	ActionId,
	DevicesRoute,
	FixAction,
	GateContext,
	GateResult,
} from "../../../../lib/device-management/model/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { useDeviceAuth } from "./device-workspace-provider";
import { useOverlayStore } from "./overlay-store";
import {
	type GateSources,
	type GateTarget,
	buildGateContext,
	useAttentionState,
} from "./use-attention";

export { buildGateContext };
export type { GateSources, GateTarget };

function useGateContext(deviceId?: string, target?: GateTarget): GateContext {
	const { workspace, input, tokenScopeAll } = useAttentionState();
	const key = target ? JSON.stringify(target) : "";
	// biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for the target's content
	return useMemo(
		() =>
			buildGateContext({ workspace, input, tokenScopeAll }, deviceId, target),
		[workspace, input, tokenScopeAll, deviceId, key],
	);
}

/**
 * Whether one action is allowed now, and if not the first failing gate with
 * its reason and fix (R7: the control stays visible and disabled; `hide` only
 * when it cannot apply to this object).
 */
export function useGate(
	action: ActionId,
	deviceId?: string,
	target?: GateTarget,
): GateResult {
	const context = useGateContext(deviceId, target);
	return useMemo(() => evaluateGate(action, context), [action, context]);
}

export function useGates<A extends ActionId>(
	actions: readonly A[],
	deviceId?: string,
	target?: GateTarget,
): Record<A, GateResult> {
	const context = useGateContext(deviceId, target);
	const key = actions.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for the action list's content
	return useMemo(() => evaluateGates(actions, context), [key, context]);
}

/** What a fix button did: handled here, or where the caller should go. */
export type FixOutcome =
	| { kind: "done" }
	| { kind: "navigate"; route: DevicesRoute }
	| {
			kind: "external";
			what:
				| "sign_in"
				| "use_full_token"
				| "use_desktop"
				| "see_plans"
				| "fix_clock";
	  };

const DONE: FixOutcome = { kind: "done" };

const FIX_ROUTES: {
	[K in FixAction["kind"]]?: (
		fix: Extract<FixAction, { kind: K }>,
	) => DevicesRoute;
} = {
	open_hub_status: () => ({ screen: "hub" }),
	restore_keys: (fix) => ({ screen: "keys", focusDeviceId: fix.deviceId }),
	import_key_file: (fix) => ({ screen: "keys", focusDeviceId: fix.deviceId }),
	request_access: () => ({
		screen: "access",
		tab: "shared",
		action: "request",
	}),
	ask_to_renew: () => ({ screen: "access", tab: "shared" }),
	update_agent: (fix) => ({
		screen: "device",
		deviceId: fix.deviceId,
		tab: "settings",
	}),
	ask_owner: (fix) => ({
		screen: "device",
		deviceId: fix.deviceId,
		tab: "access",
	}),
	forget_identity: (fix) => ({
		screen: "device",
		deviceId: fix.deviceId,
		tab: "keys",
	}),
};

/** The screen a fix leads to, for fixes that are a place and not an action. */
export function fixRoute(fix: FixAction): DevicesRoute | undefined {
	const route = FIX_ROUTES[fix.kind] as
		| ((value: FixAction) => DevicesRoute)
		| undefined;
	return route?.(fix);
}

interface FixContext {
	workspace: DeviceWorkspace;
	signIn?: () => void;
}

type FixRunner<K extends FixAction["kind"]> = (
	fix: Extract<FixAction, { kind: K }>,
	context: FixContext,
) => FixOutcome;

const overlay = () => useOverlayStore.getState();
const external =
	(what: Extract<FixOutcome, { kind: "external" }>["what"]) =>
	(): FixOutcome => ({
		kind: "external",
		what,
	});

/** Fixes that happen in place: overlays, key actions, the host's sign-in. */
const FIX_RUNNERS: { [K in FixAction["kind"]]?: FixRunner<K> } = {
	unlock(fix) {
		overlay().openUnlock(
			fix.deviceId,
			fix.connectLive ? { connectLive: true } : {},
		);
		return DONE;
	},
	review_identity(fix) {
		overlay().openUnlock(fix.deviceId);
		return DONE;
	},
	diagnose(fix) {
		overlay().openDiagnose(fix.deviceId, fix.serviceId);
		return DONE;
	},
	take_over(fix, { workspace }) {
		void workspace.keys.takeOver(fix.deviceId).catch(() => undefined);
		return DONE;
	},
	connect(fix, { workspace }) {
		const { keys } = workspace;
		if (keys.snapshot(fix.deviceId).state !== "unlocked")
			overlay().openUnlock(fix.deviceId, { connectLive: true });
		else
			void keys
				.unlock(fix.deviceId, "", { connectLive: true })
				.catch(() => undefined);
		return DONE;
	},
	keep_keys_safely(_fix, { workspace }) {
		void workspace.local.requestPersistence().catch(() => undefined);
		return DONE;
	},
	sign_in(_fix, { signIn }) {
		if (!signIn) return { kind: "external", what: "sign_in" };
		signIn();
		return DONE;
	},
	use_full_token: external("use_full_token"),
	use_desktop: external("use_desktop"),
	see_plans: external("see_plans"),
	fix_clock: external("fix_clock"),
};

/**
 * Runs the fix of a gate notice, pre-flight row or attention item: overlays
 * and key actions happen here, places come back as a route to navigate to.
 */
export function useFixAction(): (fix: FixAction) => FixOutcome {
	const { workspace } = useAttentionState();
	const { signIn } = useDeviceAuth();
	return useCallback(
		(fix: FixAction): FixOutcome => {
			const run = FIX_RUNNERS[fix.kind] as
				| FixRunner<FixAction["kind"]>
				| undefined;
			if (run) return run(fix, { workspace, ...(signIn ? { signIn } : {}) });
			const route = fixRoute(fix);
			return route ? { kind: "navigate", route } : DONE;
		},
		[workspace, signIn],
	);
}
