// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectUserSearch } from "../../hooks/use-project-user-search";
import { PackagePermissionBits } from "../../lib/permission/wasm-package-permission";
import { PackageInviteDialog } from "./package-invite-dialog";

const search = {
	results: [
		{ user: { id: "member", name: "Mia Member", created_at: "" } },
		{ user: { id: "outsider", name: "Otto Outsider", created_at: "" } },
	].map((entry) => ({ ...entry, fromProject: false })),
	canSearchDirectory: true,
	isSearchingDirectory: false,
	directoryError: null,
	contactsError: null,
	isLoadingContacts: false,
} as unknown as ProjectUserSearch;
const searchCalls: unknown[][] = [];

vi.mock("../../hooks/use-project-user-search", () => ({
	useProjectUserSearch: (...args: unknown[]) => {
		searchCalls.push(args);
		return search;
	},
}));

let root: Root;
let container: HTMLDivElement;

async function render(onInvite: () => Promise<boolean>) {
	const onOpenChange = vi.fn();
	await act(async () =>
		root.render(
			<PackageInviteDialog
				open
				onOpenChange={onOpenChange}
				onInvite={onInvite}
				memberIds={new Set(["member"])}
			/>,
		),
	);
	return onOpenChange;
}

function inviteButtons() {
	return [...document.body.querySelectorAll("button")].filter(
		(button) => button.textContent?.trim() === "Invite",
	);
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	searchCalls.length = 0;
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

describe("package invite dialog", () => {
	it("searches the whole directory and hides current members", async () => {
		await render(async () => true);
		expect(searchCalls.at(-1)?.[0]).toBeUndefined();
		expect(document.body.textContent).toContain("Otto Outsider");
		expect(document.body.textContent).not.toContain("Mia Member");
		expect(inviteButtons()).toHaveLength(1);
	});

	it("invites the picked person as a User and closes once it succeeds", async () => {
		const onInvite = vi.fn(async () => true);
		const onOpenChange = await render(onInvite);
		await act(async () => inviteButtons()[0].click());
		expect(onInvite).toHaveBeenCalledWith({
			inviteeId: "outsider",
			permission: PackagePermissionBits.User,
		});
		expect(onOpenChange).toHaveBeenCalledWith(false);
	});

	it("stays open when the invitation fails", async () => {
		const onOpenChange = await render(async () => false);
		await act(async () => inviteButtons()[0].click());
		expect(onOpenChange).not.toHaveBeenCalled();
		expect(inviteButtons()[0].disabled).toBe(false);
	});
});
