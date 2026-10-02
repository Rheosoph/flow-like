import type { ReactNode } from "react";
import { eventEligibility } from "../../../../lib/device-management/deployment";
import {
	APPS,
	PLACEMENTS,
	sampleDevices,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	type AppDeviceInput,
	buildAppView,
} from "../../../../lib/device-management/model/app-plan";
import type {
	DevicesRoute,
	DevicesScope,
	FixAction,
} from "../../../../lib/device-management/model/types";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { devicesHref } from "../routing/devices-href";
import type { Coverage } from "../workspace";
import {
	type EventsDevicesLive,
	type EventsDevicesStatus,
	type EventsDevicesValue,
	EventsDevicesValueProvider,
	blockOf,
} from "./events-devices";
import { runsOnDeviceNames, runsOnRows } from "./runs-on-model";

/* Test support: the Events device column over the pure sample apps, without a workspace. */

export type SampleApp = keyof typeof APPS;

export const sampleEvents = (appId: SampleApp) =>
	APPS[appId].events as unknown as IEvent[];

/** The sample events with the fields the Events list reads besides the device rule. */
export function samplePageEvents(appId: SampleApp): IEvent[] {
	const at = { secs_since_epoch: 0, nanos_since_epoch: 0 };
	return sampleEvents(appId).map((event, index) => ({
		...event,
		description: `What ${event.name} does`,
		board_id: `board_${index % 2}`,
		node_id: `node_${index}`,
		priority: index,
		config: [],
		created_at: at,
		updated_at: at,
	})) as IEvent[];
}

const unreadable = (device: AppDeviceInput) => !Array.isArray(device.services);

function coverageOf(devices: readonly AppDeviceInput[]): Coverage {
	const seen = devices.filter((device) => device.presence.kind !== "revoked");
	const unknown = seen.filter(unreadable);
	const ids = (list: readonly AppDeviceInput[]) =>
		list.map((device) => device.id);
	return {
		total: seen.length,
		readable: seen.length - unknown.length,
		live: 0,
		snapshot: seen.length - unknown.length,
		unknown: ids(unknown),
		never: ids(unknown.filter((device) => device.presence.kind === "never")),
		locked: ids(unknown.filter((device) => device.keyState === "locked")),
		noKeys: ids(unknown.filter((device) => device.keyState === "none")),
		noAccess: [],
		partial: [],
		deployed: [],
		notDeployed: [],
	};
}

export interface SampleLiveOptions {
	labUnlocked?: boolean;
	devices?: AppDeviceInput[];
	canDeploy?: boolean;
	/** Fixes a gate asked for, in order. */
	fixes?: FixAction[];
}

/** What the passive workspace would report for a sample app. */
export function sampleLive(
	appId: SampleApp,
	options: SampleLiveOptions = {},
): EventsDevicesLive {
	const devices = options.devices ?? sampleDevices(options);
	const view = buildAppView({
		app: APPS[appId],
		devices,
		placements: PLACEMENTS[appId],
		focusDeviceIds: devices.map((device) => device.id),
	});
	return {
		view,
		rows: runsOnRows(view),
		names: runsOnDeviceNames(view),
		coverage: coverageOf(devices),
		canDeploy: options.canDeploy ?? true,
		hub: { src: "hub", age: "current", at: 1_790_769_587, cadenceS: 30 },
		runFix: (fix) => {
			options.fixes?.push(fix);
			return { kind: "done" };
		},
	};
}

export interface SampleValueOptions extends SampleLiveOptions {
	status?: EventsDevicesStatus;
	value?: Partial<EventsDevicesValue>;
	/** Hrefs opened through `go` and plain link clicks. */
	opened?: string[];
}

/** The context value of `EventsDevicesProvider` for a sample app in one status. */
export function sampleValue(
	appId: SampleApp,
	options: SampleValueOptions = {},
): EventsDevicesValue {
	const status = options.status ?? "ready";
	const events = sampleEvents(appId);
	const scope: DevicesScope = { kind: "app", appId };
	const live = status === "ready" ? sampleLive(appId, options) : null;
	const href = (route: DevicesRoute, target: DevicesScope = scope) =>
		devicesHref(route, target);
	let claimed = 0;
	return {
		appId,
		scope,
		status,
		live,
		problem: null,
		block: blockOf({ status, live }),
		events: new Map(events.map((event) => [event.id, event])),
		eligibility: new Map(
			events.map((event) => [event.id, eventEligibility(event)]),
		),
		focus: null,
		claimFocus: (token) => {
			if (claimed === token) return false;
			claimed = token;
			return true;
		},
		showOnDevices: () => undefined,
		explainMode: () => undefined,
		link: (route, target) => ({
			href: href(route, target),
			onClick: (event) => {
				event.preventDefault();
				options.opened?.push(href(route, target));
			},
		}),
		go: (route, target) => {
			options.opened?.push(href(route, target));
		},
		...options.value,
	};
}

export function SampleEventsDevices({
	value,
	children,
}: Readonly<{ value: EventsDevicesValue; children: ReactNode }>) {
	return (
		<EventsDevicesValueProvider value={value}>
			{children}
		</EventsDevicesValueProvider>
	);
}
