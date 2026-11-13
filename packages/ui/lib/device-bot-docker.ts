import { BOT_HOST_FRESH_MS } from "./device-bot-host";
import type { DeviceRow } from "./devices";

export type BotDockerState =
	| "ready"
	| "choose_device"
	| "unknown"
	| "offline"
	| "not_installed"
	| "unreachable"
	| "remote_endpoint"
	| "linux_required"
	| "hub_unreachable"
	| "image_unavailable";

/** Docker must be ready on the chosen device, with a recent report from its agent. */
export function botDockerState(
	device: DeviceRow | undefined,
	now = Date.now(),
): BotDockerState {
	if (!device) return "choose_device";
	const docker = device.bot_docker;
	if (!docker) return "unknown";
	if (
		device.status !== "active" ||
		!Number.isFinite(docker.seen_at) ||
		docker.seen_at > now + 30_000 ||
		now - docker.seen_at > BOT_HOST_FRESH_MS
	)
		return "offline";
	if (!docker.installed) return "not_installed";
	if (docker.available) return "ready";
	switch (docker.reason) {
		case "docker_remote_endpoint":
			return "remote_endpoint";
		case "docker_linux_required":
			return "linux_required";
		case "docker_image_unavailable":
			return "image_unavailable";
		case "docker_hub_unreachable":
			return "hub_unreachable";
		default:
			return "unreachable";
	}
}
