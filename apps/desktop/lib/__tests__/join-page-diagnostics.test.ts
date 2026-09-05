// @vitest-environment happy-dom

import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import WebJoinPage from "../../../web/app/join/page";
import DesktopJoinPage from "../../app/join/page";

const mocks = vi.hoisted(() => ({
	joinInviteLink: vi.fn(),
	push: vi.fn(),
	toastError: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	Button: () => null,
	LoadingScreen: () => null,
	addAppToProfile: vi.fn(),
	useBackend: () => ({ teamState: { joinInviteLink: mocks.joinInviteLink } }),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: mocks.push }),
	useSearchParams: () =>
		new URLSearchParams({ appId: "app-id", token: "invite-secret" }),
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ isLoading: false, isAuthenticated: true }),
}));
vi.mock("sonner", () => ({
	toast: { error: mocks.toastError, success: vi.fn() },
}));
vi.mock("../../lib/pending-invite", () => ({
	setPendingInvite: vi.fn(),
	clearPendingInvite: vi.fn(),
}));

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	vi.clearAllMocks();
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
