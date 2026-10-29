import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	byRole,
	click,
	installDom,
	queryByRole,
} from "./settings/devices/testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"./settings/devices/testing/mount-devices"
);
await preloadDevices();
const { createFakeWorkspace } = await import(
	"./settings/devices/testing/fake-workspace"
);
const { getApiOrigin } = await import("../lib/api-url");
const { PatSelectorDialog, clearStoredPat, storedPatKey } = await import(
	"./pat-selector-dialog"
);

afterEach(async () => {
	localStorage.clear();
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const STORED = "pat_tok_mine.secret";

async function mountDialog(
	store: (hub: string, account: string) => void,
	open = true,
) {
	const selected: string[] = [];
	const fake = await createFakeWorkspace();
	store(getApiOrigin(fake.profile), fake.scope.account);
	const view = await mountDevices(
		() => (
			<PatSelectorDialog
				open={open}
				onOpenChange={() => undefined}
				onPatSelected={(pat) => selected.push(pat)}
			/>
		),
		{
			fake,
			providers: false,
			backend: {
				profile: fake.profile,
				userState: {
					getProfile: async () => fake.profile,
					getPATs: async () => [
						{
							id: "tok_mine",
							name: "Desktop triggers",
							created_at: "2026-10-01T00:00:00Z",
							valid_until: null,
							permission: 2,
						},
					],
				},
			} as never,
		},
	);
	return { ...view, selected };
}

const offered = () =>
	document.body.textContent?.includes("Stored Token Available") ?? false;

describe("the stored personal access token", () => {
	test("is offered on the hub and to the account it was stored for", async () => {
		const view = await mountDialog((hub, account) =>
			localStorage.setItem(storedPatKey(hub, account), STORED),
		);
		expect(offered()).toBe(true);
		await click(byRole("radio", "Use Stored Token"));
		await click(byRole("button", "Use Stored Token"));
		expect(view.selected).toEqual([STORED]);
	});

	test("is never offered to another account, on another hub, or when stored before it was scoped", async () => {
		await mountDialog((hub, account) => {
			localStorage.setItem(storedPatKey(hub, `${account}_other`), STORED);
			localStorage.setItem(
				storedPatKey("https://hub.elsewhere.test", account),
				STORED,
			);
			localStorage.setItem("flow-like-selected-pat", STORED);
		});
		expect(offered()).toBe(false);
		await click(byRole("radio", "Use Stored Token"));
		expect(queryByRole("button", "Use Stored Token")).toBeNull();
	});

	test("stored before it was scoped is removed once the dialog mounts, even closed", async () => {
		let own = "";
		await mountDialog((hub, account) => {
			own = storedPatKey(hub, account);
			localStorage.setItem(own, STORED);
			localStorage.setItem("flow-like-selected-pat", STORED);
		}, false);
		expect(localStorage.getItem("flow-like-selected-pat")).toBeNull();
		expect(localStorage.getItem(own)).toBe(STORED);
	});

	test("clearing forgets every stored token, scoped or not", () => {
		localStorage.setItem(storedPatKey("https://a.test", "user_a"), STORED);
		localStorage.setItem(storedPatKey("https://b.test", "user_b"), STORED);
		localStorage.setItem("flow-like-selected-pat", STORED);
		localStorage.setItem("flow-like:language", "de");
		clearStoredPat();
		expect(Object.keys(localStorage)).toEqual(["flow-like:language"]);
	});
});
