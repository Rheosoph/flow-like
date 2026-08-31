import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { IOwnRole } from "../../../state/backend-state";
import { resolveDangerActions } from "./app-danger-zone";

function ownRole(overrides: Partial<IOwnRole> = {}): IOwnRole {
	return {
		role_id: "role_1",
		role_name: "Member",
		permissions: 0,
		is_owner: false,
		can_leave: true,
		...overrides,
	};
}

describe("resolveDangerActions", () => {
	test("an owner may delete but may not quit", () => {
		expect(
			resolveDangerActions(
				ownRole({ role_name: "Owner", is_owner: true, can_leave: false }),
				true,
			),
		).toEqual({ canDelete: true, canLeave: false });
	});

	test("a plain member may quit but may not delete", () => {
		expect(resolveDangerActions(ownRole(), true)).toEqual({
			canDelete: false,
			canLeave: true,
		});
	});

	// The hub's own predicates differ: deleting passes an `Owner` check that
	// `Admin` satisfies, while leaving only needs the `Owner` bit to be absent.
	test("an admin who is not the owner may do both", () => {
		expect(
			resolveDangerActions(
				ownRole({ role_name: "Admin", is_owner: true, can_leave: true }),
				true,
			),
		).toEqual({ canDelete: true, canLeave: true });
	});

	test("a host that forbids editing still cannot suppress quitting", () => {
		expect(resolveDangerActions(ownRole({ is_owner: true }), false)).toEqual({
			canDelete: false,
			canLeave: true,
		});
	});

	test("an unreadable role offers nothing, rather than assuming ownership", () => {
		expect(resolveDangerActions(undefined, true)).toEqual({
			canDelete: false,
			canLeave: false,
		});
	});
});

type QueryStub = { isPending: boolean; data?: IOwnRole };

let queryStub: QueryStub = { isPending: true };

mock.module("../../../hooks", () => ({
	useInvoke: () => queryStub,
}));

mock.module("../../../state/backend-state", () => ({
	useBackend: () => ({ roleState: { getOwnRole: () => Promise.resolve() } }),
}));

// Radix builds both confirmations out of portalled context providers, which a
// sibling suite's own mocking can leave half-wired. These tests are about which
// affordance is offered, so the confirmations are reduced to their triggers.
mock.module("../../verification-dialog", () => ({
	VerificationDialog: ({ children }: { children: React.ReactNode }) => children,
}));

mock.module("../../ui/alert-dialog", () => {
	const passthrough = ({ children }: { children?: React.ReactNode }) => children;
	return {
		AlertDialog: passthrough,
		AlertDialogTrigger: passthrough,
		AlertDialogContent: passthrough,
		AlertDialogHeader: passthrough,
		AlertDialogFooter: passthrough,
		AlertDialogTitle: passthrough,
		AlertDialogDescription: passthrough,
		AlertDialogAction: passthrough,
		AlertDialogCancel: passthrough,
	};
});

afterAll(() => mock.restore());

async function renderZone(stub: QueryStub) {
	queryStub = stub;
	const window = new Window();
	Object.assign(globalThis, {
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		window,
	});
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

	const { AppDangerZone } = await import("./app-danger-zone");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(
			<AppDangerZone
				appId="app_1"
				canEdit
				onDeleted={() => {}}
				onLeft={() => {}}
			/>,
		);
	});
	const text = container.textContent ?? "";
	await act(async () => root.unmount());
	window.close();
	return text;
}

describe("AppDangerZone", () => {
	test("offers a member the way out they actually have", async () => {
		const text = await renderZone({ isPending: false, data: ownRole() });
		expect(text).toContain("Quit project");
		expect(text).not.toContain("Delete app");
	});

	test("offers an owner deletion, and says why quitting is not on the table", async () => {
		const text = await renderZone({
			isPending: false,
			data: ownRole({ is_owner: true, can_leave: false }),
		});
		expect(text).toContain("Delete app");
		expect(text).not.toContain("Quit project");
		expect(text).toContain("An owner cannot quit their own project");
	});

	test("offers nothing destructive while the role is still loading", async () => {
		const text = await renderZone({ isPending: true });
		expect(text).not.toContain("Delete app");
		expect(text).not.toContain("Quit project");
	});

	test("offers nothing destructive when the role could not be read", async () => {
		const text = await renderZone({ isPending: false });
		expect(text).toContain("Could not check your access");
		expect(text).not.toContain("Delete app");
		expect(text).not.toContain("Quit project");
	});
});
