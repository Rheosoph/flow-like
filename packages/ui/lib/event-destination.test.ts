import { describe, expect, test } from "bun:test";
import {
	type DestinationEnvironment,
	blockingReason,
	destinationHintText,
	destinationReasonText,
	destinationSinkExecution,
	destinationStatus,
	destinationStatuses,
	initialDestination,
	resolveDestination,
} from "./event-destination";
import { withDeviceEventSource } from "./event-source";

const env = (patch: Partial<DestinationEnvironment> = {}) => ({
	deviceCreation: true,
	canExecuteLocally: true,
	isOffline: false,
	boardMode: "Hybrid",
	...patch,
});
const localSink = { availability: "local" as const };
const remoteSink = { availability: "remote" as const };
const t = (key: string, fallback: string) => `${key}|${fallback}`;

describe("destination status", () => {
	test("an unset or runnable type fits every destination on a desktop with an online hub", () => {
		for (const eventType of [undefined, "quick_action", "cron", "api"]) {
			const statuses = destinationStatuses({ eventType }, env());
			expect(Object.values(statuses).every((status) => status.available)).toBe(
				true,
			);
		}
	});

	test("a type no device runs gives the device a reason, a page always fits", () => {
		expect(
			destinationStatus("device", { eventType: "deeplink" }, env()),
		).toEqual({ available: false, reason: "device_unsupported" });
		expect(
			destinationStatus(
				"device",
				{ eventType: "deeplink", pageId: "desk" },
				env(),
			),
		).toEqual({ available: true });
	});

	test("a host without device creation has no device destination", () => {
		expect(
			destinationStatus("device", {}, env({ deviceCreation: false })),
		).toEqual({ available: false, reason: "device_unavailable" });
	});

	test("a web client cannot use this computer", () => {
		expect(
			destinationStatus("computer", {}, env({ canExecuteLocally: false })),
		).toEqual({ available: false, reason: "no_local" });
	});

	test("the flow's execution mode excludes the other side", () => {
		expect(destinationStatus("hub", {}, env({ boardMode: "Local" }))).toEqual({
			available: false,
			reason: "board_local",
		});
		expect(
			destinationStatus("computer", {}, env({ boardMode: "Remote" })),
		).toEqual({ available: false, reason: "board_remote" });
	});

	test("an offline app cannot use the hub, an unknown or failed check can", () => {
		expect(destinationStatus("hub", {}, env({ isOffline: true }))).toEqual({
			available: false,
			reason: "offline",
		});
		expect(destinationStatus("hub", {}, env({ isOffline: undefined }))).toEqual(
			{ available: true },
		);
		expect(
			destinationStatus(
				"hub",
				{},
				env({ isOffline: undefined, offlineCheckFailed: true }),
			),
		).toEqual({ available: true, hint: "offline_unknown" });
	});

	test("sink availability splits the trigger between computer and hub", () => {
		const local = destinationStatuses(
			{ eventType: "deeplink", sink: localSink },
			env(),
		);
		expect(local.hub).toEqual({ available: false, reason: "sink_local_only" });
		expect(local.computer.available).toBe(true);
		const remote = destinationStatuses(
			{ eventType: "webhook", sink: remoteSink },
			env(),
		);
		expect(remote.computer).toEqual({
			available: false,
			reason: "sink_remote_only",
		});
		expect(remote.hub.available).toBe(true);
	});

	test("a server-only type runs on the hub only, and only when the hub names it", () => {
		const target = { eventType: "teams", sink: remoteSink };
		const known = destinationStatuses(
			target,
			env({ hubSupportedSinks: { teams: true } }),
		);
		expect(known.computer).toEqual({
			available: false,
			reason: "sink_remote_only",
		});
		expect(known.device).toEqual({
			available: false,
			reason: "device_unsupported",
		});
		expect(known.hub).toEqual({ available: true });
		expect(
			destinationStatus("hub", target, env({ hubSupportedSinks: {} })),
		).toEqual({ available: false, reason: "hub_unsupported" });
		expect(destinationStatus("hub", target, env())).toEqual({
			available: true,
		});
	});
});

describe("destination resolution", () => {
	test("keeps the preferred destination while it fits", () => {
		const statuses = destinationStatuses({ eventType: "cron" }, env());
		expect(resolveDestination("computer", statuses)).toEqual({
			destination: "computer",
			none: false,
		});
	});

	test("falls back to the first destination that fits and says why", () => {
		const statuses = destinationStatuses({ eventType: "deeplink" }, env());
		expect(resolveDestination("device", statuses)).toEqual({
			destination: "computer",
			none: false,
			fellBackFrom: { destination: "device", reason: "device_unsupported" },
		});
		const web = destinationStatuses(
			{ eventType: "teams", sink: remoteSink },
			env({ canExecuteLocally: false }),
		);
		expect(resolveDestination("device", web).destination).toBe("hub");
	});

	test("a preferred computer that stops fitting prefers the device before the hub", () => {
		const statuses = destinationStatuses(
			{ eventType: "cron" },
			env({ boardMode: "Remote" }),
		);
		expect(resolveDestination("computer", statuses).destination).toBe("device");
	});

	test("reports when nothing fits and keeps the preferred destination", () => {
		const statuses = destinationStatuses(
			{ eventType: "teams", sink: remoteSink },
			env({ isOffline: true }),
		);
		expect(resolveDestination("hub", statuses)).toEqual({
			destination: "hub",
			none: true,
		});
		expect(blockingReason(statuses)).toBe("offline");
	});

	test("a type that fits somewhere has no blocking reason", () => {
		expect(
			blockingReason(destinationStatuses({ eventType: "deeplink" }, env())),
		).toBeNull();
	});

	test("a local-only trigger on the web is blocked by the desktop requirement", () => {
		const statuses = destinationStatuses(
			{ eventType: "deeplink", sink: localSink },
			env({ canExecuteLocally: false }),
		);
		expect(blockingReason(statuses)).toBe("sink_local_only");
	});
});

describe("initial destination", () => {
	const host = { deviceCreation: true, canExecuteLocally: true };

	test("opens on a device when the host can create them, else computer, else hub", () => {
		expect(initialDestination(host)).toBe("device");
		expect(initialDestination({ ...host, deviceCreation: false })).toBe(
			"computer",
		);
		expect(
			initialDestination({ deviceCreation: false, canExecuteLocally: false }),
		).toBe("hub");
	});

	test("a template's execution mode and device source choose the destination", () => {
		expect(initialDestination(host, { execution_mode: "Remote" })).toBe("hub");
		expect(initialDestination(host, { execution_mode: "Local" })).toBe(
			"computer",
		);
		expect(
			initialDestination(
				{ ...host, canExecuteLocally: false },
				{ execution_mode: "Local" },
			),
		).toBe("device");
		const marked = withDeviceEventSource({ config: [] as number[] }).config;
		expect(
			initialDestination(host, { config: marked, execution_mode: "Remote" }),
		).toBe("device");
		expect(
			initialDestination(
				{ ...host, deviceCreation: false },
				{ config: marked, execution_mode: "Local" },
			),
		).toBe("computer");
	});
});

describe("destination wording", () => {
	test("every reason has a sentence and the hub names the missing type", () => {
		const reasons = [
			"device_unavailable",
			"device_unsupported",
			"offline",
			"board_local",
			"board_remote",
			"no_local",
			"sink_local_only",
			"sink_remote_only",
			"hub_unsupported",
		] as const;
		for (const reason of reasons)
			expect(destinationReasonText(t, reason).length).toBeGreaterThan(10);
		expect(destinationHintText(t, "offline_unknown")).toContain("again");
	});

	test("a schedule carries the target of its destination, a device none", () => {
		expect(destinationSinkExecution("computer")).toBe("LOCAL");
		expect(destinationSinkExecution("hub")).toBe("REMOTE");
		expect(destinationSinkExecution("device")).toBeUndefined();
	});
});
