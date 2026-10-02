import { describe, expect, test } from "bun:test";
import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { IApp } from "../schema/app/app";
import type { PreparedProjectArtifact } from "./artifacts";
import {
	DeployPrepareDesktopOnlyError,
	type DeployPrepareInput,
	type DeployPrepareSteps,
	prepareDeployBundle,
} from "./deploy-prepare";
import type { ApprovedOnlineMetadata } from "./online-metadata";
import type { ExportCommands } from "./project-export";

const backend = { name: "backend" } as unknown as IBackendState;
const profile = { id: "profile" } as IProfile;
const artifact = (name: string) =>
	({ descriptor: { project_id: name } }) as unknown as PreparedProjectArtifact;

function approvedApp(change: Partial<IApp> = {}): ApprovedOnlineMetadata {
	return {
		app: { id: "app", bits: [], packages: {}, ...change } as unknown as IApp,
	} as ApprovedOnlineMetadata;
}

function harness(approved = approvedApp()) {
	const calls: string[] = [];
	const commands = { name: "commands" } as unknown as ExportCommands;
	const steps: DeployPrepareSteps = {
		async metadata(appId, withBackend, withProfile) {
			expect([withBackend, withProfile]).toEqual([backend, profile]);
			calls.push(`metadata ${appId}`);
			return approved;
		},
		async dependencies(app, _backend, _profile, _signal, given) {
			expect(given).toBe(approved);
			calls.push(`dependencies ${app.id}`);
			return {
				artifact: artifact("browser"),
			} as Awaited<ReturnType<DeployPrepareSteps["dependencies"]>>;
		},
		async desktop(appId, withCommands, _signal, given) {
			expect(withCommands).toBe(commands);
			calls.push(`desktop ${appId} ${given ? "approved" : "copy"}`);
			return {
				artifact: artifact("native"),
				assets: { bit_pins: [], package_pins: [] },
				release: async () => {
					calls.push("release");
				},
			};
		},
		async exportCommands(onlineApp, account) {
			calls.push(`commands ${onlineApp?.id ?? "-"} ${account ?? "-"}`);
			return commands;
		},
	};
	const input = (change: Partial<DeployPrepareInput>): DeployPrepareInput => ({
		appId: "app",
		mode: "online",
		desktop: false,
		backend,
		profile,
		onPhase: (phase) => calls.push(`phase ${phase}`),
		steps,
		...change,
	});
	return { calls, input, approved };
}

describe("online apps", () => {
	test("the browser collects the approved definitions and their files itself", async () => {
		const h = harness(approvedApp({ bits: ["bit"] as never }));
		const bundle = await prepareDeployBundle(h.input({}));
		expect(bundle).toEqual({
			artifact: artifact("browser"),
			approved: h.approved,
		});
		expect(h.calls).toEqual([
			"phase read_hub",
			"metadata app",
			"phase collect",
			"dependencies app",
		]);
	});

	test("the desktop app exports natively only when the app pins models or packages", async () => {
		const plain = harness();
		await prepareDeployBundle(plain.input({ desktop: true }));
		expect(plain.calls).toContain("dependencies app");

		const pinned = harness(approvedApp({ packages: { pkg: {} } as never }));
		const bundle = await prepareDeployBundle(pinned.input({ desktop: true }));
		expect(bundle.artifact).toEqual(artifact("native"));
		expect(bundle.approved).toBe(pinned.approved);
		expect(pinned.calls).toEqual([
			"phase read_hub",
			"metadata app",
			"phase collect",
			"commands app -",
			"desktop app approved",
		]);
		await bundle.release?.();
		expect(pinned.calls.at(-1)).toBe("release");
	});

	test("the caller's check of the approved definitions stops the run before anything is collected", async () => {
		const h = harness();
		const refused = new Error("event not approved");
		await expect(
			prepareDeployBundle(
				h.input({
					onApproved: (approved) => {
						expect(approved).toBe(h.approved);
						throw refused;
					},
				}),
			),
		).rejects.toBe(refused);
		expect(h.calls).toEqual(["phase read_hub", "metadata app"]);
	});
});

describe("local-only apps", () => {
	test("the desktop app exports this computer's copy for the signed-in account", async () => {
		const h = harness();
		const bundle = await prepareDeployBundle(
			h.input({ mode: "offline", desktop: true, account: "owner" }),
		);
		expect(bundle.artifact).toEqual(artifact("native"));
		expect(bundle).not.toHaveProperty("approved");
		expect(h.calls).toEqual([
			"phase read_app",
			"commands - owner",
			"desktop app copy",
		]);
	});

	test("the browser has no copy to send", async () => {
		const h = harness();
		await expect(
			prepareDeployBundle(h.input({ mode: "offline" })),
		).rejects.toBeInstanceOf(DeployPrepareDesktopOnlyError);
		expect(h.calls).toEqual([]);
	});
});
