import type { ReactNode } from "react";
import { eventEligibility } from "../../../../lib/device-management/deployment";
import {
	APPS,
	NOW0,
	PLACEMENTS,
	sampleDevices,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	type AppDeviceInput,
	type AppEventInput,
	type AppInput,
	buildAppView,
} from "../../../../lib/device-management/model/app-plan";
import type {
	AppDevicePlacements,
	AppScheduleRow,
	DevicesRoute,
	DevicesScope,
	FixAction,
} from "../../../../lib/device-management/model/types";
import type { IEvent } from "../../../../lib/schema/flow/event";
import { AreaNowContext } from "../primitives/area-context";
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

/** The sample events with the fields the Events list reads besides the device rule; a schedule, a route or a bot's settings sit in the config bytes, as in the app's store. */
export function samplePageEvents(appId: SampleApp): IEvent[] {
	const at = { secs_since_epoch: 0, nanos_since_epoch: 0 };
	return APPS[appId].events.map((event, index) => {
		const { schedule, config, ...record }: AppEventInput = event;
		return {
			...record,
			description: `What ${event.name} does`,
			board_id: `board_${index % 2}`,
			node_id: `node_${index}`,
			priority: index,
			config: schedule
				? [...new TextEncoder().encode(JSON.stringify(schedule))]
				: [...(config ?? [])],
			created_at: at,
			updated_at: at,
		};
	}) as unknown as IEvent[];
}

/** A sample app with one of its events changed (paused, another schedule, a flow with edits). */
export function sampleAppWith(
	appId: SampleApp,
	eventId: string,
	patch: Partial<AppEventInput>,
): AppInput {
	const app: AppInput = APPS[appId];
	return {
		...app,
		events: app.events.map((event) =>
			event.id === eventId ? { ...event, ...patch } : event,
		),
	};
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
	/** The sample app with some events changed (a schedule paused, a flow with edits). */
	app?: AppInput;
	/** Where the app's schedules and bots run, as the hub lists them; none by default. */
	schedules?: AppScheduleRow[];
	/** The cloud approvals the hub lists for the app; the sample's by default. */
	grants?: AppDevicePlacements["placements"];
	/** False: the viewer may not edit the app's events. */
	canEditEvents?: boolean;
	/** Events handed back to the hub through "Run it on the hub again", in order. */
	givenBack?: string[];
	/** Hub-corrected unix seconds; the sample's clock by default. */
	now?: number;
	/** The `run_event` gate per device; open everywhere by default. */
	runGate?: EventsDevicesLive["runGate"];
}

/** What the passive workspace would report for a sample app. */
export function sampleLive(
	appId: SampleApp,
	options: SampleLiveOptions = {},
): EventsDevicesLive {
	const devices = options.devices ?? sampleDevices(options);
	const now = options.now ?? NOW0;
	const view = buildAppView({
		app: options.app ?? APPS[appId],
		devices,
		placements: {
			...PLACEMENTS[appId],
			...(options.schedules ? { schedules: options.schedules } : {}),
			...(options.grants ? { placements: options.grants } : {}),
		},
		focusDeviceIds: devices.map((device) => device.id),
		now,
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
		schedules: {
			canEdit: options.canEditEvents ?? true,
			giveBack: async (eventId) => {
				options.givenBack?.push(eventId);
				return { kind: "ok", data: { hub_resumes_at: now + 300 } };
			},
		},
		runGate: options.runGate ?? (() => ({ ok: true })),
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
	const events = (options.app ?? APPS[appId]).events as unknown as IEvent[];
	const scope: DevicesScope = { kind: "app", appId };
	const live = status === "ready" ? sampleLive(appId, options) : null;
	const href = (route: DevicesRoute, target: DevicesScope = scope) =>
		devicesHref(route, target);
	let claimed = 0;
	// As on the page: the event's own rule, replaced by the device area's once it has read the app.
	const eligibility = new Map(
		events.map((event) => [event.id, eventEligibility(event)]),
	);
	for (const row of live
		? [...live.view.events.rows, ...live.view.events.ineligible]
		: [])
		eligibility.set(row.eventId, row.eligibility);
	return {
		appId,
		scope,
		status,
		live,
		problem: null,
		block: blockOf({ status, live }),
		events: new Map(events.map((event) => [event.id, event])),
		eligibility,
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

/** The provider of a sample value; `now` (unix seconds) pins the clock that times and "in 12 hr" are read against. */
export function SampleEventsDevices({
	value,
	now,
	children,
}: Readonly<{
	value: EventsDevicesValue;
	now?: number;
	children: ReactNode;
}>) {
	const content = (
		<EventsDevicesValueProvider value={value}>
			{children}
		</EventsDevicesValueProvider>
	);
	return now === undefined ? (
		content
	) : (
		<AreaNowContext.Provider value={now * 1000}>
			{content}
		</AreaNowContext.Provider>
	);
}
