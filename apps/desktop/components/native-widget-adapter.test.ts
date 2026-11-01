import { beforeEach, describe, expect, it, vi } from "vitest";
import { appBuildFingerprint } from "../../../packages/ui/lib/app-build/fingerprint";

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
	stage: vi.fn(),
	surface: {
		surfaceId: "page",
		kind: "page",
		appId: "app",
		pageId: "page",
		boardId: "board",
		currentComponents: [
			{
				id: "root",
				component: { type: "text", content: { literalString: "Unsaved" } },
			},
		],
		currentCanvasSettings: { customCss: ".draft {}" },
		selectedComponentIds: ["root"],
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
vi.mock(
	"@flow-like/flow-like-ui/lib/app-build/fingerprint",
	async () => import("../../../packages/ui/lib/app-build/fingerprint"),
);
vi.mock("@flow-like/flow-like-ui/state/assistant-surface", () => ({
	useAssistantSurface: {
		getState: () => ({ widgetSurface: mocks.surface }),
		subscribe: () => () => {},
	},
}));
vi.mock("@flow-like/flow-like-ui/state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: {
		getState: () => ({
			activeConversationId: "conversation",
			pendingComponents: null,
			pendingComponentsOwnerRunId: null,
			setPendingComponents: mocks.stage,
		}),
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
	mocks.invoke
		.mockReset()
		.mockImplementation(async (command) =>
			command === "flowpilot_capture_widget_snapshot"
				? { profile_id: "profile" }
				: null,
		);
	mocks.info.mockReset();
	mocks.stage.mockReset();
	mocks.listen.mockReset().mockImplementation(async (_event, callback) => {
		mocks.onProposal = callback;
		return () => {};
	});
	mocks.surface.currentCanvasSettings = { customCss: ".draft {}" };
	vi.stubGlobal("window", {
		setInterval: vi.fn().mockReturnValue(1),
		clearInterval: vi.fn(),
	});
});
function proposal(fingerprint: string) {
	return {
		proposal_id: "proposal",
		profile_id: "profile",
		owner: { ...mocks.owner },
		run_id: "run",
		conversation_id: "conversation",
		expected_fingerprint: fingerprint,
		surface_id: "page",
		app_id: "app",
		components: [
			{
				id: "root",
				component: { type: "text", content: { literalString: "Proposed" } },
			},
		],
	};
}
describe("native widget presentation", () => {
	it("captures unsaved components, stylesheet and selection before execution", async () => {
		const { syncNativeWidgetSnapshot } = await import(
			"./native-widget-adapter"
		);
		await syncNativeWidgetSnapshot();
		expect(mocks.invoke).toHaveBeenCalledWith(
			"flowpilot_capture_widget_snapshot",
			{ snapshot: mocks.surface, expectedOwner: mocks.owner },
		);
	});
	it("never stages a proposal over a draft that changed during generation", async () => {
		const {
			installNativeWidgetAdapter,
			syncNativeWidgetSnapshot,
			widgetSnapshot,
		} = await import("./native-widget-adapter");
		const fingerprint = appBuildFingerprint(
			"widget-surface-v1",
			widgetSnapshot(mocks.surface as Parameters<typeof widgetSnapshot>[0]),
		);
		const cleanup = installNativeWidgetAdapter();
		await syncNativeWidgetSnapshot();
		mocks.surface.currentCanvasSettings = { customCss: ".user-change {}" };
		mocks.onProposal({ payload: proposal(fingerprint) });
		await vi.waitFor(() => expect(mocks.info).toHaveBeenCalled());
		expect(mocks.stage).not.toHaveBeenCalled();
		cleanup();
	});
	it("stages matching components in the owning run review card", async () => {
		const {
			installNativeWidgetAdapter,
			syncNativeWidgetSnapshot,
			widgetSnapshot,
		} = await import("./native-widget-adapter");
		const fingerprint = appBuildFingerprint(
			"widget-surface-v1",
			widgetSnapshot(mocks.surface as Parameters<typeof widgetSnapshot>[0]),
		);
		const cleanup = installNativeWidgetAdapter();
		await syncNativeWidgetSnapshot();
		const candidate = proposal(fingerprint);
		mocks.onProposal({ payload: candidate });
		await vi.waitFor(() =>
			expect(mocks.stage).toHaveBeenCalledWith(
				"run",
				expect.objectContaining({
					components: candidate.components,
					surfaceId: "page",
					appId: "app",
				}),
			),
		);
		cleanup();
	});
});

it("rejects a queued snapshot after the signed-in account changes", async () => {
	const { syncNativeWidgetSnapshot } = await import("./native-widget-adapter");
	const queued = syncNativeWidgetSnapshot();
	mocks.owner.subject = "bob";
	await expect(queued).rejects.toThrow("profile changed");
	expect(mocks.invoke).not.toHaveBeenCalled();
});
