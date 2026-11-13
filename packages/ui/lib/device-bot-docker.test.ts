import { describe, expect, test } from "bun:test";
import { deviceRow } from "../components/bots-tab/testing/mount-with-backend";
import { botDockerState } from "./device-bot-docker";
import { parseDeviceRow } from "./devices";

const now = 1_800_000_000_000;
const device = (docker: unknown) =>
	parseDeviceRow(deviceRow("device", { bot_docker: docker }));

describe("Docker availability on the selected device", () => {
	test("requires a recent positive daemon report", () => {
		expect(
			botDockerState(
				device({ installed: true, available: true, seen_at: now }),
				now,
			),
		).toBe("ready");
		expect(
			botDockerState(
				device({ installed: true, available: true, seen_at: now - 180_001 }),
				now,
			),
		).toBe("offline");
		expect(
			botDockerState(
				device({ installed: true, available: true, seen_at: now + 60_000 }),
				now,
			),
		).toBe("offline");
	});
	test("missing or malformed status never enables Docker", () => {
		expect(botDockerState(undefined, now)).toBe("choose_device");
		for (const status of [
			undefined,
			null,
			{},
			{ installed: "yes", available: true, seen_at: now },
		]) {
			expect(botDockerState(device(status), now)).toBe("unknown");
		}
	});
	test("an installed CLI alone is insufficient", () => {
		expect(
			botDockerState(
				device({ installed: false, available: true, seen_at: now }),
				now,
			),
		).toBe("not_installed");
		expect(
			botDockerState(
				device({ installed: true, available: false, seen_at: now }),
				now,
			),
		).toBe("unreachable");
	});
	test.each([
		["docker_remote_endpoint", "remote_endpoint"],
		["docker_linux_required", "linux_required"],
		["docker_image_unavailable", "image_unavailable"],
		["docker_hub_unreachable", "hub_unreachable"],
	] as const)("explains %s without enabling execution", (reason, expected) => {
		expect(
			botDockerState(
				device({ installed: true, available: false, seen_at: now, reason }),
				now,
			),
		).toBe(expected);
	});
});
