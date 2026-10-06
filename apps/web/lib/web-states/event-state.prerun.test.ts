import type { PageTrigger } from "@flow-like/flow-like-ui/lib/schema/flow/page-trigger";
import {
	IValueType,
	type IVariable,
	IVariableType,
} from "@flow-like/flow-like-ui/lib/schema/flow/variable";
import type { IPrerunEventResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiPost: vi.fn(),
	apiGet: vi.fn(),
	checkOAuthTokensFromPrerun: vi.fn(),
	getConsentedProviderIds: vi.fn(),
	notifyPageContractRejected: vi.fn(),
	apiBase: "https://api.test",
}));

vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await import("@flow-like/flow-like-ui/lib/schema/flow/page-trigger")),
	checkOAuthTokensFromPrerun: mocks.checkOAuthTokensFromPrerun,
	classifyPageContractError: () => "missing_contract",
	finishAllProgressToasts: vi.fn(),
	notifyPageContractRejected: mocks.notifyPageContractRejected,
}));
vi.mock("@flow-like/flow-like-ui/components/payments/payment-events", () => ({
	dispatchPaymentRequest: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui/lib/device-bridge", () => ({
	withDeviceCommandBridge: (
		_context: unknown,
		callback: unknown,
		run: (callback: unknown) => Promise<unknown>,
	) => run(callback),
}));
vi.mock("./api-utils", () => ({
	apiPost: mocks.apiPost,
	apiGet: mocks.apiGet,
	getApiBaseUrl: () => mocks.apiBase,
}));
vi.mock("../oauth-db", () => ({
	oauthConsentStore: { getConsentedProviderIds: mocks.getConsentedProviderIds },
	oauthTokenStore: {},
}));
vi.mock("../oauth-service", () => ({
	getOAuthApiBaseUrl: () => mocks.apiBase,
	getOAuthService: () => ({ refreshToken: vi.fn() }),
}));

import type { WebBackendRef } from "./api-utils";
import { WebEventState } from "./event-state";

const LOAD: PageTrigger = {
	kind: "special",
	specialEvent: "load",
	manifestRevision: "rendered-revision",
};
const noProviders = { requiredProviders: [], missingProviders: [], tokens: {} };
const runtimeVariable: IVariable = {
	id: "value",
	name: "Value",
	data_type: IVariableType.String,
	value_type: IValueType.Normal,
	editable: true,
	exposed: false,
	secret: false,
	default_value: [34, 120, 34],
};
let now: number;

function backendRef() {
	return {
		profile: { id: "profile-1", hub: "" },
		auth: { user: { access_token: "token-1", profile: { sub: "user-1" } } },
	};
}

function prerun(revision = "current-revision"): IPrerunEventResponse {
	return {
		board_id: "board-1",
		manifest_revision: revision,
		oauth_requirements: [],
		runtime_variables: [],
		can_execute_locally: false,
	} as unknown as IPrerunEventResponse;
}

function execute(
	state: WebEventState,
	preparedPrerun?: IPrerunEventResponse,
	{
		appId = "app-1",
		eventId = "event-1",
		trigger = LOAD,
	}: {
		appId?: string;
		eventId?: string;
		trigger?: PageTrigger;
	} = {},
) {
	return state.executeEvent(
		appId,
		eventId,
		{
			id: "page_load",
			payload: {},
			runtime_variables: { value: runtimeVariable },
		},
		false,
		undefined,
		undefined,
		false,
		trigger,
		undefined,
		preparedPrerun,
	);
}

function invocationBody() {
	return JSON.parse(vi.mocked(fetch).mock.calls.at(-1)?.[1]?.body as string);
}

beforeEach(() => {
	vi.clearAllMocks();
	now = 1_000;
	vi.spyOn(performance, "now").mockImplementation(() => now);
	vi.spyOn(console, "log").mockImplementation(() => {});
	mocks.apiBase = "https://api.test";
	mocks.apiPost.mockImplementation(async () => prerun());
	mocks.checkOAuthTokensFromPrerun.mockResolvedValue(noProviders);
	mocks.getConsentedProviderIds.mockResolvedValue(new Set());
	vi.stubGlobal(
		"fetch",
		vi.fn(async (url: string) =>
			url.endsWith("/api/v1") ? Response.json({}) : new Response(null),
		),
	);
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe("web Page prerun handoff", () => {
	test("reuses its dispatch's response once while preserving revision, OAuth and invocation credentials", async () => {
		const backend = backendRef();
		const state = new WebEventState(backend as unknown as WebBackendRef);
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		const tokens = { provider: { access_token: "oauth-token" } };
		mocks.checkOAuthTokensFromPrerun.mockResolvedValue({
			...noProviders,
			tokens,
			requiredProviders: [{ id: "provider", name: "Provider" }],
		});
		mocks.getConsentedProviderIds.mockResolvedValue(new Set(["provider"]));

		await execute(state, prepared);

		expect(mocks.apiPost).toHaveBeenCalledTimes(1);
		expect(mocks.apiGet).not.toHaveBeenCalled();
		expect(mocks.checkOAuthTokensFromPrerun.mock.calls[0]?.[0]).toBe(
			prepared.oauth_requirements,
		);
		expect(invocationBody()).toMatchObject({
			profile_id: "profile-1",
			token: "token-1",
			oauth_tokens: tokens,
			runtime_variables: { value: runtimeVariable },
			page_trigger: { manifest_revision: "current-revision" },
		});
		expect(
			new Headers(vi.mocked(fetch).mock.calls[0]?.[1]?.headers).get(
				"Authorization",
			),
		).toBe("Bearer token-1");

		await execute(state, prepared);
		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
	});

	test("concurrent dispatches keep their own responses even when the second prerun finishes first", async () => {
		const state = new WebEventState({});
		let resolveFirst!: (result: IPrerunEventResponse) => void;
		mocks.apiPost.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveFirst = resolve;
				}),
		);
		mocks.apiPost.mockResolvedValueOnce(prerun("second-revision"));
		const first = state.prerunEvent("app-1", "event-1", undefined, LOAD);
		const second = await state.prerunEvent("app-1", "event-1", undefined, LOAD);
		resolveFirst(prerun("first-revision"));
		const firstResult = await first;

		await Promise.all([execute(state, second), execute(state, firstResult)]);

		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
		expect(
			vi
				.mocked(fetch)
				.mock.calls.map(
					([, init]) =>
						JSON.parse(init?.body as string).page_trigger.manifest_revision,
				),
		).toEqual(["second-revision", "first-revision"]);
	});

	test("a consent or runtime-variable delay expires the response and refreshes the revision", async () => {
		const state = new WebEventState({});
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		now += 15_001;
		mocks.apiPost.mockResolvedValueOnce(prerun("after-prompt"));

		await execute(state, prepared);

		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
		expect(invocationBody().page_trigger.manifest_revision).toBe(
			"after-prompt",
		);
	});

	test("an account switch while prerun is in flight cannot reassign its response", async () => {
		const backend = backendRef();
		const state = new WebEventState(backend as unknown as WebBackendRef);
		let resolve!: (response: IPrerunEventResponse) => void;
		mocks.apiPost.mockImplementationOnce(
			() =>
				new Promise((done) => {
					resolve = done;
				}),
		);
		const pending = state.prerunEvent("app-1", "event-1", undefined, LOAD);
		backend.auth.user.profile.sub = "user-2";
		backend.auth.user.access_token = "token-2";
		resolve(prerun("user-1-revision"));

		await execute(state, await pending);

		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
		expect(invocationBody().page_trigger.manifest_revision).toBe(
			"current-revision",
		);
	});

	test("freshness includes the time spent waiting for the prerun response", async () => {
		const state = new WebEventState({});
		mocks.apiPost.mockImplementationOnce(async () => {
			now += 15_001;
			return prerun("slow-response");
		});
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		await execute(state, prepared);
		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
	});

	test("dynamic Page grants are scoped by capability and keep their original revision", async () => {
		const state = new WebEventState({});
		const trigger: PageTrigger = {
			kind: "action",
			actionId: "da1_action",
			capabilityJwt: "capability-a",
			manifestRevision: "granted-revision",
		};
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			trigger,
		);
		await execute(state, prepared, {
			trigger: { ...trigger, capabilityJwt: "capability-b" },
		});
		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
		expect(invocationBody().page_trigger).toMatchObject({
			capability_jwt: "capability-b",
			manifest_revision: "granted-revision",
		});
	});

	test.each([
		["app", { appId: "app-2" }],
		["event", { eventId: "event-2" }],
		[
			"trigger",
			{ trigger: { ...LOAD, specialEvent: "interval" } as PageTrigger },
		],
		["revision", { trigger: { ...LOAD, manifestRevision: "other-page" } }],
	])("does not reuse a response for another %s", async (_name, options) => {
		const state = new WebEventState({});
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		await execute(state, prepared, options);
		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
	});

	test.each(["profile", "hub", "user", "token", "origin"])(
		"does not reuse a response after %s changes",
		async (changed) => {
			const backend = backendRef();
			const state = new WebEventState(backend as unknown as WebBackendRef);
			const prepared = await state.prerunEvent(
				"app-1",
				"event-1",
				undefined,
				LOAD,
			);
			if (changed === "profile") backend.profile.id = "profile-2";
			if (changed === "hub") backend.profile.hub = "https://hub-2.test";
			if (changed === "user") backend.auth.user.profile.sub = "user-2";
			if (changed === "token") backend.auth.user.access_token = "token-2";
			if (changed === "origin") mocks.apiBase = "https://api-2.test";
			await execute(state, prepared);
			expect(mocks.apiPost).toHaveBeenCalledTimes(2);
		},
	);

	test("does not reuse another backend's response, a copy, or a versioned prerun", async () => {
		const first = new WebEventState({});
		const second = new WebEventState({});
		const prepared = await first.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		await execute(second, prepared);
		await execute(first, { ...prepared });
		const versioned = await first.prerunEvent(
			"app-1",
			"event-1",
			[1, 0, 0],
			LOAD,
		);
		await execute(first, versioned);
		expect(mocks.apiPost).toHaveBeenCalledTimes(5);
	});

	test("a direct run does not consume a response from another pending dispatch", async () => {
		const state = new WebEventState({});
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		await execute(state);
		await execute(state, prepared);
		expect(mocks.apiPost).toHaveBeenCalledTimes(2);
	});

	test("reused metadata still requires OAuth consent before invocation", async () => {
		const state = new WebEventState({});
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		mocks.checkOAuthTokensFromPrerun.mockResolvedValue({
			...noProviders,
			tokens: { provider: { access_token: "oauth-token" } },
			requiredProviders: [{ id: "provider", name: "Provider" }],
		});
		await expect(execute(state, prepared)).rejects.toMatchObject({
			isOAuthError: true,
		});
		expect(fetch).not.toHaveBeenCalled();
		expect(mocks.apiPost).toHaveBeenCalledTimes(1);
	});

	test("an expired response cannot bypass a removed Page action", async () => {
		const state = new WebEventState({});
		const prepared = await state.prerunEvent(
			"app-1",
			"event-1",
			undefined,
			LOAD,
		);
		now += 15_001;
		mocks.apiPost.mockRejectedValueOnce(new Error("Page action removed"));
		await expect(execute(state, prepared)).rejects.toThrow(
			"Page action removed",
		);
		expect(fetch).not.toHaveBeenCalled();
		expect(mocks.notifyPageContractRejected).toHaveBeenCalled();
	});
});
