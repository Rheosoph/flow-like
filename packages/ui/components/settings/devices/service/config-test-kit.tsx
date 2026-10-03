import { expect } from "bun:test";
import { SAMPLE_IDS } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { useDevicesRoute } from "../routing/use-devices-route";
import { advance } from "../testing/dom-harness";
import {
	type FakeWorkspace,
	type FakeWorkspaceOptions,
	createFakeWorkspace,
} from "../testing/fake-workspace";
import {
	type MountDevicesOptions,
	mountDevices,
} from "../testing/mount-devices";
import { ServiceConfigurationTab } from "./configuration-tab";
import { ServiceEndpointTab } from "./endpoint-tab";
import { ServiceOfflineTab } from "./offline-tab";

/*
 * Shared by the three settings-tab DOM tests. Import it dynamically, after
 * `installDom()`: it pulls in the area's module graph.
 */

export const EDGE = SAMPLE_IDS.edge;
export const STUDIO = SAMPLE_IDS.studio;
export const WAREHOUSE = SAMPLE_IDS.warehouse;
export const LAB = SAMPLE_IDS.lab;

export type SettingsTab = "configuration" | "endpoint" | "offline";
export type Config = Record<string, unknown>;

function Page() {
	const { route, scope } = useDevicesRoute();
	if (route.screen !== "service")
		return <p data-left="">{`left:${route.screen}`}</p>;
	const props = {
		route,
		scope,
		deviceId: route.deviceId,
		serviceId: route.serviceId,
	};
	if (route.tab === "configuration")
		return <ServiceConfigurationTab {...props} />;
	if (route.tab === "endpoint") return <ServiceEndpointTab {...props} />;
	if (route.tab === "offline") return <ServiceOfflineTab {...props} />;
	return <p data-left="">{`tab:${route.tab ?? "status"}`}</p>;
}

export interface OpenOptions extends FakeWorkspaceOptions {
	tab: SettingsTab;
	device: string;
	service: string;
	/** Runs after the fake workspace exists and before the tab mounts. */
	arrange?(fake: FakeWorkspace): void | Promise<void>;
	mount?: Pick<
		MountDevicesOptions,
		"host" | "backend" | "overlays" | "widthBucket"
	>;
	/** App scope: the app whose settings host the area. */
	appId?: string;
}

export async function openTab({
	tab,
	device,
	service,
	arrange,
	mount,
	appId,
	...options
}: OpenOptions) {
	const fake = await createFakeWorkspace(undefined, options);
	await arrange?.(fake);
	fake.api.commands.length = 0;
	const view = await mountDevices(<Page />, {
		fake,
		search: `${appId ? `id=${appId}&` : ""}device=${device}&service=${service}&tab=${tab}`,
		...(appId ? { host: "app" as const } : {}),
		...mount,
	});
	await view.settle();
	return view;
}

export type View = Awaited<ReturnType<typeof openTab>>;

export const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

/** Commands the device's agent received since the tab mounted, by type. */
export const sent = (view: View, deviceId?: string) =>
	view.fake.api.commands
		.filter(([device]) => !deviceId || device === deviceId)
		.map(([, type]) => type);

export const commandsOf = (view: View, type: string) =>
	view.fake.api.commands
		.filter(([, sentType]) => sentType === type)
		.map(([, , command]) => command);

const WRITES = new Set([
	"apply",
	"stage_rollout",
	"activate_rollout",
	"rollout_secret",
	"set_secret",
	"start",
	"stop",
	"restart",
	"scale",
	"remove",
	"offline_queue_retry",
	"offline_queue_skip",
]);

/** Everything that changes the device; reads don't count. */
export const writes = (view: View) =>
	sent(view).filter((type) => WRITES.has(type));

export async function until(check: () => boolean, ms = 6_000) {
	const end = performance.now() + ms;
	while (!check() && performance.now() < end) await advance(25);
	expect(check()).toBe(true);
}

/** The settings the fake device holds for a service, as a copy to change and store back with `setConfig`. */
export async function readConfig(
	fake: FakeWorkspace,
	device: string,
	service: string,
): Promise<Config> {
	const response = await fake.workspace.live.call(device, { lane: "poll" })({
		type: "placement_configuration",
		placement_id: service,
	});
	return structuredClone(
		(response.result as { config: Config }).config,
	) as Config;
}

export function setConfig(
	fake: FakeWorkspace,
	device: string,
	service: string,
	config: Config,
) {
	fake.agent(device).configs.set(service, config);
}

/** Changes the stored settings of a service before the tab opens. */
export async function patchConfig(
	fake: FakeWorkspace,
	device: string,
	service: string,
	patch: (config: Config) => void,
) {
	const config = await readConfig(fake, device, service);
	patch(config);
	setConfig(fake, device, service, config);
}

export const primaries = (root: ParentNode = document) =>
	root.querySelectorAll("[data-dv-primary]").length;

/** R3: wire values and condition keys never reach the screen. */
export const MACHINE =
	/offline_writes|secret_overrides|max_replicas|outcome_unknown|rollout_|placement|replica|\bgrant\b|quarantin|activating|validating|stage_rollout|set_secret|linux_sandbox|trusted_process|table_upsert/;
