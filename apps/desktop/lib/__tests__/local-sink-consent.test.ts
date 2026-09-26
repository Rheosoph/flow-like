// @vitest-environment happy-dom
import { beforeEach, describe, expect, test } from "vitest";
import {
	LOCAL_SINK_CONSENT_REQUIRED,
	LOCAL_SINK_CONSENT_RESULT,
	type LocalSinkConsentAnswer,
	type LocalSinkConsentRequest,
	hasLocalSinkConsent,
	requestLocalSinkConsent,
	saveLocalSinkConsent,
} from "../../components/local-sink/local-sink-consent";

const request = {
	appId: "app-1",
	eventId: "event-1",
	eventName: "Nightly report",
	eventType: "cron",
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

function answer(requestId: string | undefined, answer: LocalSinkConsentAnswer) {
	window.dispatchEvent(
		new CustomEvent(LOCAL_SINK_CONSENT_RESULT, {
			detail: { answer, requestId },
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

			await expect(pending).resolves.toBe(expected);
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
		await expect(pending).resolves.toBe("allow");
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

		await expect(requestLocalSinkConsent(request)).resolves.toBe("allow");
		expect(asked).toBe(false);
		expect(hasLocalSinkConsent({ ...request, eventId: "another" })).toBe(true);
		expect(hasLocalSinkConsent({ ...request, appId: "app-2" })).toBe(false);
	});

	test("an event approval covers that event and trigger type only", () => {
		saveLocalSinkConsent("event", request);
		expect(hasLocalSinkConsent({ ...request, appId: "app-2" })).toBe(true);
		expect(hasLocalSinkConsent({ ...request, eventId: "event-2" })).toBe(false);
		expect(hasLocalSinkConsent({ ...request, eventType: "http" })).toBe(false);
	});
});
