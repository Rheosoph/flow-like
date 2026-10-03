import type { PlacementStatusPlus } from "../../../../lib/device-management/model/types";
import { useDevicesRoute } from "../routing/use-devices-route";
import {
	type FakeWorkspace,
	type FakeWorkspaceOptions,
	createFakeWorkspace,
} from "../testing/fake-workspace";
import {
	type MountDevicesOptions,
	mountDevices,
} from "../testing/mount-devices";
import { SHOP, serveShopOnEdge } from "../testing/schedule-scenarios";
import { ServiceScreen } from "./service-screen";

/*
 * Shared by the DOM tests of Service › Status's round-two blocks: Shop
 * Assistant on edge-berlin-01, opened on one of its tabs. Import it
 * dynamically, after `installDom()`: it pulls in the area's module graph.
 */

export { SHOP };

function Page() {
	const { route, scope } = useDevicesRoute();
	if (route.screen !== "service")
		return <p data-left="">{`left:${route.screen}`}</p>;
	return (
		<ServiceScreen
			route={route}
			scope={scope}
			deviceId={route.deviceId}
			serviceId={route.serviceId}
		/>
	);
}

export interface ShopOpenOptions extends FakeWorkspaceOptions {
	/** The events the service holds; every one a device can run by default. */
	events?: readonly string[];
	/** False: nobody released its bots and its one-time schedule. */
	release?: boolean;
	/** Changes the world after the service started and before the page opens. */
	arrange?(fake: FakeWorkspace): void | Promise<void>;
	/**
	 * Who reads the device. `status`: a computer that can't reach it and only
	 * has its published status. `locked`: one without the keys for it.
	 */
	reader?: "status" | "locked";
	tab?: "status" | "configuration" | "endpoint";
	mount?: Pick<MountDevicesOptions, "backend" | "overlays" | "widthBucket">;
}

/** The service's row as its device holds it. */
export function shopRow(fake: FakeWorkspace): PlacementStatusPlus {
	const row = fake.agent(SHOP.device).placement(SHOP.service);
	if (!row) throw new Error("The device has no shop-assistant service.");
	return row;
}

/** Stops the service the way a device reports it afterwards, keeping what its row lists. */
export function stopShop(fake: FakeWorkspace) {
	Object.assign(shopRow(fake), {
		desired_state: "stopped",
		observed_state: "stopped",
		running_replicas: 0,
		ready_replicas: 0,
	});
}

/** Shop Assistant on edge-berlin-01, served and started as a deploy leaves it, opened on a tab of its service page. */
export async function openShop({
	events,
	release,
	arrange,
	reader,
	tab = "status",
	mount,
	...options
}: ShopOpenOptions = {}) {
	const world = await createFakeWorkspace(undefined, options);
	await serveShopOnEdge(world, {
		...(events ? { events } : {}),
		...(release === false ? { release } : {}),
	});
	await arrange?.(world);
	world.api.hub.publishStatus(SHOP.device, world.agent(SHOP.device));
	if (reader === "status") world.agent(SHOP.device).online = false;
	// The same hub and device, read by another computer.
	const fake = reader
		? await createFakeWorkspace(undefined, {
				api: world.api,
				viewFacts: false,
				...(reader === "locked" ? { unlock: "none" as const } : {}),
			})
		: world;
	world.api.commands.length = 0;
	const view = await mountDevices(<Page />, {
		fake,
		search: `device=${SHOP.device}&service=${SHOP.service}&tab=${tab}`,
		...mount,
	});
	await view.settle();
	return { ...view, world };
}

export type ShopView = Awaited<ReturnType<typeof openShop>>;

export const text = (root: ParentNode | null | undefined) =>
	(root?.textContent ?? "").replace(/\s+/g, " ").trim();
