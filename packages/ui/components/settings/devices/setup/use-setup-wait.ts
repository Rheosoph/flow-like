"use client";

import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import {
	type HubError,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import {
	deviceKeys,
	queries,
} from "../../../../lib/device-management/hub/queries";
import { useDeviceWorkspace } from "../workspace";
import type { SetupDraft } from "./setup-state";
import { finishSetupActivity, setupActivityId } from "./use-setup-create";

/** How often the waiting step asks the hub for the new device (IA §6.2 N5 step 7). */
export const WAIT_POLL_MS = 5_000;

/** The hub has no device with this id yet: 404 today, 403 on hubs before the registry work. */
const NOT_REGISTERED = new Set(["not_found", "forbidden"]);

export interface SetupWait {
	/** Unix seconds the device registered (it started the package). */
	registeredAt?: number;
	/** Unix seconds of the first check-in. */
	checkedInAt?: number;
	/** Epoch milliseconds of the last answer, a "not registered yet" included. */
	checkedAt?: number;
	/** The hub gave no usable answer on the last try; polling goes on. */
	error?: HubError;
}

/** Polls `GET /devices/{id}` while the waiting step is open and the device has not checked in. */
export function useSetupWait(
	deviceId: string | undefined,
	enabled: boolean,
): SetupWait {
	const { hub } = useDeviceWorkspace();
	const query = useQuery({
		...queries.device(hub, deviceId ?? ""),
		enabled: enabled && !!deviceId,
		refetchInterval: WAIT_POLL_MS,
		staleTime: 0,
		retry: false,
	});
	const { data, error, dataUpdatedAt, errorUpdatedAt } = query;
	return useMemo(() => {
		const failure = error ? toHubError(error) : undefined;
		const waiting = !!failure && NOT_REGISTERED.has(failure.code);
		const checkedAt = Math.max(dataUpdatedAt, errorUpdatedAt);
		const row = failure ? undefined : data;
		return {
			...(row ? { registeredAt: row.registered_at } : {}),
			...(typeof row?.last_seen_at === "number"
				? { checkedInAt: row.last_seen_at }
				: {}),
			...(checkedAt ? { checkedAt } : {}),
			...(failure && !waiting ? { error: failure } : {}),
		};
	}, [data, error, dataUpdatedAt, errorUpdatedAt]);
}

/**
 * The waiting step's watch: what the hub says about the new device goes into
 * the draft, and its first check-in finishes the tray entry and refreshes the
 * lists the device now belongs to.
 */
export function useSetupArrival(
	draft: SetupDraft,
	watching: boolean,
	update: (patch: Partial<SetupDraft>) => void,
): SetupWait {
	const workspace = useDeviceWorkspace();
	const { created } = draft;
	const wait = useSetupWait(created?.deviceId || undefined, watching);
	const { registeredAt, checkedInAt } = wait;
	const enrollmentId = created?.enrollmentId;
	const deviceId = created?.deviceId ?? "";
	const known =
		draft.registeredAt === registeredAt && draft.checkedInAt === checkedInAt;

	useEffect(() => {
		if (registeredAt === undefined || known) return;
		update({
			registeredAt,
			...(checkedInAt === undefined ? {} : { checkedInAt }),
		});
		if (checkedInAt === undefined) return;
		const itemId = setupActivityId(workspace, enrollmentId);
		if (itemId) finishSetupActivity(workspace, itemId, deviceId);
		const { scopeKey } = workspace;
		for (const queryKey of [
			deviceKeys.list(scopeKey),
			deviceKeys.enrollments(scopeKey, "open"),
			deviceKeys.usage(scopeKey),
		])
			void workspace.deps.queryClient.invalidateQueries({ queryKey });
	}, [
		registeredAt,
		checkedInAt,
		known,
		enrollmentId,
		deviceId,
		update,
		workspace,
	]);

	return wait;
}
