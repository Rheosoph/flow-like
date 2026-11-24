import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { PackagePermissionBits as P } from "../../lib/permission/wasm-package-permission";
import type {
	PackagePendingInvitation,
	PackageUser,
} from "../../lib/schema/wasm";
import type { PackageAccessModel } from "./package-workspace/use-package-access";

/** The first dynamic import transpiles the tab's graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 60_000;

const actualUserSearch = {
	...(await import("../../hooks/use-project-user-search")),
};
mock.module("../../hooks/use-project-user-search", () => ({
	...actualUserSearch,
	useProjectUserSearch: () => ({
		results: [],
		canSearchDirectory: true,
		isSearchingDirectory: false,
		directoryError: null,
		contactsError: null,
		isLoadingContacts: false,
	}),
}));
afterAll(() => {
	mock.module("../../hooks/use-project-user-search", () => actualUserSearch);
});

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({
		url: "https://app.flow-like.com/store/package-workspace",
	});
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		HTMLImageElement: window.HTMLImageElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		DOMRect: window.DOMRect,
		Range: window.Range,
		Selection: window.Selection,
		HTMLCollection: window.HTMLCollection,
		NodeList: window.NodeList,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		IntersectionObserver: window.IntersectionObserver,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
		ResizeObserver: window.ResizeObserver,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { SyntaxError, TypeError });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	await window.happyDOM.abort();
	restoreGlobals();
});

const member = (userId: string, name: string, permission: number) =>
	({
		id: `row-${userId}`,
		userId,
		name,
		permission,
		grantedAt: "2026-11-01T00:00:00Z",
	}) satisfies PackageUser;

const invitation: PackagePendingInvitation = {
	id: "inv-1",
	inviteeId: "invitee",
	name: "Ida Invitee",
	invitedById: "owner",
	invitedByName: "Olga Owner",
	permission: P.User,
	createdAt: "2026-11-22T00:00:00Z",
	expiresAt: "2099-11-29T00:00:00Z",
};

function mutation() {
	return {
		mutate: mock(() => {}),
		mutateAsync: mock(async () => undefined),
		isPending: false,
	};
}

function model(invitations: PackagePendingInvitation[]) {
	const list = (items: unknown[]) => ({
		list: items,
		isLoading: false,
		isError: false,
		refetch: mock(async () => undefined),
	});
	return {
		users: list([
			member("owner", "Olga Owner", P.Owner),
			member("maint", "Max Maintainer", P.Maintainer),
			member("buyer", "Bea Buyer", P.Buyer),
		]),
		invitations: list(invitations),
		requests: list([]),
		invite: mutation(),
		withdraw: mutation(),
		setRole: mutation(),
		transfer: mutation(),
		remove: mutation(),
		leave: mutation(),
		answerRequest: mutation(),
	};
}

async function render(
	access: ReturnType<typeof model>,
	caller: number,
	selfId: string,
) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { useBackendStore } = await import("../../state/backend-state");
	const { PackageUsersTab } = await import("./package-users-tab");
	useBackendStore.getState().setBackend({
		profile: undefined,
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
		},
	} as never);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<PackageUsersTab
					access={access as unknown as PackageAccessModel}
					callerPermission={caller}
					selfId={selfId}
					onLeft={() => {}}
				/>
			</QueryClientProvider>,
		),
	);
}

function labelled(label: string) {
	return window.document.body.querySelector(
		`[aria-label="${label}"]`,
	) as unknown as HTMLButtonElement | null;
}

async function click(element: HTMLElement | null) {
	const { act } = await import("react");
	await act(async () => element?.click());
}

describe("package people", () => {
	test(
		"lists pending invitations next to the members",
		async () => {
			await render(model([invitation]), P.Owner, "owner");
			const text = window.document.body.textContent ?? "";
			expect(text).toContain("Ida Invitee");
			expect(text).toContain("Pending");
			expect(text).toContain("Invited by Olga Owner");
			expect(text).toContain("Bea Buyer");
			expect(text).toContain("Buyer");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("withdraws and resends an invitation", async () => {
		const access = model([invitation]);
		await render(access, P.Owner, "owner");
		await click(labelled("Withdraw the invitation to Ida Invitee"));
		expect(access.withdraw.mutate).toHaveBeenCalledWith("inv-1");
		await click(labelled("Send the invitation to Ida Invitee again"));
		expect(access.invite.mutate.mock.calls[0]?.[0]).toEqual({
			inviteeId: "invitee",
			permission: P.User,
		});
	});

	test("gives the owner row and buyers no role switch", async () => {
		await render(model([]), P.Owner, "owner");
		expect(labelled("Role of Olga Owner")).toBeNull();
		expect(labelled("Role of Bea Buyer")).toBeNull();
		expect(labelled("Role of Max Maintainer")).not.toBeNull();
		expect(labelled("Actions for Olga Owner")).toBeNull();
	});

	test("lets a maintainer act on users only, withdraw User invites, and leave", async () => {
		await render(model([invitation]), P.Maintainer, "maint");
		expect(labelled("Role of Max Maintainer")).toBeNull();
		expect(labelled("Actions for Max Maintainer")).not.toBeNull();
		expect(labelled("Actions for Bea Buyer")).toBeNull();
		expect(labelled("Withdraw the invitation to Ida Invitee")).not.toBeNull();
	});
});
