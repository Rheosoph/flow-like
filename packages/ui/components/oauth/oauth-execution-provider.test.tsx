// @vitest-environment happy-dom
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { type Mock, afterEach, beforeEach, expect, it, vi } from "vitest";
import type { IOAuthConsentStore } from "../../db/oauth-db";
import type { OAuthService } from "../../lib/oauth/service";
import type {
	IOAuthProvider,
	IOAuthRuntime,
	IOAuthTokenStoreWithPending,
	IStoredOAuthToken,
} from "../../lib/oauth/types";
import {
	type OAuthExecutionContextValue,
	OAuthExecutionProvider,
	useOAuthExecutionContext,
} from "./oauth-execution-provider";

vi.mock("./device-flow-dialog", () => ({ DeviceFlowDialog: () => null }));
vi.mock("../ui/dialog", () => {
	const Section = ({ children }: { children?: ReactNode }) => (
		<div>{children}</div>
	);
	return {
		Dialog: ({ open, children }: { open: boolean; children: ReactNode }) =>
			open ? <dialog open>{children}</dialog> : null,
		DialogContent: Section,
		DialogDescription: Section,
		DialogFooter: Section,
		DialogHeader: Section,
		DialogTitle: Section,
	};
});

const apiScope = "https://www.googleapis.com/auth/calendar.events";
const provider: IOAuthProvider = {
	id: "google",
	name: "Google",
	auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
	token_url: "https://oauth2.googleapis.com/token",
	client_id: "google-client",
	scopes: ["openid", "email", "profile"],
	merged_scopes: ["openid", "email", "profile", apiScope],
	pkce_required: true,
	requires_secret_proxy: true,
	use_device_flow: false,
};
const grantedIdentityScopes = [
	"openid",
	"https://www.googleapis.com/auth/userinfo.email",
	"https://www.googleapis.com/auth/userinfo.profile",
];
const execution = {
	appId: "app",
	eventId: "chat-event",
	sessionId: "conversation",
	requestId: "authorization-request",
	boardId: "board",
	nodeId: "event",
	payload: { message: "Run after authorization" },
};

let root: Root;
let container: HTMLDivElement;
let context: OAuthExecutionContextValue;
let startAuthorization: ReturnType<typeof vi.fn>;
let retry: Mock<(event: Event) => void>;

function CaptureContext() {
	context = useOAuthExecutionContext();
	return null;
}

function button(label: string) {
	const match = [...container.querySelectorAll("button")].find(
		(element) => element.textContent?.trim() === label,
	);
	if (!match) throw new Error(`Missing button: ${label}`);
	return match;
}

function callbackToken(scopes: string[]): IStoredOAuthToken {
	return {
		providerId: provider.id,
		access_token: "google-access-token",
		token_type: "Bearer",
		storedAt: Date.now(),
		scopes,
	};
}

beforeEach(async () => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	startAuthorization = vi.fn().mockResolvedValue(undefined);
	retry = vi.fn<(event: Event) => void>();
	window.addEventListener("flow:oauth-retry", retry);
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
	await act(async () => {
		root.render(
			<OAuthExecutionProvider
				oauthService={{ startAuthorization } as unknown as OAuthService}
				runtime={{} as IOAuthRuntime}
				tokenStore={
					{
						getToken: vi.fn().mockResolvedValue(undefined),
						isExpired: () => false,
					} as unknown as IOAuthTokenStoreWithPending
				}
				consentStore={
					{
						hasConsentWithScopes: vi.fn().mockResolvedValue(false),
						setConsent: vi.fn().mockResolvedValue(undefined),
					} as unknown as IOAuthConsentStore
				}
				providerCacheRef={{ current: new Map() }}
			>
				<CaptureContext />
			</OAuthExecutionProvider>,
		);
	});
	await act(async () => {
		window.dispatchEvent(
			new CustomEvent("flow:oauth-required", {
				detail: { ...execution, missingProviders: [provider] },
			}),
		);
	});
	await act(async () => {
		container.querySelector<HTMLButtonElement>("#acknowledge")?.click();
	});
	await act(async () => button("Authorize").click());
	expect(startAuthorization).toHaveBeenCalledWith(provider);
	expect(button("Waiting for 1 service").disabled).toBe(true);
});

afterEach(async () => {
	window.removeEventListener("flow:oauth-retry", retry);
	await act(async () => root.unmount());
	container.remove();
});

it("allows the waiting workflow to continue after Google returns canonical identity scopes", async () => {
	await act(async () => {
		context.handleOAuthCallback(
			provider.id,
			callbackToken([...grantedIdentityScopes, apiScope]),
		);
	});
	expect(container.textContent).toContain("Authorized");
	expect(button("Continue").disabled).toBe(false);
	await act(async () => button("Continue").click());
	expect(container.querySelector("dialog")).toBeNull();
	expect(retry).toHaveBeenCalledOnce();
	expect((retry.mock.calls[0][0] as CustomEvent).detail).toEqual({
		...execution,
		skipConsentCheck: true,
	});
});

it("keeps the workflow waiting when the callback lacks a required API scope", async () => {
	await act(async () => {
		context.handleOAuthCallback(
			provider.id,
			callbackToken(grantedIdentityScopes),
		);
	});
	expect(button("Waiting for 1 service").disabled).toBe(true);
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Google did not grant all required permissions. Authorize again and allow: calendar.events.",
	);
	expect(container.textContent).not.toContain("Authorized");
	expect(retry).not.toHaveBeenCalled();
});

it("reports the originating request when authorization is cancelled", async () => {
	const cancelled = vi.fn<(event: Event) => void>();
	window.addEventListener("flow:oauth-cancelled", cancelled);
	try {
		await act(async () => button("Cancel").click());
		expect(container.querySelector("dialog")).toBeNull();
		expect(cancelled).toHaveBeenCalledOnce();
		expect((cancelled.mock.calls[0][0] as CustomEvent).detail).toEqual(
			execution,
		);
		expect(retry).not.toHaveBeenCalled();
	} finally {
		window.removeEventListener("flow:oauth-cancelled", cancelled);
	}
});
