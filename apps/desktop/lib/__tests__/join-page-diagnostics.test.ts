// @vitest-environment happy-dom

import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { type ComponentProps, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import WebJoinPage from "../../../web/app/join/page";
import DesktopJoinPage from "../../app/join/page";

const mocks = vi.hoisted(() => ({
	joinInviteLink: vi.fn(),
	joinRoomLink: vi.fn(),
	searchParams: "appId=app-id&token=invite-secret",
	push: vi.fn(),
	toastError: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	Button: ({
		variant: _variant,
		...props
	}: ComponentProps<"button"> & { variant?: string }) =>
		createElement("button", { type: "button", ...props }),
	addAppToProfile: vi.fn(),
	useBackend: () => ({ teamState: { joinInviteLink: mocks.joinInviteLink } }),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: mocks.push }),
	useSearchParams: () => new URLSearchParams(mocks.searchParams),
}));
vi.mock("@flow-like/flow-like-ui/hooks/use-join-room", () => ({
	useJoinRoomLink: () => mocks.joinRoomLink,
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ isLoading: false, isAuthenticated: true }),
}));
vi.mock("sonner", () => ({
	toast: { error: mocks.toastError, success: vi.fn() },
}));
vi.mock("../../lib/pending-invite", () => ({
	setPendingInvite: vi.fn(),
	setPendingRoomInvite: vi.fn(),
	clearPendingInvite: vi.fn(),
}));

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	vi.clearAllMocks();
	mocks.searchParams = "appId=app-id&token=invite-secret";
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe.each([
	["desktop", DesktopJoinPage],
	["web", WebJoinPage],
] as const)("%s invite diagnostics", (_name, JoinPage) => {
	test("keeps the invitation card visible while the join is pending", async () => {
		mocks.joinInviteLink.mockImplementation(() => new Promise(() => {}));

		await act(async () => root.render(createElement(JoinPage)));

		expect(container.querySelector("h2")?.textContent).toBe(
			"You've been invited",
		);
		expect(container.querySelector("output")?.textContent).toBe(
			"Accepting invitation…",
		);
		expect(container.textContent).not.toContain("Loading your workspace");
		expect(container.querySelector(".fixed")).toBeNull();
		expect(mocks.joinInviteLink).toHaveBeenCalledOnce();
		expect(mocks.push).not.toHaveBeenCalled();
	});

	test.each(["", "appId=app-id", "token=invite-secret"])(
		"shows an actionable error for incomplete invite params %s",
		async (params) => {
			mocks.searchParams = params;

			await act(async () => root.render(createElement(JoinPage)));

			expect(container.querySelector("[role=alert]")?.textContent).toContain(
				"This invite link is incomplete.",
			);
			expect(container.querySelector("output")).toBeNull();
			expect(mocks.joinInviteLink).not.toHaveBeenCalled();
			expect(mocks.joinRoomLink).not.toHaveBeenCalled();
			await act(async () => container.querySelector("button")?.click());
			expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/");
		},
	);

	test.each([
		[403, "forbidden"],
		[404, "invalid"],
	] as const)(
		"logs only the failure classification for HTTP %s",
		async (status, kind) => {
			const error = new ApiResponseError({
				status,
				message: "Rejected invite-secret",
				path: "apps/app-id/team/link/join/invite-secret",
			});
			mocks.joinInviteLink.mockRejectedValue(error);
			const logged = vi.spyOn(console, "error").mockImplementation(() => {});

			await act(async () => root.render(createElement(JoinPage)));

			expect(mocks.joinInviteLink).toHaveBeenCalledExactlyOnceWith(
				"app-id",
				"invite-secret",
			);
			expect(logged).toHaveBeenCalledExactlyOnceWith("Failed to join:", kind);
			expect(JSON.stringify(logged.mock.calls)).not.toContain("invite-secret");
			expect(error.path).toContain("invite-secret");
			expect(mocks.toastError).toHaveBeenCalledOnce();
			expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/");
		},
	);
});
