import { afterAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { createRoot } from "react-dom/client";
import type { QuotaOperationDetail } from "../../../lib/quota";

let detail = {
	operationId: "embedding-1",
	fundingClass: "hosted",
	status: "completed",
	meteringBasis: "input_bytes",
	inputBytes: 120,
	providerReportedWords: 20,
	tokenCountEstimated: true,
	embeddingTokens: 120,
	providerCostMicroUsd: 3,
	estimated: true,
} as QuotaOperationDetail;
let enabled = false;
let scope: unknown[] = [];
// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const actual = {
	invoke: { ...(await import("../../../hooks/use-invoke")) },
	backendState: { ...(await import("../../../state/backend-state")) },
	oidc: { ...(await import("react-oidc-context")) },
	locales: { ...(await import("@flow-like/locales")) },
};
const globalDescriptors = [
	"window",
	"document",
	"HTMLElement",
	"Element",
	"Node",
	"NodeFilter",
	"Event",
	"CustomEvent",
	"MutationObserver",
	"HTMLInputElement",
	"navigator",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
mock.module("../../../hooks/use-invoke", () => ({
	...actual.invoke,
	useInvoke: (
		_fn: unknown,
		_context: unknown,
		_args: unknown[],
		active: boolean,
		dependencies: unknown[],
	) => {
		enabled = active;
		scope = dependencies;
		return {
			data: active ? detail : undefined,
			isLoading: false,
			isError: false,
		};
	},
}));
mock.module("../../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => ({ userState: { getQuotaOperationDetail: () => {} } }),
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => ({ user: { profile: { sub: "payer" } } }),
}));
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
afterAll(() => {
	mock.restore();
	mock.module("../../../hooks/use-invoke", () => actual.invoke);
	mock.module("../../../state/backend-state", () => actual.backendState);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module("@flow-like/locales", () => actual.locales);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("operation details use an accessible lazy-loaded panel and retain estimates and own-model disclosures", async () => {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError, Error });
	Object.assign(globalThis, {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Element: window.Element,
		Node: window.Node,
		NodeFilter: window.NodeFilter,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		MutationObserver: window.MutationObserver,
		HTMLInputElement: window.HTMLInputElement,
		navigator: window.navigator,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { UsageOperationDetails } = await import("./usage-operation-details");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	await act(async () =>
		root.render(<UsageOperationDetails id="embedding-1" />),
	);
	expect(enabled).toBe(false);
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	const trigger = container.querySelector("button")!;
	await act(async () => {
		trigger.focus();
		trigger.click();
	});
	expect(enabled).toBe(true);
	expect(scope).toEqual(["payer"]);
	const panel = window.document.querySelector('[role="dialog"]')!;
	expect(panel).not.toBeNull();
	expect(panel.getAttribute("aria-labelledby")).toBeTruthy();
	expect(panel.getAttribute("aria-describedby")).toBeTruthy();
	expect(panel.textContent).toContain("Operation details");
	expect(panel.textContent).toContain("Input bytes");
	expect(panel.textContent).toContain("Reported words (estimate)");
	expect(panel.textContent).toContain("Token upper bound (estimate)");
	expect(panel.textContent).toContain("Internal inference estimate");
	expect(panel.textContent).not.toContain("Provider inference");
	expect(panel.textContent).not.toContain("Embedding tokens");
	expect(panel.textContent).toContain("Awaiting final usage");
	expect(container.textContent).not.toContain("Input bytes");
	const close = Array.from(panel.querySelectorAll("button")).find(
		(button) => button.textContent === "Close",
	)!;
	await act(async () => close.click());
	expect(enabled).toBe(false);
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	detail = { ...detail, fundingClass: "byok", status: "completed" };
	await act(async () => trigger.click());
	const ownPanel = window.document.querySelector('[role="dialog"]')!;
	expect(ownPanel.textContent).toContain("Your provider");
	expect(ownPanel.textContent).toContain("No hosted AI allowance used");
	expect(ownPanel.textContent).toContain(
		"Cloud orchestration still uses cloud runtime",
	);
	expect(ownPanel.textContent).not.toContain("Token usage");
	await act(async () =>
		window.document.dispatchEvent(
			new window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
		),
	);
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	expect(enabled).toBe(false);
	await act(async () => root.unmount());
	window.happyDOM.abort();
});
