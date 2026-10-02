"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Boxes,
	FileBadge,
	KeyRound,
	LayoutGrid,
	Lock,
	LockOpen,
	Plus,
	Rocket,
	Server,
	UserPlus,
	Users,
} from "lucide-react";
import { useEffect, useMemo } from "react";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import {
	type SpotlightItem,
	useSpotlightStore,
} from "../../../../state/spotlight-state";
import type { DevicesT } from "../primitives/area-context";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import type { NavigateOptions } from "../routing/use-devices-route";

/** Spotlight group and item source of the area; both exist only while the area is mounted (P9). */
export const DEVICES_SPOTLIGHT_GROUP = "devices-area";

export interface SpotlightDevice {
	id: string;
	name: string;
	/** "Online · needs attention". */
	note?: string;
	keywords?: readonly string[];
}

export interface SpotlightService {
	deviceId: string;
	serviceId: string;
	deviceName: string;
	appName?: string;
}

export interface SpotlightApp {
	id: string;
	name: string;
}

export interface DevicesSpotlightInput {
	scope: DevicesScope;
	devices: readonly SpotlightDevice[];
	services: readonly SpotlightService[];
	apps: readonly SpotlightApp[];
	navigate(route: DevicesRoute, options?: NavigateOptions): void;
	/** Absent when no keys on this computer are locked. */
	onUnlockSeveral?(): void;
	/** Absent when nothing is unlocked. */
	onLockAll?(): void;
}

type Item = Omit<SpotlightItem, "group">;

const id = (...parts: string[]) =>
	[DEVICES_SPOTLIGHT_GROUP, ...parts].join(":");

function deviceItems(input: DevicesSpotlightInput): SpotlightItem[] {
	return input.devices.map((device) => ({
		id: id("device", device.id),
		type: "dynamic",
		group: DEVICES_SPOTLIGHT_GROUP,
		label: device.name,
		description: device.note,
		icon: Server,
		keywords: [device.id, ...(device.keywords ?? [])],
		action: () =>
			input.navigate({
				screen: "device",
				deviceId: device.id,
				tab: "overview",
			}),
	}));
}

function serviceItems(
	input: DevicesSpotlightInput,
	t: DevicesT,
): SpotlightItem[] {
	return input.services.map((service) => ({
		id: id("service", service.deviceId, service.serviceId),
		type: "dynamic",
		group: DEVICES_SPOTLIGHT_GROUP,
		label: service.serviceId,
		description: service.appName
			? t("devices:shell.spotlight.serviceOf", "{{app}} on {{device}}", {
					app: service.appName,
					device: service.deviceName,
				})
			: service.deviceName,
		icon: Boxes,
		keywords: [
			service.deviceName,
			...(service.appName ? [service.appName] : []),
		],
		action: () =>
			input.navigate({
				screen: "service",
				deviceId: service.deviceId,
				serviceId: service.serviceId,
			}),
	}));
}

function appItems(input: DevicesSpotlightInput, t: DevicesT): SpotlightItem[] {
	return input.apps.map((app) => ({
		id: id("app", app.id),
		type: "dynamic",
		group: DEVICES_SPOTLIGHT_GROUP,
		label: app.name,
		description: t(
			"devices:shell.spotlight.appDevices",
			"Where this app runs on devices",
		),
		icon: LayoutGrid,
		keywords: [app.id],
		action: () =>
			input.navigate(
				{ screen: "app-devices", by: "device" },
				{ scope: { kind: "app", appId: app.id } },
			),
	}));
}

/** Devices, services and apps are sub-items: found by search, never listed while the query is empty. */
function pageItems(input: DevicesSpotlightInput, t: DevicesT): Item[] {
	const account: NavigateOptions = { scope: ACCOUNT_SCOPE };
	return [
		{
			id: id("page", "fleet"),
			type: "navigation",
			label: t("devices:shell.spotlight.fleet", "Fleet overview"),
			description: t(
				"devices:shell.spotlight.fleetHint",
				"What needs you, and every device you can see",
			),
			icon: Server,
			keywords: ["devices", "fleet", "servers", "attention"],
			subItems: deviceItems(input),
			action: () =>
				input.navigate({ screen: "fleet", view: "devices" }, account),
		},
		{
			id: id("page", "services"),
			type: "navigation",
			label: t("devices:shell.spotlight.services", "Services on devices"),
			description: t(
				"devices:shell.spotlight.servicesHint",
				"Every service on the devices you can read",
			),
			icon: Boxes,
			keywords: ["services", "deployments", "apps", "running"],
			subItems: [...serviceItems(input, t), ...appItems(input, t)],
			action: () =>
				input.navigate({ screen: "fleet", view: "services" }, account),
		},
		{
			id: id("page", "access"),
			type: "navigation",
			label: t("devices:shell.spotlight.access", "Device access"),
			description: t(
				"devices:shell.spotlight.accessHint",
				"People, devices shared with you, cloud approvals and spending",
			),
			icon: Users,
			keywords: [
				"access",
				"share",
				"sharing",
				"people",
				"permissions",
				"cloud",
			],
			action: () => input.navigate({ screen: "access" }, account),
		},
		{
			id: id("page", "certificates"),
			type: "navigation",
			label: t("devices:shell.spotlight.certificates", "Device certificates"),
			description: t(
				"devices:shell.spotlight.certificatesHint",
				"Expiry across devices, your authorities and reminders",
			),
			icon: FileBadge,
			keywords: ["certificates", "tls", "https", "expiry", "authority"],
			action: () => input.navigate({ screen: "certificates" }, account),
		},
		{
			id: id("page", "keys"),
			type: "navigation",
			label: t("devices:shell.spotlight.keys", "Device keys & recovery"),
			description: t(
				"devices:shell.spotlight.keysHint",
				"Keys on this computer, backups and device passwords",
			),
			icon: KeyRound,
			keywords: ["keys", "recovery", "backup", "password", "restore"],
			action: () => input.navigate({ screen: "keys" }, account),
		},
		{
			id: id("page", "hub"),
			type: "navigation",
			label: t("devices:shell.spotlight.hub", "Hub status for devices"),
			description: t(
				"devices:shell.spotlight.hubHint",
				"Whether this hub supports devices, its limits and agent releases",
			),
			icon: Server,
			keywords: ["hub status", "hub", "readiness", "limits", "releases"],
			action: () => input.navigate({ screen: "hub" }, account),
		},
	];
}

function actionItems(input: DevicesSpotlightInput, t: DevicesT): Item[] {
	const account: NavigateOptions = { scope: ACCOUNT_SCOPE };
	const items: Item[] = [
		{
			id: id("action", "setup"),
			type: "action",
			label: t("devices:shell.spotlight.setup", "Set up a device"),
			icon: Plus,
			keywords: ["add device", "enroll", "install", "new device"],
			action: () => input.navigate({ screen: "setup" }, account),
		},
		{
			id: id("action", "deploy"),
			type: "action",
			label: t("devices:shell.spotlight.deploy", "Deploy an app to devices"),
			icon: Rocket,
			keywords: ["deploy", "run on device", "update"],
			action: () =>
				input.navigate(
					input.scope.kind === "app"
						? { screen: "deploy", deviceIds: [], mode: "new" }
						: { screen: "deploy", deviceIds: [] },
				),
		},
		{
			id: id("action", "request-access"),
			type: "action",
			label: t(
				"devices:shell.spotlight.requestAccess",
				"Request shared access to a device",
			),
			icon: UserPlus,
			keywords: ["request access", "shared", "join"],
			action: () =>
				input.navigate(
					{ screen: "access", tab: "shared", action: "request" },
					account,
				),
		},
	];
	const { onUnlockSeveral, onLockAll } = input;
	if (onUnlockSeveral)
		items.push({
			id: id("action", "unlock-several"),
			type: "action",
			label: t(
				"devices:shell.spotlight.unlockSeveral",
				"Unlock several devices…",
			),
			icon: LockOpen,
			keywords: ["unlock", "password", "keys"],
			action: onUnlockSeveral,
		});
	if (onLockAll)
		items.push({
			id: id("action", "lock-all"),
			type: "action",
			label: t("devices:shell.spotlight.lockAll", "Lock all devices"),
			icon: Lock,
			keywords: ["lock", "keys"],
			action: onLockAll,
		});
	return items;
}

/** SPEC §3.7 through the app's Spotlight (P9): pages and actions listed, objects found by search. */
export function buildSpotlightItems(
	input: DevicesSpotlightInput,
	t: DevicesT,
): SpotlightItem[] {
	return [...pageItems(input, t), ...actionItems(input, t)].map((item) => ({
		...item,
		group: DEVICES_SPOTLIGHT_GROUP,
	}));
}

/** Registers the area's Spotlight group while mounted; `null` registers nothing (an area gate is showing). */
export function useDevicesSpotlight(input: DevicesSpotlightInput | null): void {
	const { t } = useTranslation("devices");
	const items = useMemo(
		() => (input ? buildSpotlightItems(input, t) : []),
		[input, t],
	);
	const label = t("shell.spotlight.group", "Devices");
	useEffect(() => {
		if (items.length === 0) return;
		const store = useSpotlightStore.getState();
		store.registerGroup({ id: DEVICES_SPOTLIGHT_GROUP, label, priority: 80 });
		store.registerDynamicItems(DEVICES_SPOTLIGHT_GROUP, items);
		return () => {
			const current = useSpotlightStore.getState();
			current.unregisterDynamicItems(DEVICES_SPOTLIGHT_GROUP);
			current.unregisterGroup(DEVICES_SPOTLIGHT_GROUP);
		};
	}, [items, label]);
}
