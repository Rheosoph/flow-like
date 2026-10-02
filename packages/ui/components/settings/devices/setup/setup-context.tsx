"use client";

import { type ReactNode, createContext, useContext } from "react";
import type {
	DevicesRoute,
	SetupStep,
} from "../../../../lib/device-management/model/types";
import type { VerifiedRelease } from "../../../../lib/device-management/package";
import type { GateKind } from "../primitives/icons";
import type { RouteLinkProps } from "../routing/use-devices-route";
import type { SetupDraft, TargetOption } from "./setup-state";
import type { SetupChecks } from "./use-setup-checks";
import type { BuiltPackage, SetupCreate } from "./use-setup-create";
import type { SetupWait } from "./use-setup-wait";

/** The hub's limits as far as this hub states them (older hubs: only the hub record). */
export interface SetupLimits {
	maxDevices?: number;
	maxPending?: number;
	maxPerDay?: number;
	/** How long a package works, in seconds. */
	lifetimeS: number;
	activeDevices?: number;
	pending?: number;
	lastDay?: number;
}

export interface SetupSecrets {
	password: string;
	repeat: string;
	setPassword(value: string): void;
	setRepeat(value: string): void;
}

/** Whether the step's one primary may run, and the one line that says why not. */
export interface NextState {
	ok: boolean;
	reason?: ReactNode;
	kind?: GateKind;
	/** A field is missing: the button stays enabled and pressing it shows the messages. */
	field?: boolean;
}

/** What every step reads and does; the wizard owns the state. */
export interface SetupController {
	/** The hub's host, as people know it ("api.flow-like.com"). */
	host: string;
	draft: SetupDraft;
	step: SetupStep;
	/** The stepper's labels, by step. */
	labels: readonly string[];
	/** Hub-corrected unix seconds, ticking with the area clock. */
	nowS: number;
	update(patch: Partial<SetupDraft>): void;
	goTo(step: SetupStep): void;
	checks: SetupChecks;
	/** The verified release, kept while a re-check runs; undefined once it is rejected. */
	release: VerifiedRelease | undefined;
	options: readonly TargetOption[];
	/** The chosen platform. */
	option: TargetOption | undefined;
	secrets: SetupSecrets;
	/** Continue was pressed with something missing: every message of the step shows. */
	tried: boolean;
	/** Names of this account's devices and pending setups. */
	takenNames: readonly string[];
	limits: SetupLimits;
	create: SetupCreate;
	built: BuiltPackage | undefined;
	wait: SetupWait;
	expired: boolean;
	next: NextState;
	/** Why the last "Cancel setup" did not go through, in the viewer's words. */
	cancelError?: string;
	dismissCancelError(): void;
	cancelSetup(): void;
	/** A fresh setup, optionally named like an expired one. */
	startNew(name?: string): void;
	/** Leaves the wizard; asks first when a running creation or the package would be lost. */
	leave(route: DevicesRoute): void;
	/** The same as a link, so a modifier click opens a new tab. */
	leaveLink(route: DevicesRoute): RouteLinkProps;
}

const SetupContext = createContext<SetupController | null>(null);

export const SetupProvider = SetupContext.Provider;

export function useSetup(): SetupController {
	const setup = useContext(SetupContext);
	if (!setup) throw new Error("useSetup() needs the SetupWizard above it.");
	return setup;
}
