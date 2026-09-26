// @vitest-environment happy-dom
import { type ComponentProps, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

import type { RpaConsentDialog } from "../../components/rpa/rpa-consent-dialog";
import type { RpaPermissionDialog } from "../../components/rpa/rpa-permission-dialog";
type PermissionProps = ComponentProps<typeof RpaPermissionDialog>;
type ConsentProps = ComponentProps<typeof RpaConsentDialog>;
const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	permission: undefined as PermissionProps | undefined,
	consent: undefined as ConsentProps | undefined,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../components/rpa/rpa-permission-dialog", () => ({
	RpaPermissionDialog: (props: PermissionProps) => {
		mocks.permission = props;
		return null;
	},
}));
vi.mock("../../components/rpa/rpa-consent-dialog", () => ({
	RpaConsentDialog: (props: ConsentProps) => {
		mocks.consent = props;
		return null;
	},
}));
import {
	ensureRpaSystemPermissions,
	requestRpaAutomationConsent,
} from "../../components/rpa/rpa-consent";
import { RpaPermissionProvider } from "../../components/rpa/rpa-permission-provider";

function permission() {
	if (!mocks.permission) throw new Error("Permission dialog did not render");
	return mocks.permission;
}
function consent() {
	if (!mocks.consent) throw new Error("Consent dialog did not render");
	return mocks.consent;
}

let root: Root;
beforeEach(async () => {
	vi.clearAllMocks();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "get_rpa_requirements")
			return {
				required: ["input_control"],
				revision: "revision-a",
				identity: "profile-a",
				approved: false,
			};
		if (command === "check_rpa_permissions") return { all_granted: true };
	});
	root = createRoot(document.createElement("div"));
	await act(async () => root.render(createElement(RpaPermissionProvider)));
});
afterEach(async () => {
	await act(async () => root.unmount());
});

test("concurrent approvals are queued and independently completed", async () => {
	let first!: Promise<boolean>;
	let second!: Promise<boolean>;
	await act(async () => {
		first = requestRpaAutomationConsent({
			appId: "app",
			boardId: "first",
			context: "execution",
		});
		second = requestRpaAutomationConsent({
			appId: "app",
			boardId: "second",
			context: "execution",
		});
	});
	expect(consent().boardId).toBe("first");
	await act(async () => {
		await consent().onConfirm("none");
	});
	expect(await first).toBe(true);
	expect(consent().boardId).toBe("second");
	await act(async () => consent().onCancel());
	expect(await second).toBe(false);
	expect(consent().open).toBe(false);
});

test("grant binds current app, board, version, event, revision, and selected scope", async () => {
	let pending!: Promise<boolean>;
	await act(async () => {
		pending = requestRpaAutomationConsent({
			appId: "app",
			boardId: "board",
			eventId: "event",
			version: [1, 2, 3],
			context: "event_registration",
		});
	});
	await act(async () => {
		await consent().onConfirm("event");
	});
	expect(await pending).toBe(true);
	expect(mocks.invoke).toHaveBeenCalledWith("grant_rpa_automation", {
		appId: "app",
		boardId: "board",
		eventId: "event",
		version: [1, 2, 3],
		expectedRevision: "revision-a",
		expectedIdentity: "profile-a",
		scope: "event",
	});
	expect(mocks.invoke).toHaveBeenCalledWith("check_rpa_permissions", {
		required: ["input_control"],
	});
});

test("native grant rejection keeps the request open and does not run", async () => {
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "get_rpa_requirements")
			return {
				required: ["browser"],
				revision: "old",
				identity: "profile-a",
				approved: false,
			};
		if (command === "grant_rpa_automation") throw new Error("Workflow changed");
	});
	let resolved = false;
	await act(async () => {
		void requestRpaAutomationConsent({
			appId: "app",
			boardId: "board",
			context: "execution",
		}).then(() => {
			resolved = true;
		});
	});
	await act(async () => {
		await consent().onConfirm("board");
	});
	expect(resolved).toBe(false);
	expect(consent().open).toBe(true);
	expect(consent().error).toBe("Workflow changed");
	await act(async () => consent().onCancel());
});

test("denied capability cannot continue anyway and Cancel resolves false", async () => {
	mocks.invoke.mockResolvedValue({ all_granted: false });
	let pending!: Promise<boolean>;
	await act(async () => {
		pending = ensureRpaSystemPermissions({ required: ["input_monitoring"] });
	});
	expect("onContinueAnyway" in permission()).toBe(false);
	expect(permission().required).toEqual(["input_monitoring"]);
	await act(async () => permission().onOpenChange(false));
	expect(await pending).toBe(false);
});

test("success and close complete once and do not cancel the next request", async () => {
	mocks.invoke.mockResolvedValue({ all_granted: false });
	const results: boolean[] = [];
	const listener = (event: Event) =>
		results.push((event as CustomEvent).detail.granted);
	window.addEventListener("flow:rpa-permissions-result", listener);
	let first!: Promise<boolean>;
	let second!: Promise<boolean>;
	await act(async () => {
		first = ensureRpaSystemPermissions({ required: ["screen_capture"] });
		second = ensureRpaSystemPermissions({ required: ["accessibility"] });
	});
	await act(async () => {
		const dialog = permission();
		dialog.onPermissionsGranted?.();
		dialog.onOpenChange(false);
	});
	expect(await first).toBe(true);
	expect(results).toEqual([true]);
	expect(permission().required).toEqual(["accessibility"]);
	await act(async () => permission().onOpenChange(false));
	expect(await second).toBe(false);
	window.removeEventListener("flow:rpa-permissions-result", listener);
});

test("unmount resolves every waiting request as denied", async () => {
	let pending!: Promise<boolean>;
	await act(async () => {
		pending = requestRpaAutomationConsent({
			appId: "app",
			boardId: "board",
			context: "execution",
		});
	});
	await act(async () => root.unmount());
	expect(await pending).toBe(false);
});

test("a browser workflow requests no unrelated desktop permissions", async () => {
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "get_rpa_requirements")
			return {
				required: ["browser"],
				revision: "browser-revision",
				identity: "profile-a",
				approved: true,
			};
		if (command === "check_rpa_permissions") return { all_granted: true };
	});
	expect(
		await requestRpaAutomationConsent({
			appId: "app",
			boardId: "browser",
			context: "execution",
		}),
	).toBe(true);
	expect(mocks.invoke).toHaveBeenCalledWith("check_rpa_permissions", {
		required: ["browser"],
	});
	expect(consent().open).toBe(false);
});
