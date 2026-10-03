"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { userLookupQueryOptions } from "../../../../hooks/use-user-lookup";
import {
	deviceName,
	fleetFacts,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import type {
	AgentFeatures,
	DeviceViewModel,
	DevicesScope,
	Freshness,
	GateResult,
	GrantedCapabilities,
	InspectionPlus,
	Presence,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type {
	Capability,
	ManagementPolicy,
	PolicyView,
} from "../../../../lib/device-management/types";
import type {
	LiveFacts,
	LiveState,
} from "../../../../lib/device-management/workspace/types";
import { userDisplayName } from "../../../../lib/user-display";
import { useBackend } from "../../../../state/backend-state";
import { enumLabel } from "../copy/enum-labels";
import { type CopyFormat, gateCopy } from "../copy/gate-copy";
import type { DevicesT } from "../primitives/area-context";
import type { Gate } from "../primitives/gate-notice";
import {
	buildGateContext,
	useAttentionState,
	useDeviceView,
} from "../workspace";

/** One device (and optionally one of its services) as the observability blocks read it. */
export interface ObserveTarget {
	deviceId: string;
	/** `null`: the whole device. */
	serviceId: string | null;
	scope: DevicesScope;
	/** False while the hub list hasn't named the device yet. */
	known: boolean;
	view: DeviceViewModel | undefined;
	name: string;
	me: string;
	owner: boolean;
	revoked: boolean;
	/** Keys for this device exist on this computer. */
	hasKeys: boolean;
	/** The keys aren't unlocked: live data was dropped and nothing can be read. */
	locked: boolean;
	/** A live session is open: commands and live reads work now. */
	liveOpen: boolean;
	live: LiveState;
	presence: Presence | undefined;
	inspection: InspectionPlus | undefined;
	/** Where the services and device facts come from (live read, encrypted status or saved). */
	source: Freshness | undefined;
	features: AgentFeatures | undefined;
	/** Readable services; `null` while they can't be read. */
	services: ServiceView[] | null;
	service: ServiceView | undefined;
	/** What tabs recorded after their own live reads (history readers, shared metric readers). */
	facts: LiveFacts | undefined;
	/** The owner-signed access rules, verified with the open keys. */
	policy: ManagementPolicy | undefined;
	/** The hub's record of the rules: saved and applied versions. */
	policyView: PolicyView | undefined;
	/** What the viewer's access covers; undefined while it isn't known (older hub, locked). */
	capabilities: readonly GrantedCapabilities[] | undefined;
}

const IDLE: LiveState = { kind: "idle" };

/**
 * Whether the viewer may read `capability` on the whole device (`service`
 * null) or on one service. Undefined while the permissions aren't known:
 * then the device decides when it is asked.
 */
export function mayRead(
	target: Pick<ObserveTarget, "owner" | "capabilities">,
	capability: Capability,
	service: { serviceId: string; projectId?: string } | null,
): boolean | undefined {
	if (target.owner) return true;
	if (!target.capabilities) return undefined;
	return target.capabilities.some(
		({ scope, caps }) =>
			caps.includes(capability) &&
			(scope.kind === "device" ||
				(service !== null &&
					(scope.kind === "project"
						? scope.project_id === service.projectId
						: scope.placement_id === service.serviceId))),
	);
}

/**
 * The one app a viewer's Read logs access is limited to; `null` for
 * whole-device access, several apps, or permissions that aren't known.
 */
export function scopedApp(
	target: Pick<ObserveTarget, "owner" | "capabilities">,
): string | null {
	if (target.owner || !target.capabilities) return null;
	const reading = target.capabilities.filter((grant) =>
		grant.caps.includes("logs"),
	);
	if (reading.some((grant) => grant.scope.kind === "device")) return null;
	const apps = new Set(
		reading.flatMap((grant) =>
			grant.scope.kind === "device" ? [] : [grant.scope.project_id],
		),
	);
	return apps.size === 1 ? ([...apps][0] ?? null) : null;
}

export function useObserveTarget(
	deviceId: string,
	serviceId: string | null,
	scope: DevicesScope,
): ObserveTarget {
	const view = useDeviceView(deviceId);
	const state = useAttentionState();
	const { input } = state;
	return useMemo(() => {
		const facts = fleetFacts(input).byId.get(deviceId);
		const services = Array.isArray(facts?.services) ? facts.services : null;
		return {
			deviceId,
			serviceId,
			scope,
			known: !!view,
			view,
			name: view ? deviceName(view.row) : deviceId,
			me: input.me,
			owner: view?.relationship === "owner",
			revoked: view?.row.status === "revoked",
			hasKeys: !!facts?.vault,
			locked: view ? keysLocked(view.keys) : true,
			liveOpen: facts?.liveOpen ?? false,
			live: view?.live ?? IDLE,
			presence: view?.presence,
			inspection: facts?.inspection,
			source: facts?.inspectionSource,
			features: facts?.inspection?.features,
			services,
			service: serviceId
				? services?.find((row) => row.serviceId === serviceId)
				: undefined,
			facts: facts?.liveInput,
			policy: input.policies[deviceId]?.policy,
			policyView: input.policies[deviceId],
			capabilities: buildGateContext(state, deviceId).capabilities,
		};
	}, [view, input, state, deviceId, serviceId, scope]);
}

/**
 * Why a read is refused before it is sent: the action's gate, or access that
 * doesn't cover this scope (the gate only knows the device as a whole).
 * `null` when the read may go out.
 */
export function readRefusal(
	t: DevicesT,
	target: ObserveTarget,
	gate: GateResult,
	capability: Extract<Capability, "logs" | "metrics">,
	service: { serviceId: string; projectId?: string } | null,
	fmt?: CopyFormat,
): string | null {
	if (!gate.ok && gate.kind === "noaccess")
		return gateCopy(t, gate, fmt).inline;
	if (mayRead(target, capability, service) !== false) return null;
	const permission = enumLabel(t, "capability", capability);
	return service
		? t(
				"devices:observe.access.needsOnService",
				"Needs {{permission}} on {{service}}.",
				{ permission, service: service.serviceId },
			)
		: t(
				"devices:observe.access.needsOnDevice",
				"Needs {{permission}} on the whole device. Your access to {{device}} covers single apps or services.",
				{ permission, device: target.name },
			);
}

/** A failed gate as the one line next to its disabled control (R7); `null` when the action is allowed. */
export function gateLine(
	t: DevicesT,
	gate: GateResult,
	fmt?: CopyFormat,
): Gate | null {
	if (gate.ok) return null;
	return { kind: gate.kind, reason: gateCopy(t, gate, fmt).inline };
}

export type PersonNames = (userId: string) => string | undefined;

/** Display names of accounts; undefined while a name isn't known (callers fall back to a neutral phrase). */
export function usePeople(userIds: readonly string[]): PersonNames {
	const backend = useBackend();
	const key = JSON.stringify([...new Set(userIds)].sort());
	const ids = useMemo(() => JSON.parse(key) as string[], [key]);
	const results = useQueries({
		queries: ids.map((id) => ({
			...userLookupQueryOptions(backend.userState, id),
			retry: false,
		})),
	});
	// An account without a name, handle or mail stays unnamed: its id is never a label (R3).
	const names = results.map((result) =>
		result.data ? userDisplayName(result.data, "") : "",
	);
	const signature = names.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `signature` stands for the looked-up names
	return useMemo(() => {
		const byId = new Map(ids.map((id, index) => [id, names[index]]));
		return (userId: string) => byId.get(userId) || undefined;
	}, [ids, signature]);
}
