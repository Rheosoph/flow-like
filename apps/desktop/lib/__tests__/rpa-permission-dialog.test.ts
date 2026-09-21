// @vitest-environment happy-dom
import {
	type ComponentProps,
	type PropsWithChildren,
	act,
	createElement,
} from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("lucide-react", () => ({
	AlertCircle: () => null,
	Check: () => null,
	ShieldAlert: () => null,
}));
vi.mock("@flow-like/flow-like-ui", async () => {
	const { createElement } = await import("react");
	const Box = ({ children }: PropsWithChildren) =>
		createElement("div", null, children);
	return {
		AlertDialog: ({ open, children }: PropsWithChildren<{ open: boolean }>) =>
			open ? createElement("div", null, children) : null,
		AlertDialogContent: Box,
		AlertDialogDescription: Box,
		AlertDialogFooter: Box,
		AlertDialogHeader: Box,
		AlertDialogTitle: Box,
		Button: ({ children, onClick, disabled }: ComponentProps<"button">) =>
			createElement("button", { onClick, disabled }, children),
	};
});
import { RpaPermissionDialog } from "../../components/rpa/rpa-permission-dialog";
let root: Root;
let host: HTMLDivElement;
const required = ["input_monitoring"] as const;
const denied = {
	all_granted: false,
	platform: "macos",
	capabilities: [
		{
			capability: "input_monitoring",
			state: "denied",
			detail: "Enable Input Monitoring",
			can_request: true,
		},
	],
};
beforeEach(() => {
	vi.clearAllMocks();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
});
async function mount(granted = vi.fn(), close = vi.fn()) {
	await act(async () =>
		root.render(
			createElement(RpaPermissionDialog, {
				open: true,
				required: [...required],
				onPermissionsGranted: granted,
				onOpenChange: close,
			}),
		),
	);
	return { granted, close };
}
test("denied access offers grant, recheck, and cancellation without bypass", async () => {
	mocks.invoke.mockResolvedValue(denied);
	const { granted, close } = await mount();
	expect(host.textContent).toContain("Input Monitoring");
	expect(host.textContent).not.toContain("Continue Anyway");
	expect(granted).not.toHaveBeenCalled();
	const cancel = [...host.querySelectorAll("button")].find(
		(button) => button.textContent === "Cancel",
	);
	if (!cancel) throw new Error("Cancel button did not render");
	await act(async () => cancel.click());
	expect(close).toHaveBeenCalledExactlyOnceWith(false);
});
test("successful check completes once without a contradictory close callback", async () => {
	mocks.invoke.mockResolvedValue({ ...denied, all_granted: true });
	const { granted, close } = await mount();
	await act(async () => window.dispatchEvent(new Event("focus")));
	expect(granted).toHaveBeenCalledTimes(1);
	expect(close).not.toHaveBeenCalled();
});
test("late permission response cannot resume after cancellation", async () => {
	const deferred = Promise.withResolvers<typeof denied>();
	mocks.invoke.mockReturnValue(deferred.promise);
	const { granted, close } = await mount();
	const cancel = [...host.querySelectorAll("button")].find(
		(button) => button.textContent === "Cancel",
	);
	if (!cancel) throw new Error("Cancel button did not render");
	await act(async () => cancel.click());
	await act(async () => deferred.resolve({ ...denied, all_granted: true }));
	expect(close).toHaveBeenCalledExactlyOnceWith(false);
	expect(granted).not.toHaveBeenCalled();
});
test("permission transport failure never enables execution", async () => {
	mocks.invoke.mockRejectedValue(new Error("IPC unavailable"));
	const { granted } = await mount();
	expect(host.textContent).toContain("IPC unavailable");
	expect(granted).not.toHaveBeenCalled();
});

test("an old grant request cannot complete a later permission dialog", async () => {
	const deferred = Promise.withResolvers<boolean>();
	mocks.invoke.mockImplementation((command: string) =>
		command === "request_rpa_permission"
			? deferred.promise
			: Promise.resolve(denied),
	);
	const { granted } = await mount();
	const grant = [...host.querySelectorAll("button")].find(
		(button) => button.textContent === "Grant access",
	);
	if (!grant) throw new Error("Grant button did not render");
	await act(async () => grant.click());
	await act(async () =>
		root.render(
			createElement(RpaPermissionDialog, {
				open: false,
				onOpenChange: vi.fn(),
			}),
		),
	);
	await mount(granted);
	const checks = mocks.invoke.mock.calls.filter(
		([command]) => command === "check_rpa_permissions",
	).length;
	mocks.invoke.mockResolvedValue({ ...denied, all_granted: true });
	await act(async () => deferred.resolve(true));
	expect(granted).not.toHaveBeenCalled();
	expect(
		mocks.invoke.mock.calls.filter(
			([command]) => command === "check_rpa_permissions",
		),
	).toHaveLength(checks);
});
