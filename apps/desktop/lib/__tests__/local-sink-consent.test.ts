// @vitest-environment happy-dom
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	toast: { info: vi.fn(), error: vi.fn() },
}));

vi.mock("@flow-like/flow-like-ui", async () => {
	const { createElement } = await import("react");
	return {
		useBackend: () => ({ auth: undefined }),
		PatSelectorDialog: ({
			open,
			onOpenChange,
			onPatSelected,
		}: {
			open: boolean;
			onOpenChange: (open: boolean) => void;
			onPatSelected: (pat: string) => void;
		}) =>
			open
				? createElement(
						"div",
						null,
						createElement(
							"button",
							{
								type: "button",
								"data-action": "use-token",
								onClick: () => {
									onPatSelected("pat_chosen");
									onOpenChange(false);
								},
							},
							"use",
						),
						createElement(
							"button",
							{
								type: "button",
								"data-action": "cancel-picker",
								onClick: () => onOpenChange(false),
							},
							"cancel",
						),
					)
				: null,
	};
});
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (
			key: string,
			text?: string,
			values?: Record<string, string | undefined>,
		) => (text ?? key).replace(/{{(\w+)}}/g, (_, name) => values?.[name] ?? ""),
	}),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("react-oidc-context", async () => {
	const { createContext } = await import("react");
	return { AuthContext: createContext(undefined) };
});
vi.mock("sonner", () => ({ toast: mocks.toast }));
vi.mock("../../components/local-sink/local-sink-consent-dialog", async () => {
	const { createElement } = await import("react");
	const button = (action: string, onClick: () => void) =>
		createElement(
			"button",
			{ type: "button", "data-action": action, onClick },
			action,
		);
	return {
		LocalSinkConsentDialog: (props: {
			open: boolean;
			onConfirm: (scope: string) => void;
			onDecline: () => void;
			onDismiss: () => void;
		}) =>
			props.open
				? createElement(
						"div",
						null,
						button("allow", () => props.onConfirm("none")),
						button("remember-app", () => props.onConfirm("app")),
						button("dont-register", props.onDecline),
						button("close-consent", props.onDismiss),
					)
				: null,
		LocalTriggerTokenDialog: (props: {
			open: boolean;
			onChooseToken: () => void;
			onDecline: () => void;
			onDismiss: () => void;
		}) =>
			props.open
				? createElement(
						"div",
						null,
						button("add-token", props.onChooseToken),
						button("save-without-trigger", props.onDecline),
						button("close-token", props.onDismiss),
					)
				: null,
	};
});

import {
	LOCAL_SINK_CONSENT_REQUIRED,
	LOCAL_SINK_CONSENT_RESULT,
	type LocalSinkConsentAnswer,
	type LocalSinkConsentRequest,
	type LocalSinkConsentResult,
	TRIGGER_TOKEN_WRONG_ACCOUNT,
	firstConsentStep,
	hasLocalSinkConsent,
	isWrongAccountRefusal,
	requestLocalSinkConsent,
	saveLocalSinkConsent,
} from "../../components/local-sink/local-sink-consent";
import { LocalSinkConsentProvider } from "../../components/local-sink/local-sink-consent-provider";

/** How `verify_trigger_token` in desktop Rust refuses another account's token. */
const WRONG_ACCOUNT_REFUSAL =
	"TRIGGER_TOKEN_WRONG_ACCOUNT: This personal access token belongs to another account; choose one of your own";

const request = {
	appId: "app-1",
	eventId: "event-1",
	eventName: "Nightly report",
	eventType: "cron",
	approve: true,
	needsToken: false,
};

function captureNextRequest(): Promise<LocalSinkConsentRequest> {
	return new Promise((resolve) => {
		window.addEventListener(
			LOCAL_SINK_CONSENT_REQUIRED,
			(event) =>
				resolve((event as CustomEvent<LocalSinkConsentRequest>).detail),
			{ once: true },
		);
	});
}

function answer(
	requestId: string | undefined,
	answer: LocalSinkConsentAnswer,
	pat?: string,
) {
	window.dispatchEvent(
		new CustomEvent(LOCAL_SINK_CONSENT_RESULT, {
			detail: { answer, requestId, pat },
		}),
	);
}

describe("local trigger consent", () => {
	beforeEach(() => localStorage.clear());

	test.each(["decline", "dismiss"] as const)(
		"asks the host and resolves with its answer (%s)",
		async (expected) => {
			const asked = captureNextRequest();
			const pending = requestLocalSinkConsent(request);

			const detail = await asked;
			expect(detail).toMatchObject(request);
			answer(detail.requestId, expected);

			await expect(pending).resolves.toEqual({ answer: expected });
			expect(hasLocalSinkConsent(request)).toBe(false);
		},
	);

	test("ignores answers meant for another request", async () => {
		const asked = captureNextRequest();
		let settled = false;
		const pending = requestLocalSinkConsent(request).then((result) => {
			settled = true;
			return result;
		});
		const detail = await asked;

		answer("someone-else", "decline");
		await Promise.resolve();
		expect(settled).toBe(false);

		answer(detail.requestId, "allow");
		await expect(pending).resolves.toEqual({ answer: "allow" });
	});

	test("a remembered project approval skips the dialog", async () => {
		saveLocalSinkConsent("app", request);
		let asked = false;
		window.addEventListener(
			LOCAL_SINK_CONSENT_REQUIRED,
			() => {
				asked = true;
			},
			{ once: true },
		);

		await expect(requestLocalSinkConsent(request)).resolves.toEqual({
			answer: "allow",
		});
		expect(asked).toBe(false);
		expect(hasLocalSinkConsent({ ...request, eventId: "another" })).toBe(true);
		expect(hasLocalSinkConsent({ ...request, appId: "app-2" })).toBe(false);
	});

	test("a remembered approval with a missing token still asks for the token", async () => {
		saveLocalSinkConsent("event", request);
		const asked = captureNextRequest();
		const pending = requestLocalSinkConsent({ ...request, needsToken: true });

		const detail = await asked;
		expect(detail).toMatchObject({ approve: false, needsToken: true });
		answer(detail.requestId, "allow", "pat_chosen");

		await expect(pending).resolves.toEqual({
			answer: "allow",
			pat: "pat_chosen",
		});
	});

	test("a remembered approval does not cover a trigger moved from another app", async () => {
		saveLocalSinkConsent("event", request);
		const asked = captureNextRequest();
		const pending = requestLocalSinkConsent({
			...request,
			movesFromApp: "Offline copy",
		});

		const detail = await asked;
		expect(detail).toMatchObject({
			approve: true,
			movesFromApp: "Offline copy",
		});
		answer(detail.requestId, "decline");
		await expect(pending).resolves.toEqual({ answer: "decline" });
	});

	test("an event approval covers that event and trigger type only", () => {
		saveLocalSinkConsent("event", request);
		expect(hasLocalSinkConsent({ ...request, appId: "app-2" })).toBe(true);
		expect(hasLocalSinkConsent({ ...request, eventId: "event-2" })).toBe(false);
		expect(hasLocalSinkConsent({ ...request, eventType: "http" })).toBe(false);
	});

	test.each([
		[{ approve: true, needsToken: true }, false, "consent"],
		[{ approve: true, needsToken: false }, false, "consent"],
		[{ approve: true, needsToken: true }, true, "token"],
		[{ approve: true, needsToken: false }, true, null],
		[{ approve: false, needsToken: true }, false, "token"],
		[{ approve: false, needsToken: false }, false, null],
		[
			{ approve: true, needsToken: false, movesFromApp: "Other" },
			true,
			"consent",
		],
	] as const)(
		"firstConsentStep(%o, remembered: %s) is %s",
		(step, remembered, expected) => {
			expect(firstConsentStep(step, remembered)).toBe(expected);
		},
	);

	test("recognizes the desktop's refusal of another account's token by its code", () => {
		expect(isWrongAccountRefusal(WRONG_ACCOUNT_REFUSAL)).toBe(true);
		for (const reason of [
			"This personal access token belongs to another account; choose one of your own",
			"The hub does not accept this personal access token; it may be revoked or expired",
			`Event was saved, but its trigger on this computer was not registered: ${WRONG_ACCOUNT_REFUSAL}`,
			"TRIGGER_TOKEN_WRONG_ACCOUNT",
		]) {
			expect(isWrongAccountRefusal(reason)).toBe(false);
		}
	});

	test("matches the code desktop Rust starts that refusal with", () => {
		const rust = readFileSync(
			join(__dirname, "../../src-tauri/src/event_sink/trigger_token.rs"),
			"utf8",
		);
		expect(rust).toContain(
			`const WRONG_ACCOUNT_CODE: &str = "${TRIGGER_TOKEN_WRONG_ACCOUNT}";`,
		);
		expect(WRONG_ACCOUNT_REFUSAL.startsWith(TRIGGER_TOKEN_WRONG_ACCOUNT)).toBe(
			true,
		);
	});
});

describe("local trigger consent dialogs", () => {
	let root: Root;
	let results: LocalSinkConsentResult[];
	const recordResult = (event: Event) =>
		results.push((event as CustomEvent<LocalSinkConsentResult>).detail);

	beforeEach(async () => {
		vi.clearAllMocks();
		localStorage.clear();
		Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
		results = [];
		window.addEventListener(LOCAL_SINK_CONSENT_RESULT, recordResult);
		root = createRoot(document.body.appendChild(document.createElement("div")));
		await act(async () => {
			root.render(createElement(LocalSinkConsentProvider));
		});
	});

	afterEach(async () => {
		window.removeEventListener(LOCAL_SINK_CONSENT_RESULT, recordResult);
		await act(async () => root.unmount());
		document.body.replaceChildren();
	});

	/** Wrapped, so awaiting the dialog's appearance does not await its answer. */
	async function ask(
		overrides: Partial<Omit<LocalSinkConsentRequest, "requestId">>,
	) {
		let answered: ReturnType<typeof requestLocalSinkConsent> | undefined;
		await act(async () => {
			answered = requestLocalSinkConsent({ ...request, ...overrides });
		});
		if (!answered) throw new Error("No consent request was made");
		return { answered };
	}

	async function click(action: string) {
		const button = document.querySelector<HTMLButtonElement>(
			`[data-action="${action}"]`,
		);
		if (!button) throw new Error(`No "${action}" button is shown`);
		await act(async () => button.click());
	}

	/** The provider keeps every dialog closed briefly between two steps. */
	async function settle() {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 350));
		});
	}

	test("a chosen token completes the request once; the picker closing after it does not dismiss", async () => {
		mocks.invoke.mockResolvedValue(null);
		const { answered } = await ask({ approve: false, needsToken: true });

		await click("add-token");
		await settle();
		await click("use-token");

		await expect(answered).resolves.toEqual({
			answer: "allow",
			pat: "pat_chosen",
		});
		expect(mocks.invoke).toHaveBeenCalledWith("verify_trigger_token", {
			appId: "app-1",
			pat: "pat_chosen",
		});
		expect(results.map((result) => result.answer)).toEqual(["allow"]);
	});

	test("an approval leads to the token picker", async () => {
		mocks.invoke.mockResolvedValue(null);
		const { answered } = await ask({ approve: true, needsToken: true });

		await click("remember-app");
		await settle();
		await click("use-token");

		await expect(answered).resolves.toEqual({
			answer: "allow",
			pat: "pat_chosen",
		});
		expect(hasLocalSinkConsent(request)).toBe(true);
	});

	test("a token of another account keeps the picker open with a warning", async () => {
		mocks.invoke.mockRejectedValue({ error: WRONG_ACCOUNT_REFUSAL });
		const { answered } = await ask({ approve: false, needsToken: true });

		await click("add-token");
		await settle();
		await click("use-token");

		expect(mocks.toast.error).toHaveBeenCalledWith(
			"This token belongs to another account. Choose one of your own tokens for this hub.",
		);
		expect(results).toEqual([]);

		await click("cancel-picker");
		await expect(answered).resolves.toEqual({ answer: "dismiss" });
	});

	test("a token the hub refuses names the event and the reason, and keeps the picker open", async () => {
		const reason =
			"The hub does not accept this personal access token; it may be revoked or expired";
		mocks.invoke.mockRejectedValue({ error: reason });
		const { answered } = await ask({ approve: false, needsToken: true });

		await click("add-token");
		await settle();
		await click("use-token");

		expect(mocks.toast.error).toHaveBeenCalledWith(
			"Could not add a token to Nightly report.",
			{ description: reason },
		);
		expect(results).toEqual([]);

		await click("cancel-picker");
		await expect(answered).resolves.toEqual({ answer: "dismiss" });
	});

	test("closing the token step abandons the save, and Save without trigger declines it", async () => {
		const dismissed = await ask({ approve: false, needsToken: true });
		await click("close-token");
		await expect(dismissed.answered).resolves.toEqual({ answer: "dismiss" });

		await settle();
		const declined = await ask({ approve: false, needsToken: true });
		await click("save-without-trigger");
		await expect(declined.answered).resolves.toEqual({ answer: "decline" });
		expect(mocks.toast.info).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("Add token opens the token list itself and answers with the chosen token", async () => {
		mocks.invoke.mockResolvedValue(null);
		const { answered } = await ask({
			approve: false,
			needsToken: true,
			repair: true,
		});

		await click("use-token");

		await expect(answered).resolves.toEqual({
			answer: "allow",
			pat: "pat_chosen",
		});
	});

	test("Add token offers no Save without trigger, and closing its list abandons it without a toast", async () => {
		const { answered } = await ask({
			approve: false,
			needsToken: true,
			repair: true,
		});
		expect(
			document.querySelector('[data-action="save-without-trigger"]'),
		).toBeNull();

		await click("cancel-picker");

		await expect(answered).resolves.toEqual({ answer: "dismiss" });
		expect(mocks.toast.info).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});
