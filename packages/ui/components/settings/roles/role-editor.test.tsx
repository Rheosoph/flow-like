import { afterAll, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { RolePermissions } from "../../../lib/permission/role-permission";
import type { IBackendRole } from "../../../state/backend-state/types";
import {
	ACCESS_LADDERS,
	ROLE_TEMPLATES,
	applyElevation,
	permissionsFromTemplate,
} from "./access-ladders";

// Radix's Switch calls element.closest("form") on mount, which happy-dom's selector parser
// cannot handle. Swap it for a checkbox so these tests assert this component's decisions.
mock.module("../../ui/switch", () => ({
	Switch: ({ checked }: { checked?: boolean }) => (
		<input type="checkbox" readOnly checked={Boolean(checked)} />
	),
}));

afterAll(() => mock.restore());

function roleWith(
	permissions: RolePermissions,
	overrides: Partial<IBackendRole> = {},
): IBackendRole {
	return {
		id: "role_1",
		app_id: "app_1",
		name: "Operator",
		description: "Runs the work day to day.",
		permissions: permissions.toBigInt(),
		attributes: [],
		created_at: "2026-08-02T00:00:00",
		updated_at: "2026-08-02T00:00:00",
		...overrides,
	};
}

async function renderEditor(role: IBackendRole, memberCount?: number) {
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

	const { RoleEditor } = await import("./role-editor");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container);
	await act(async () => {
		root.render(
			<RoleEditor
				role={role}
				memberCount={memberCount}
				isDefault={false}
				knownAttributes={["oncall", "region:eu"]}
				onChange={() => {}}
				onDuplicate={() => {}}
				onDelete={() => {}}
				onSetDefault={() => {}}
			/>,
		);
	});
	const text = container.textContent ?? "";
	await act(async () => root.unmount());
	window.close();
	return text;
}

describe("RoleEditor", () => {
	test("states what an operator can do, in plain language", async () => {
		const operator = ROLE_TEMPLATES.find((entry) => entry.name === "Operator");
		if (!operator) throw new Error("template missing");
		const text = await renderEditor(
			roleWith(permissionsFromTemplate(operator)),
			9,
		);

		expect(text).toContain("Operator can ");
		expect(text).toContain("open and run workflows");
		expect(text).toContain("trigger events");
		// Every ladder is present with its level control.
		for (const ladder of ACCESS_LADDERS) {
			expect(text).toContain(ladder.label);
		}
	});

	test("reports effective access for an admin instead of the stored bit", async () => {
		const text = await renderEditor(
			roleWith(applyElevation(new RolePermissions(), "admin"), {
				name: "Administrator",
			}),
			2,
		);

		expect(text).toContain(
			"Administrator can do everything except transfer or delete the app.",
		);
		// 27 of 28 — the old card claimed 1 of 28 for exactly this role.
		expect(text).toContain("27/28");
		expect(text).toContain("Granted through");
	});

	test("marks a hand-rolled permission set as custom", async () => {
		// Edit-without-view: unreachable from the levels, only via advanced switches.
		const text = await renderEditor(
			roleWith(new RolePermissions().insert(RolePermissions.WriteBoards)),
		);

		expect(text).toContain("Custom");
		expect(text).toContain("a custom mix in workflows");
		// Advanced opens itself so the exact switches are reachable.
		expect(text).toContain("Create, modify, and delete flow boards");
	});

	test("says a role with nothing granted has no access", async () => {
		const text = await renderEditor(roleWith(new RolePermissions()));
		expect(text).toContain("has no access to anything in this app");
		expect(text).toContain("Cannot ");
	});

	test("offers unused attributes for reuse and hides ones already applied", async () => {
		const text = await renderEditor(
			roleWith(new RolePermissions(), { attributes: ["oncall"] }),
		);
		expect(text).toContain("Already in use:");
		expect(text).toContain("region:eu");
	});
});
