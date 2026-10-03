/*
 * Shared setup of the device screen's DOM tests. Import it dynamically, after
 * `installDom()`: it loads react-dom through the mount helper.
 */
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import { useDevicesRoute } from "../routing/use-devices-route";
import { useActivityTray } from "../shell/activity-tray";
import {
	type MountDevicesOptions,
	cleanupDevices,
	mountDevices,
	preloadDevices,
} from "../testing/mount-devices";
import { useOverlayStore } from "../workspace/overlay-store";
import { DeviceScreen } from "./device-screen";

await preloadDevices();

export const IDS = SAMPLE_IDS;
export const APPS = SAMPLE_APPS;

function Page() {
	const { route, scope } = useDevicesRoute();
	if (route.screen !== "device")
		return <p data-left="">{`left:${route.screen}`}</p>;
	return <DeviceScreen route={route} scope={scope} deviceId={route.deviceId} />;
}

export interface OpenOptions extends MountDevicesOptions {
	tab?: string;
	action?: "revoke" | "rename";
	/** Open the page from this app's settings. */
	app?: string;
}

export async function openDevice(deviceId: string, options: OpenOptions = {}) {
	const { tab = "overview", action, app, ...rest } = options;
	const params = [
		app ? `id=${app}` : "",
		`device=${deviceId}`,
		`tab=${tab}`,
		action ? `action=${action}` : "",
	].filter(Boolean);
	const view = await mountDevices(<Page />, {
		search: params.join("&"),
		...(app ? { host: "app" as const } : {}),
		...rest,
	});
	await view.settle();
	return view;
}

export type DeviceView = Awaited<ReturnType<typeof openDevice>>;

export async function resetDevices(): Promise<void> {
	await cleanupDevices();
	useActivityTray.getState().setOpen(false);
	useOverlayStore.getState().close();
}

export const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

/** R2: at most one coral action per view. */
export const primaries = (root: ParentNode = document) =>
	root.querySelectorAll("[data-dv-primary]").length;

export const commandTypes = (view: DeviceView) =>
	view.fake.api.commands.map(([, type]) => type);

/** The last in-area navigation as `[mode, href]`. */
export const lastNavigation = (view: DeviceView) => {
	const last = view.navigations.at(-1);
	return last ? ([last.mode, last.href] as const) : undefined;
};

/** R3: wire values, condition keys and gate codes never reach the screen. */
export const MACHINE =
	/crash_looping|update_in_progress|stopped_by_user|failed_stopped|rolled_back|rollout_|placement|replica|\bgrant\b|activating|validating|offline_writes|cloud_approval|unlock_required|never_reported|no_keys_here|identity_mismatch|host_operation|update_agent|linux_sandbox|trusted_process/;

/** Commands only a newer agent understands (plan §3.4.3). */
export const NEWER_COMMANDS = [
	"host_operation",
	"rollout_history",
	"operations",
	"metrics_history",
	"offline_queue_operations",
	"offline_queue_lookup",
	"artifact",
];

export { useOverlayStore, useActivityTray };
