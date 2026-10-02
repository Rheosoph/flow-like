"use client";

import type { ComponentType } from "react";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { AccessScreen } from "../access/access-screen";
import { AppDevicesScreen } from "../app/app-devices-screen";
import { CertificatesScreen } from "../certificates/certificates-screen";
import { DeployWizard } from "../deploy/deploy-wizard";
import { DeviceScreen } from "../device/device-screen";
import { FleetScreen } from "../fleet/fleet-screen";
import { HubScreen } from "../hub/hub-screen";
import { KeysScreen } from "../keys/keys-screen";
import type { ScreenProps } from "../screen-props";
import { ServiceScreen } from "../service/service-screen";
import { SetupWizard } from "../setup/setup-wizard";

export interface ScreenComponents {
	fleet: ComponentType<ScreenProps>;
	device: ComponentType<ScreenProps & { deviceId: string }>;
	service: ComponentType<ScreenProps & { deviceId: string; serviceId: string }>;
	setup: ComponentType<ScreenProps>;
	deploy: ComponentType<ScreenProps>;
	access: ComponentType<ScreenProps>;
	certificates: ComponentType<ScreenProps>;
	keys: ComponentType<ScreenProps>;
	hub: ComponentType<ScreenProps>;
	"app-devices": ComponentType<ScreenProps & { appId: string }>;
}

export const SCREENS: ScreenComponents = {
	fleet: FleetScreen,
	device: DeviceScreen,
	service: ServiceScreen,
	setup: SetupWizard,
	deploy: DeployWizard,
	access: AccessScreen,
	certificates: CertificatesScreen,
	keys: KeysScreen,
	hub: HubScreen,
	"app-devices": AppDevicesScreen,
};

export type RailMode = "docked" | "overlay" | "none";

/**
 * SPEC §3.1: the fleet table is the device list, so its rail opens as an overlay;
 * device and service pages dock it when the area is wide; in an app the config
 * nav already takes the left column; wizards and the account sections have none.
 */
export function railMode(
	route: DevicesRoute,
	scope: DevicesScope,
	wide: boolean,
) {
	const rail = SCREEN_RAIL[route.screen];
	if (rail !== "dock") return rail;
	return scope.kind === "account" && wide ? "docked" : "overlay";
}

/** `dock`: docked next to the page when there is room for it. */
const SCREEN_RAIL: Record<
	DevicesRoute["screen"],
	Exclude<RailMode, "docked"> | "dock"
> = {
	fleet: "overlay",
	"app-devices": "overlay",
	device: "dock",
	service: "dock",
	setup: "none",
	deploy: "none",
	access: "none",
	certificates: "none",
	keys: "none",
	hub: "none",
};

function screenFor(
	route: DevicesRoute,
	scope: DevicesScope,
	screens: ScreenComponents,
) {
	switch (route.screen) {
		case "device":
			return (
				<screens.device route={route} scope={scope} deviceId={route.deviceId} />
			);
		case "service":
			return (
				<screens.service
					route={route}
					scope={scope}
					deviceId={route.deviceId}
					serviceId={route.serviceId}
				/>
			);
		case "app-devices": {
			if (scope.kind !== "app")
				return <screens.fleet route={route} scope={scope} />;
			const AppDevices = screens["app-devices"];
			return <AppDevices route={route} scope={scope} appId={scope.appId} />;
		}
		default: {
			const Screen = screens[route.screen];
			return <Screen route={route} scope={scope} />;
		}
	}
}

/** Maps the route to its screen. `screens` is replaceable so the mapping can be tested without the real screens. */
export function ScreenSwitch({
	route,
	scope,
	screens = SCREENS,
}: Readonly<{
	route: DevicesRoute;
	scope: DevicesScope;
	screens?: ScreenComponents;
}>) {
	return (
		<div data-screen={route.screen} className="contents">
			{screenFor(route, scope, screens)}
		</div>
	);
}
