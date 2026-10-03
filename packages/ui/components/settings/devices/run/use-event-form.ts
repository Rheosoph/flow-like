"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
	agentSupports,
	readEventForm,
} from "../../../../lib/device-management/agent-reads";
import type {
	AgentFeatures,
	EventForm,
} from "../../../../lib/device-management/model/types";
import {
	type DeviceFailure,
	LiveCallError,
	classifyDeviceError,
} from "../../../../lib/device-management/workspace/errors";
import { useDeviceCall } from "../workspace/use-live";

export type FormRead =
	| { state: "idle" }
	| { state: "loading" }
	| { state: "ok"; form: EventForm }
	/** The agent can't run forms and quick actions. */
	| { state: "unsupported" }
	/** The service changed or isn't running: the device has no form for its current settings. */
	| { state: "changed" }
	| { state: "failed"; failure: DeviceFailure };

export interface FormReadView {
	read: FormRead;
	/** Asks the device again, also after a form was read. */
	reload(): void;
}

/**
 * The form of one event as the service on the device runs it (`event_form`).
 * Read once while `enabled`; a form already read stays when the gate flickers.
 */
export function useEventForm(input: {
	deviceId: string;
	serviceId: string;
	eventId: string;
	features: AgentFeatures | undefined;
	enabled: boolean;
}): FormReadView {
	const { deviceId, serviceId, eventId, features, enabled } = input;
	const call = useDeviceCall(deviceId);
	const supported = agentSupports(features, "on_demand_events");
	const [read, setRead] = useState<FormRead>({ state: "idle" });
	const [round, setRound] = useState(0);
	const answered = useRef(-1);
	const reload = useCallback(() => setRound((value) => value + 1), []);

	useEffect(() => {
		if (!enabled || answered.current === round) return;
		let alive = true;
		setRead({ state: "loading" });
		const flags: AgentFeatures | undefined = supported
			? { on_demand_events: 1 }
			: undefined;
		readEventForm(call, flags, {
			placementId: serviceId,
			eventId,
		}).then(
			(answer) => {
				if (!alive) return;
				answered.current = round;
				setRead(
					answer.kind === "ok"
						? { state: "ok", form: answer.data }
						: { state: "unsupported" },
				);
			},
			(error) => {
				if (!alive) return;
				answered.current = round;
				setRead(
					error instanceof LiveCallError &&
						error.code === "rejected_revision_conflict"
						? { state: "changed" }
						: { state: "failed", failure: classifyDeviceError(error) },
				);
			},
		);
		return () => {
			alive = false;
		};
	}, [call, supported, serviceId, eventId, enabled, round]);

	return { read, reload };
}
