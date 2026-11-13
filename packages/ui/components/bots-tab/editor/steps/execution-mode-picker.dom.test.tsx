import { afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	backendWith,
	deviceRow,
	mountWithBackend,
} from "../../testing/mount-with-backend";

let unmount: (() => Promise<void>) | undefined;
afterEach(async () => {
	await unmount?.();
	unmount = undefined;
});

async function mount(docker: unknown, value: "native" | "docker" = "native") {
	const { ExecutionModePicker } = await import("./execution-mode-picker");
	const changes: string[] = [];
	const view = await mountWithBackend(
		<ExecutionModePicker
			location="device"
			deviceId="device"
			value={value}
			onChange={(next) => changes.push(next)}
		/>,
		backendWith([deviceRow("device", { bot_docker: docker })]),
	);
	unmount = view.unmount;
	const radio = (mode: string) => {
		const element = view.container.querySelector<HTMLButtonElement>(
			`button[value="${mode}"]`,
		);
		if (!element) throw new Error(`Missing ${mode} radio`);
		return element;
	};
	return { ...view, radio, changes };
}

describe("choosing Docker for a device bot", () => {
	test("missing Docker disables selection and links installation instructions", async () => {
		const view = await mount({
			installed: false,
			available: false,
			seen_at: Date.now(),
		});
		expect(view.radio("docker").disabled).toBe(true);
		expect(view.container.textContent).toContain(
			"Install Docker on the selected device",
		);
		expect(view.container.querySelector<HTMLAnchorElement>("a")?.href).toBe(
			"https://docs.docker.com/get-started/get-docker/",
		);
		await act(async () => view.radio("docker").click());
		expect(view.changes).toEqual([]);
	});
	test("only a fresh successful check enables Docker", async () => {
		const view = await mount({
			installed: true,
			available: true,
			seen_at: Date.now(),
		});
		expect(view.radio("docker").disabled).toBe(false);
		expect(view.container.textContent).toContain("our prebuilt image");
		await act(async () => view.radio("docker").click());
		expect(view.changes).toEqual(["docker"]);
	});
	test("an unreachable daemon keeps an existing Docker choice without switching to native", async () => {
		const view = await mount(
			{
				installed: true,
				available: false,
				reason: "docker_unreachable",
				seen_at: Date.now(),
			},
			"docker",
		);
		expect(view.radio("docker").disabled).toBe(true);
		expect(view.radio("docker").getAttribute("aria-checked")).toBe("true");
		expect(view.changes).toEqual([]);
		expect(view.container.textContent).toContain("agent cannot connect");
		await act(async () => view.radio("native").click());
		expect(view.changes).toEqual(["native"]);
	});
	test("an old agent has no implied Docker support", async () => {
		const view = await mount(undefined);
		expect(view.radio("docker").disabled).toBe(true);
		expect(view.container.textContent).toContain(
			"Update and start the device agent",
		);
	});
});
