"use client";

import { create } from "zustand";
import { isUpgradeRequiredError } from "../lib/api-error";

export type UpgradeReason =
	| "project-limit"
	| "model-tier"
	| "storage"
	| "executions"
	| "generic";

export interface UpgradeDialogTrigger {
	reason: UpgradeReason;
	/** Human-readable detail, e.g. the server's plan-limit message. */
	message?: string;
	/** Tier that would unlock the blocked action, when known. */
	requiredTier?: string;
}

interface UpgradeDialogState {
	isOpen: boolean;
	trigger?: UpgradeDialogTrigger;
	/**
	 * Mirrors `conversion.enabled` from the hub config; synced by the mounted
	 * GlobalUpgradeDialog. While false, plan-limit errors fall back to plain
	 * toasts instead of the dialog.
	 */
	enabled: boolean;
	open: (trigger?: UpgradeDialogTrigger) => void;
	close: () => void;
	setEnabled: (enabled: boolean) => void;
}

/**
 * Global upgrade/conversion dialog. Opened whenever the user runs into a plan
 * limit (project quota, restricted model tier, ...) so the moment of friction
 * becomes an upgrade opportunity instead of a dead end.
 */
export const useUpgradeDialogStore = create<UpgradeDialogState>((set) => ({
	isOpen: false,
	trigger: undefined,
	enabled: true,
	open: (trigger) => set({ isOpen: true, trigger }),
	close: () => set({ isOpen: false }),
	setEnabled: (enabled) => set({ enabled }),
}));

/** Imperative open — usable outside React components. */
export function openUpgradeDialog(trigger?: UpgradeDialogTrigger): void {
	useUpgradeDialogStore.getState().open(trigger);
}

/**
 * Opens the dialog unless the hub disabled the conversion flow. Returns
 * whether it opened, so call sites can fall back to a plain toast.
 */
export function openUpgradeDialogIfEnabled(
	trigger?: UpgradeDialogTrigger,
): boolean {
	const store = useUpgradeDialogStore.getState();
	if (!store.enabled) return false;
	store.open(trigger);
	return true;
}

/**
 * Routes a plan-limit rejection (HTTP 402 / PAYMENT_REQUIRED) into the upgrade
 * dialog. Returns true when the error was consumed; callers keep their normal
 * error handling for everything else (and for hubs with conversion disabled).
 */
export function handleUpgradeRequiredError(
	error: unknown,
	reason: UpgradeReason = "generic",
): boolean {
	if (!isUpgradeRequiredError(error)) return false;
	const store = useUpgradeDialogStore.getState();
	if (!store.enabled) return false;
	store.open({ reason, message: error.serverMessage ?? error.message });
	return true;
}
