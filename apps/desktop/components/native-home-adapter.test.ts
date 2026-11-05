import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	owner: {
		profileId: "profile",
		hub: "https://hub.test",
		subject: "alice",
		issuer: "issuer",
	},
	listen: vi.fn(),
	info: vi.fn(),
	surface: {
		getSnapshot: () => ({
			profileId: "profile",
			candidateFingerprint: "unsaved-draft",
			editing: true,
			layout: { version: 1, widgets: [] as unknown[] },
		}),
		stageLayout: vi.fn(),
	},
	onProposal: (_event: { payload: Record<string, unknown> }) => {},
}));
vi.mock("../lib/native-snapshot-owner", () => ({
	nativeSnapshotOwner: () => ({ ...mocks.owner }),
	matchesNativeSnapshotOwner: (owner: unknown) =>
		JSON.stringify(owner) === JSON.stringify(mocks.owner),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("sonner", () => ({ toast: { info: mocks.info } }));
vi.mock("@flow-like/flow-like-ui/state/assistant-surface", () => ({
	useAssistantSurface: {
		getState: () => ({ homeSurface: mocks.surface }),
		subscribe: () => () => {},
	},
}));

beforeEach(() => {
	vi.resetModules();
	mocks.owner = {
		profileId: "profile",
		hub: "https://hub.test",
		subject: "alice",
		issuer: "issuer",
	};
	mocks.invoke.mockReset().mockResolvedValue(null);
	mocks.info.mockReset();
	mocks.listen.mockReset().mockImplementation(async (_event, callback) => {
		mocks.onProposal = callback;
		return () => {};
	});
	const snapshot = {
		profileId: "profile",
		candidateFingerprint: "unsaved-draft",
		editing: true,
		layout: { version: 1, widgets: [] },
	};
	mocks.surface = {
		getSnapshot: () => snapshot,
		stageLayout: vi.fn().mockReturnValue({
			status: "stale",
			message: "The visible Home draft changed.",
		}),
	};
	vi.stubGlobal("window", {
		addEventListener: vi.fn(),
		removeEventListener: vi.fn(),
		setInterval: vi.fn().mockReturnValue(1),
		clearInterval: vi.fn(),
	});
});

describe("native Home presentation", () => {
	it("awaits capture of the unsaved layout before a native run starts", async () => {
		const { syncNativeHomeSnapshot } = await import("./native-home-adapter");
		await syncNativeHomeSnapshot();
		expect(mocks.invoke).toHaveBeenCalledWith(
			"flowpilot_capture_home_snapshot",
			{
				snapshot: mocks.surface.getSnapshot(),
				expectedOwner: mocks.owner,
			},
		);
	});

	it("preserves a changed visible draft when a native proposal arrives", async () => {
		const { installNativeHomeAdapter, syncNativeHomeSnapshot } = await import(
			"./native-home-adapter"
		);
		const cleanup = installNativeHomeAdapter();
		await syncNativeHomeSnapshot();
		const layout = { version: 1, widgets: [{ id: "proposed" }] };
		mocks.onProposal({
			payload: {
				profile_id: "profile",
				owner: { ...mocks.owner },
				layout,
				expected_fingerprint: "saved-layout",
				candidate_fingerprint: "proposal",
			},
		});
		expect(mocks.surface.stageLayout).toHaveBeenCalledWith(layout, {
			expectedProfileId: "profile",
			expectedFingerprint: "saved-layout",
		});
		expect(mocks.info).toHaveBeenCalledWith(
			"A FlowPilot Home proposal is waiting for review.",
			{ description: "The visible Home draft changed." },
		);
		expect(
			mocks.invoke.mock.calls.some(
				([command]) => command === "flowpilot_resolve_home_proposal",
			),
		).toBe(false);
		cleanup();
	});
});

it("rejects a queued snapshot after the signed-in account changes", async () => {
	const { syncNativeHomeSnapshot } = await import("./native-home-adapter");
	const queued = syncNativeHomeSnapshot();
	mocks.owner.subject = "bob";
	await expect(queued).rejects.toThrow("profile changed");
	expect(mocks.invoke).not.toHaveBeenCalled();
});
