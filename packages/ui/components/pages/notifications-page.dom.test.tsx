// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiResponseError } from "../../lib/api-error";
import { PackagePermissionBits } from "../../lib/permission/wasm-package-permission";
import type { PackageInvitation } from "../../lib/schema/wasm";
import type { IInvite } from "../../state/backend-state/types";
import { NotificationsPageScreen } from "./notifications-page";

const context = vi.hoisted(() => ({
	backend: {} as Record<string, Record<string, ReturnType<typeof vi.fn>>>,
	auth: {
		isAuthenticated: true,
		isLoading: false,
		user: { profile: { sub: "recipient" } },
	},
	errorToast: vi.fn(),
}));

vi.mock("../../state/backend-state", () => ({
	useBackend: () => context.backend,
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => context.auth }));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock("sonner", () => ({
	toast: { error: context.errorToast, success: vi.fn() },
}));
vi.mock("../notifications/notification-icon", () => ({
	NotificationIcon: () => null,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: string, values?: Record<string, unknown>) =>
			(fallback ?? key).replace(/\{\{(\w+)\}\}/g, (_, name) =>
				String(values?.[name] ?? ""),
			),
	}),
}));
vi.mock("framer-motion", async () => {
	const { createElement } = await import("react");
	const element =
		(tag: string) =>
		({
			children,
			className,
		}: { children?: React.ReactNode; className?: string }) =>
			createElement(tag, { className }, children);
	return {
		AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
		motion: { div: element("div"), section: element("section") },
	};
});
vi.mock("../ui", async () => ({
	...(await import("../ui/badge")),
	...(await import("../ui/button")),
	...(await import("../ui/skeleton")),
	...(await import("../ui/tabs")),
}));

const packageInvite: PackageInvitation = {
	id: "package-invite",
	packageId: "private-math",
	inviteeId: "recipient",
	invitedById: "maintainer",
	permission: PackagePermissionBits.Maintainer,
	status: "pending",
	createdAt: "2026-10-01T10:00:00Z",
};
const appInvite: IInvite = {
	id: "app-invite",
	app_id: "workspace",
	user_id: "recipient",
	by_member_id: "maintainer",
	name: "Team workspace",
	created_at: "2026-10-01T10:00:00Z",
	updated_at: "2026-10-01T10:00:00Z",
};

let client: QueryClient;
let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	context.auth.isAuthenticated = true;
	context.errorToast.mockReset();
	context.backend = {
		teamState: {
			getInvites: vi.fn(async () => [] as IInvite[]),
			acceptInvite: vi.fn(async () => {}),
			rejectInvite: vi.fn(async () => {}),
		},
		registryState: {
			listMyInvitations: vi.fn(async () => [packageInvite]),
			acceptInvitation: vi.fn(async () => {
				context.backend.registryState.listMyInvitations.mockResolvedValue([]);
			}),
			rejectInvitation: vi.fn(async () => {
				context.backend.registryState.listMyInvitations.mockResolvedValue([]);
			}),
		},
		userState: {
			listNotifications: vi.fn(async () => []),
			getNotifications: vi.fn(async () => ({
				invites_count: 1,
				unread_count: 0,
				notifications_count: 0,
			})),
			lookupUser: vi.fn(async () => ({ name: "Morgan", id: "maintainer" })),
		},
	};
	// useInvoke keys queries by the backend method's name.
	for (const state of Object.values(context.backend)) {
		for (const [name, method] of Object.entries(state)) {
			Object.defineProperty(method, "name", { value: name });
		}
	}
	client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	host.remove();
	vi.restoreAllMocks();
});

async function render() {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<NotificationsPageScreen />
			</QueryClientProvider>,
		),
	);
	await flush();
}

async function flush() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

function button(label: string) {
	const candidate = [...host.querySelectorAll("button")].find(
		(node) => node.textContent?.trim() === label,
	);
	if (!candidate)
		throw new Error(`Missing button ${label}: ${host.textContent}`);
	return candidate;
}

async function click(label: string) {
	await act(async () => button(label).click());
	await flush();
}

async function showInvitations() {
	const tab = host.querySelector<HTMLElement>(
		'[role="tab"][id$="-trigger-invitations"]',
	);
	if (!tab) throw new Error("Missing invitations tab");
	await act(async () =>
		tab.dispatchEvent(
			new MouseEvent("mousedown", { bubbles: true, button: 0 }),
		),
	);
	await flush();
}

describe("package invitations in the inbox", () => {
	it("shows package and app invitations in both inbox tabs and includes both in the count", async () => {
		context.backend.teamState.getInvites.mockResolvedValue([appInvite]);
		await render();
		expect(host.textContent).toContain("private-math");
		expect(host.textContent).toContain("Package invitation");
		expect(host.textContent).toContain("Maintainer access");
		expect(host.textContent).toContain("Team workspace");
		expect(host.textContent).toContain(
			"2 invitations, 0 workflow notifications",
		);
		expect(context.backend.userState.lookupUser).toHaveBeenCalledWith(
			"maintainer",
		);
		await showInvitations();
		expect(host.textContent).toContain("private-math");
		expect(host.textContent).toContain("Team workspace");
		expect(host.textContent).not.toContain("No pending invitations");
	});

	it("accepts through the registry, prevents duplicate actions, and refreshes package access and the badge", async () => {
		let resolveAccept!: () => void;
		context.backend.registryState.acceptInvitation.mockImplementation(
			async () => {
				await new Promise<void>((resolve) => {
					resolveAccept = resolve;
				});
				context.backend.registryState.listMyInvitations.mockResolvedValue([]);
			},
		);
		const refreshedKeys = [
			["registry-library", "recipient"],
			["mine-registry-maintained", "recipient"],
			["registry-package", "private-math", "recipient"],
			["getNotifications"],
		];
		for (const key of refreshedKeys) client.setQueryData(key, {});
		await render();
		await click("Accept");
		expect(button("Accept").disabled).toBe(true);
		expect(button("Decline").disabled).toBe(true);
		expect(
			context.backend.registryState.acceptInvitation,
		).toHaveBeenCalledExactlyOnceWith("package-invite");
		expect(context.backend.teamState.acceptInvite).not.toHaveBeenCalled();
		await act(async () => resolveAccept());
		await flush();
		expect(host.textContent).not.toContain("private-math");
		expect(host.textContent).toContain("No activity just yet");
		for (const key of refreshedKeys)
			expect(client.getQueryState(key)?.isInvalidated).toBe(true);
	});

	it("declines a package invitation without granting or refreshing package access", async () => {
		client.setQueryData(["registry-library", "recipient"], {});
		await render();
		await showInvitations();
		await click("Decline");
		expect(
			context.backend.registryState.rejectInvitation,
		).toHaveBeenCalledExactlyOnceWith("package-invite");
		expect(context.backend.teamState.rejectInvite).not.toHaveBeenCalled();
		expect(
			context.backend.registryState.acceptInvitation,
		).not.toHaveBeenCalled();
		expect(host.textContent).toContain("No pending invitations");
		expect(
			client.getQueryState(["registry-library", "recipient"])?.isInvalidated,
		).toBe(false);
	});

	it("declines team invitations through the team API while keeping package invitations", async () => {
		context.backend.teamState.getInvites.mockResolvedValue([appInvite]);
		context.backend.teamState.rejectInvite.mockImplementation(async () => {
			context.backend.teamState.getInvites.mockResolvedValue([]);
		});
		await render();
		await click("Decline");
		expect(
			context.backend.teamState.rejectInvite,
		).toHaveBeenCalledExactlyOnceWith("app-invite");
		expect(
			context.backend.registryState.rejectInvitation,
		).not.toHaveBeenCalled();
		expect(
			context.backend.registryState.acceptInvitation,
		).not.toHaveBeenCalled();
		expect(host.textContent).not.toContain("Team workspace");
		expect(host.textContent).toContain("private-math");
	});

	it("loads newly received package invitations and refreshes the sidebar badge on refresh", async () => {
		client.setQueryData(["getNotifications"], { invites_count: 0 });
		context.backend.registryState.listMyInvitations.mockResolvedValueOnce([]);
		await render();
		expect(host.textContent).toContain("No activity just yet");
		await click("Refresh");
		expect(host.textContent).toContain("private-math");
		expect(host.textContent).toContain(
			"1 invitation, 0 workflow notifications",
		);
		expect(client.getQueryState(["getNotifications"])?.isInvalidated).toBe(
			true,
		);
	});

	it("keeps package loading visible until the invitation request finishes", async () => {
		let resolveList!: (invitations: PackageInvitation[]) => void;
		context.backend.registryState.listMyInvitations.mockImplementation(
			() =>
				new Promise((resolve) => {
					resolveList = resolve;
				}),
		);
		await render();
		expect(host.textContent).toContain("Loading activity and invitations");
		expect(host.textContent).not.toContain("No activity just yet");
		await showInvitations();
		expect(host.textContent).not.toContain("No pending invitations");
		await act(async () => resolveList([packageInvite]));
		await flush();
		expect(host.textContent).toContain("private-math");
	});

	it("shows a retry when package invitations fail even if team invitations loaded", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		context.backend.teamState.getInvites.mockResolvedValue([appInvite]);
		context.backend.registryState.listMyInvitations.mockRejectedValueOnce(
			new Error("offline"),
		);
		await render();
		expect(host.textContent).toContain("Team workspace");
		expect(host.textContent).toContain("Couldn't load your notifications");
		await showInvitations();
		expect(host.textContent).toContain("Couldn't load your notifications");
		await click("Retry");
		expect(host.textContent).toContain("private-math");
		expect(host.textContent).not.toContain("Couldn't load your notifications");
	});

	it("reports acceptance errors, rechecks the invitation, and re-enables its actions", async () => {
		context.backend.registryState.acceptInvitation.mockRejectedValue(
			new ApiResponseError({ status: 410, message: "Invitation expired" }),
		);
		await render();
		await click("Accept");
		expect(context.errorToast).toHaveBeenCalledWith("Invitation expired");
		expect(
			context.backend.registryState.listMyInvitations,
		).toHaveBeenCalledTimes(2);
		expect(button("Accept").disabled).toBe(false);
		expect(button("Decline").disabled).toBe(false);
	});

	it("does not fetch package invitations while signed out, including on refresh", async () => {
		context.auth.isAuthenticated = false;
		await render();
		await click("Refresh");
		expect(
			context.backend.registryState.listMyInvitations,
		).not.toHaveBeenCalled();
	});
});
