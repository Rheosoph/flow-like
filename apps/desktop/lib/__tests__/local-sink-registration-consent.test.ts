import { type IEvent, IEventExecutionMode } from "@flow-like/flow-like-ui";
import {
	isDeviceEventSource,
	withDeviceEventSource,
} from "@flow-like/flow-like-ui/lib/event-source";
import { resetDeviceEventCreationCache } from "@flow-like/flow-like-ui/lib/event-source-capability";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	consent: vi.fn(),
	toast: Object.assign(vi.fn(), {
		warning: vi.fn(),
		error: vi.fn(),
		info: vi.fn(),
	}),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	IEventExecutionMode: { Local: "Local", Remote: "Remote" },
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../api", () => ({ fetcher: mocks.fetcher, streamFetcher: vi.fn() }));
vi.mock("../oauth-db", () => ({ oauthConsentStore: {}, oauthTokenStore: {} }));
vi.mock("../oauth-service", () => ({ oauthService: {} }));
vi.mock("../../components/rpa", () => ({}));
vi.mock(
	"../../components/local-sink/local-sink-consent",
	async (importOriginal) => ({
		...(await importOriginal<
			typeof import("../../components/local-sink/local-sink-consent")
		>()),
		requestLocalSinkConsent: mocks.consent,
	}),
);
vi.mock("sonner", () => ({ toast: mocks.toast }));

import { EventState } from "../../components/tauri-provider/event-state";

type Plan = "none" | "existing" | "new";

function event(overrides: Partial<IEvent> = {}): IEvent {
	return {
		id: "event-1",
		name: "Nightly report",
		description: "",
		active: true,
		event_type: "cron",
		execution_mode: IEventExecutionMode.Local,
		board_id: "",
		node_id: "node-1",
		config: [],
		event_version: [0, 0, 0],
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		priority: 0,
		variables: {},
		...overrides,
	};
}

function offlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(true),
		boardState: { getBoard: vi.fn() },
	};
}

function onlineBackend() {
	return {
		isOffline: vi.fn().mockResolvedValue(false),
		boardState: { getBoard: vi.fn() },
		profile: { id: "profile-1" },
		auth: { user: { access_token: "token" } },
		queryClient: { setQueryData: vi.fn() },
	};
}

/** Signing out keeps the auth context, without a user. */
function signedOutBackend() {
	return {
		...onlineBackend(),
		auth: { user: null },
		boardState: { getBoard: vi.fn(), getBoardRunRequirements: vi.fn() },
	};
}

function backendPlans(
	registration: Plan,
	options: {
		stored?: IEvent;
		needsToken?: boolean;
		movesFromApp?: string;
		saved?: Record<string, unknown>;
	} = {},
) {
	mocks.invoke.mockImplementation((command: string) => {
		if (command === "get_event") {
			return options.stored
				? Promise.resolve(options.stored)
				: Promise.reject(new Error("Event not found"));
		}
		if (command === "local_trigger_plan") {
			return Promise.resolve({
				registration,
				needs_token: options.needsToken ?? false,
				...(options.movesFromApp
					? { moves_from_app: options.movesFromApp }
					: {}),
			});
		}
		if (command === "verify_trigger_token") return Promise.resolve(null);
		if (command === "upsert_event") {
			return Promise.resolve({
				...event(),
				trigger_state: "ready",
				...options.saved,
			});
		}
		if (command === "restore_event") {
			return Promise.resolve({ plan: { restored: event(), issues: [] } });
		}
		if (command === "sync_local_trigger") return Promise.resolve("ready");
		if (command === "local_trigger_status") return Promise.resolve("ready");
		throw new Error(`Unexpected command: ${command}`);
	});
}

function calls(command: string) {
	return mocks.invoke.mock.calls
		.filter(([name]) => name === command)
		.map(([, args]) => args as Record<string, unknown>);
}

const githubToken = {
	github: { access_token: "gho_token", provider_id: "github" },
} as never;

describe("local trigger consent when saving an event", () => {
	beforeEach(() => {
		vi.resetAllMocks();
		resetDeviceEventCreationCache();
		backendPlans("new");
	});

	test("registers a new trigger once the user allows it", async () => {
		mocks.consent.mockResolvedValue({ answer: "allow" });
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(calls("local_trigger_plan")[0]).toMatchObject({
			appId: "app-1",
		});
		expect(mocks.consent).toHaveBeenCalledWith({
			appId: "app-1",
			eventId: "event-1",
			eventName: "Nightly report",
			eventType: "cron",
			approve: true,
			needsToken: false,
		});
		expect(calls("upsert_event")[0]?.registerSink).toBe("register");
		expect(calls("upsert_event")[0]).not.toHaveProperty("offline");
	});

	test("saves the event without a trigger when the user declines", async () => {
		mocks.consent.mockResolvedValue({ answer: "decline" });
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(calls("upsert_event")[0]?.registerSink).toBe("skip");
	});

	test("creates a device event without registering a source trigger or requesting local permissions", async () => {
		const backend = offlineBackend();
		const state = new EventState(backend as never);
		const target = event({ board_id: "board-1" });

		await state.upsertEvent("app-1", target, undefined, undefined, undefined, {
			source: "device",
		});

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(backend.boardState.getBoard).not.toHaveBeenCalled();
		expect(calls("local_trigger_plan")).toHaveLength(0);
		expect(calls("upsert_event")).toEqual([
			expect.objectContaining({
				event: withDeviceEventSource(target),
				enforceId: true,
				registerSink: "skip",
			}),
		]);
		expect(target.active).toBe(true);
	});

	test("disables both hub and local source triggers when creating an online device event", async () => {
		const state = new EventState(onlineBackend() as never);
		const target = event();
		mocks.fetcher
			.mockResolvedValueOnce({ device_event_creation: true })
			.mockResolvedValueOnce(withDeviceEventSource(target));

		await state.upsertEvent("app-1", target, undefined, undefined, undefined, {
			source: "device",
		});

		const request = mocks.fetcher.mock.calls[1]?.[2] as { body: string };
		expect(JSON.parse(request.body)).toMatchObject({
			register_source: false,
			event: { id: target.id, active: true },
		});
		expect(calls("upsert_event")[0]).toMatchObject({
			event: withDeviceEventSource(target),
			registerSink: "skip",
		});
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("refuses an older hub before writing either source copy", async () => {
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockResolvedValue({});

		await expect(
			state.upsertEvent("app-1", event(), undefined, undefined, undefined, {
				source: "device",
			}),
		).rejects.toThrow("Update the hub");

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("editing a device event keeps source triggers disabled without creation options", async () => {
		const state = new EventState(offlineBackend() as never);
		const target = withDeviceEventSource(event({ name: "Updated schedule" }));

		await state.upsertEvent("app-1", target);

		expect(calls("upsert_event")[0]).toMatchObject({
			event: target,
			registerSink: "skip",
		});
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("a marker-stripped save of a stored device event asks for no approval", async () => {
		const stored = withDeviceEventSource(event({ board_id: "board-1" }));
		backendPlans("new", { stored });
		const backend = offlineBackend();
		const state = new EventState(backend as never);

		await state.upsertEvent(
			"app-1",
			event({ board_id: "board-1", name: "Renamed" }),
		);

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(backend.boardState.getBoard).not.toHaveBeenCalled();
		expect(calls("local_trigger_plan")).toHaveLength(0);
		const saved = calls("upsert_event")[0];
		expect(saved?.registerSink).toBe("skip");
		expect(isDeviceEventSource(saved?.event as IEvent)).toBe(true);
	});

	test("keeps the hub source disabled when a stripped save of a device event is online", async () => {
		backendPlans("new", { stored: withDeviceEventSource(event()) });
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockResolvedValueOnce(withDeviceEventSource(event()));

		await state.upsertEvent("app-1", event());

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		const request = mocks.fetcher.mock.calls[0]?.[2] as { body: string };
		expect(JSON.parse(request.body)).toMatchObject({ register_source: false });
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("an event unknown locally is saved like an ordinary event", async () => {
		mocks.consent.mockResolvedValue({ answer: "allow" });
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(mocks.consent).toHaveBeenCalledTimes(1);
		expect(calls("upsert_event")[0]?.registerSink).toBe("register");
	});

	test("an explicit clear of a device event asks for consent like an ordinary event", async () => {
		backendPlans("new", { stored: withDeviceEventSource(event()) });
		mocks.consent.mockResolvedValue({ answer: "allow" });
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent(
			"app-1",
			withDeviceEventSource(event()),
			undefined,
			undefined,
			undefined,
			{ source: "default" },
		);

		expect(mocks.consent).toHaveBeenCalledTimes(1);
		const saved = calls("upsert_event")[0];
		expect(saved?.registerSink).toBe("register");
		expect(isDeviceEventSource(saved?.event as IEvent)).toBe(false);
		expect(calls("local_trigger_plan")[0]?.event).toMatchObject({
			id: "event-1",
		});
	});

	test("an explicit clear tells the hub to register the source", async () => {
		backendPlans("existing");
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockResolvedValueOnce(event());

		await state.upsertEvent(
			"app-1",
			withDeviceEventSource(event()),
			undefined,
			undefined,
			undefined,
			{ source: "default" },
		);

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		const request = mocks.fetcher.mock.calls[0]?.[2] as { body: string };
		expect(JSON.parse(request.body)).toMatchObject({ register_source: true });
		expect(calls("upsert_event")[0]?.registerSink).toBe("register");
	});

	test("checks a token the caller hands in and never sends it to the hub", async () => {
		backendPlans("existing");
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockResolvedValueOnce(event());

		await state.upsertEvent("app-1", event(), undefined, "pat_id.secret");

		expect(calls("verify_trigger_token")).toEqual([
			{ appId: "app-1", pat: "pat_id.secret" },
		]);
		const request = mocks.fetcher.mock.calls[0]?.[2] as { body: string };
		expect(JSON.parse(request.body)).not.toHaveProperty("pat");
		expect(calls("upsert_event")[0]?.pat).toBe("pat_id.secret");
	});

	test("a token the caller hands in that the hub refuses stops the save before any write", async () => {
		backendPlans("existing");
		const verify = mocks.invoke.getMockImplementation();
		mocks.invoke.mockImplementation((command: string, args: unknown) =>
			command === "verify_trigger_token"
				? Promise.reject({
						error:
							"TRIGGER_TOKEN_WRONG_ACCOUNT: This personal access token belongs to another account; choose one of your own",
					})
				: verify?.(command, args),
		);
		const state = new EventState(onlineBackend() as never);

		await expect(
			state.upsertEvent("app-1", event(), undefined, "pat_other.secret"),
		).rejects.toThrow(
			"This token belongs to another account. Choose one of your own tokens for this hub.",
		);
		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("a caller's token spares only the token step", async () => {
		backendPlans("new", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "allow" });
		mocks.fetcher.mockResolvedValueOnce(event());
		const state = new EventState(onlineBackend() as never);

		await state.upsertEvent("app-1", event(), undefined, "pat_id.secret");

		expect(mocks.consent).toHaveBeenCalledWith(
			expect.objectContaining({ approve: true, needsToken: false }),
		);
		expect(calls("upsert_event")[0]).toMatchObject({
			pat: "pat_id.secret",
			registerSink: "register",
		});
	});

	test("a new trigger without a token asks for approval and a token, then saves with the chosen token and the caller's OAuth", async () => {
		backendPlans("new", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "allow", pat: "pat_chosen" });
		mocks.fetcher.mockResolvedValueOnce(event());
		const state = new EventState(onlineBackend() as never);

		await state.upsertEvent(
			"app-1",
			event(),
			undefined,
			undefined,
			githubToken,
		);

		expect(mocks.consent).toHaveBeenCalledWith(
			expect.objectContaining({ approve: true, needsToken: true }),
		);
		expect(calls("upsert_event")[0]).toMatchObject({
			pat: "pat_chosen",
			oauthTokens: githubToken,
			registerSink: "register",
		});
		expect(calls("verify_trigger_token")).toHaveLength(0);
		const request = mocks.fetcher.mock.calls[0]?.[2] as { body: string };
		expect(JSON.parse(request.body)).not.toHaveProperty("pat");
	});

	test("a trigger that holds a token of this account saves without asking", async () => {
		backendPlans("existing");
		mocks.fetcher.mockResolvedValueOnce(event());
		const state = new EventState(onlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]).toMatchObject({
			registerSink: "register",
		});
		expect(calls("upsert_event")[0]?.pat).toBeUndefined();
	});

	test("an existing trigger without a token asks only for the token", async () => {
		backendPlans("existing", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "allow", pat: "pat_chosen" });
		mocks.fetcher.mockResolvedValueOnce(event());
		const state = new EventState(onlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(mocks.consent).toHaveBeenCalledWith(
			expect.objectContaining({ approve: false, needsToken: true }),
		);
		expect(calls("upsert_event")[0]?.pat).toBe("pat_chosen");
	});

	test("a save shaped like FlowPilot's, without a token or OAuth, still asks for a token", async () => {
		backendPlans("new", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "decline" });
		mocks.fetcher.mockResolvedValueOnce(event());
		const state = new EventState(onlineBackend() as never);

		await state.upsertEvent("app-1", event(), undefined, undefined, undefined);

		expect(mocks.consent).toHaveBeenCalledWith(
			expect.objectContaining({ needsToken: true }),
		);
		expect(calls("upsert_event")[0]?.registerSink).toBe("skip");
	});

	test("names the app a trigger moves from", async () => {
		backendPlans("new", { movesFromApp: "app-offline" });
		mocks.consent.mockResolvedValue({ answer: "allow" });
		const backend = {
			...offlineBackend(),
			appState: {
				getAppMeta: vi.fn().mockResolvedValue({ name: "Offline copy" }),
			},
		};
		const state = new EventState(backend as never);

		await state.upsertEvent("app-1", event());

		expect(backend.appState.getAppMeta).toHaveBeenCalledWith("app-offline");
		expect(mocks.consent).toHaveBeenCalledWith(
			expect.objectContaining({ approve: true, movesFromApp: "Offline copy" }),
		);
	});

	test("reports a trigger that could not follow the save without failing it", async () => {
		const triggerError =
			"Event was saved, but its trigger on this computer was not registered: bot login failed";
		backendPlans("existing", {
			saved: {
				trigger_state: { failed: { reason: "bot login failed" } },
				trigger_error: triggerError,
			},
		});
		const state = new EventState(offlineBackend() as never);

		const saved = await state.upsertEvent("app-1", event());

		expect(saved.trigger_error).toBe(triggerError);
		expect(saved).not.toHaveProperty("trigger_state");
		expect(mocks.toast.warning).toHaveBeenCalledWith("Nightly report", {
			description: triggerError,
		});
	});

	test("resolves with the stored event alone when the trigger followed the save", async () => {
		backendPlans("existing");
		const state = new EventState(offlineBackend() as never);

		await expect(state.upsertEvent("app-1", event())).resolves.toEqual(event());
		expect(mocks.toast.warning).not.toHaveBeenCalled();
	});

	test("probes the hub once for consecutive device creations", async () => {
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockImplementation((_profile, path: string) =>
			Promise.resolve(
				path.endsWith("device-placements")
					? { device_event_creation: true }
					: event(),
			),
		);

		for (const id of ["event-1", "event-2"]) {
			await state.upsertEvent(
				"app-1",
				event({ id }),
				undefined,
				undefined,
				undefined,
				{ source: "device" },
			);
		}

		const probes = mocks.fetcher.mock.calls.filter(([, path]) =>
			String(path).endsWith("device-placements"),
		);
		expect(probes).toHaveLength(1);
	});

	test("explains a forbidden capability probe before writing either copy", async () => {
		const state = new EventState(onlineBackend() as never);
		mocks.fetcher.mockRejectedValue(
			Object.assign(new Error("forbidden"), { status: 403 }),
		);

		await expect(
			state.upsertEvent("app-1", event(), undefined, undefined, undefined, {
				source: "device",
			}),
		).rejects.toThrow("cannot manage devices");

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("reports a failed removal of the source trigger before deployment can start", async () => {
		const state = new EventState(offlineBackend() as never);
		mocks.invoke.mockRejectedValue(new Error("Source trigger removal failed"));

		await expect(
			state.upsertEvent("app-1", event(), undefined, undefined, undefined, {
				source: "device",
			}),
		).rejects.toThrow("Source trigger removal failed");
	});

	test("abandons the save when a dialog is dismissed", async () => {
		backendPlans("new", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "dismiss" });
		const state = new EventState(onlineBackend() as never);

		await expect(state.upsertEvent("app-1", event())).rejects.toMatchObject({
			isLocalSinkConsentDismissed: true,
		});
		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("a signed-out save of an online app fails before anyone is asked", async () => {
		const state = new EventState({
			...onlineBackend(),
			auth: undefined,
		} as never);

		await expect(state.upsertEvent("app-1", event())).rejects.toThrow(
			"Profile, auth or query client not set",
		);
		expect(calls("local_trigger_plan")).toHaveLength(0);
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("a save that needs a token while signed out fails before anyone is asked", async () => {
		backendPlans("new", { needsToken: true });
		const backend = signedOutBackend();
		const state = new EventState(backend as never);

		await expect(
			state.upsertEvent("app-1", event({ board_id: "board-1" })),
		).rejects.toThrow(
			"Sign in to the hub before choosing a token for the trigger of Nightly report",
		);
		expect(backend.boardState.getBoardRunRequirements).not.toHaveBeenCalled();
		expect(mocks.consent).not.toHaveBeenCalled();
		expect(mocks.fetcher).not.toHaveBeenCalled();
		expect(calls("upsert_event")).toHaveLength(0);
	});

	test("a signed-out restore that needs a token fails before anyone is asked", async () => {
		backendPlans("new", { needsToken: true });
		const plans = mocks.invoke.getMockImplementation();
		mocks.invoke.mockImplementation((command: string, args: unknown) =>
			command === "restore_event"
				? Promise.resolve({
						plan: { restored: event({ board_id: "board-1" }), issues: [] },
					})
				: plans?.(command, args),
		);
		const backend = signedOutBackend();
		const state = new EventState(backend as never);

		await expect(
			state.restoreEvent("app-1", "event-1", [0, 0, 1], { dryRun: false }),
		).rejects.toThrow("Sign in to the hub");
		expect(calls("restore_event").map((args) => args.dryRun)).toEqual([true]);
		expect(backend.boardState.getBoardRunRequirements).not.toHaveBeenCalled();
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("a restore that registers no trigger needs no hub session", async () => {
		backendPlans("none");
		const state = new EventState(signedOutBackend() as never);

		await state.restoreEvent("app-1", "event-1", [0, 0, 1], { dryRun: false });

		expect(calls("restore_event")[1]).toMatchObject({
			dryRun: false,
			registerSink: "keep",
		});
		expect(mocks.consent).not.toHaveBeenCalled();
	});

	test("refreshes an approved trigger without asking again", async () => {
		backendPlans("existing");
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event());

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]?.registerSink).toBe("register");
	});

	test("does not ask when nothing would register", async () => {
		backendPlans("none");
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event({ event_type: "chat" }));

		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]?.registerSink).toBe("keep");
	});

	test("leaves an inactive event to the backend without asking", async () => {
		const state = new EventState(offlineBackend() as never);

		await state.upsertEvent("app-1", event({ active: false }));

		expect(calls("local_trigger_plan")).toHaveLength(0);
		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("upsert_event")[0]?.registerSink).toBe("keep");
	});

	test("asks for the token before the server write and passes it to the local mirror", async () => {
		backendPlans("new", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "allow", pat: "pat_chosen" });
		const saved = event({ name: "Saved on the server" });
		mocks.fetcher.mockImplementation(async () => {
			expect(mocks.consent).toHaveBeenCalled();
			return saved;
		});
		const state = new EventState(onlineBackend() as never);

		await expect(state.upsertEvent("app-1", event())).resolves.toEqual(saved);

		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(calls("upsert_event")[0]).toMatchObject({
			event: saved,
			pat: "pat_chosen",
			registerSink: "register",
		});
	});

	test("a non-dry restore goes through the same approval and passes the chosen token", async () => {
		backendPlans("new", { needsToken: true });
		mocks.consent.mockResolvedValue({ answer: "allow", pat: "pat_chosen" });
		const state = new EventState(onlineBackend() as never);

		await state.restoreEvent("app-1", "event-1", [0, 0, 1], {
			dryRun: false,
		});

		const restores = calls("restore_event");
		expect(restores.map((args) => args.dryRun)).toEqual([true, false]);
		expect(restores[1]).toMatchObject({
			registerSink: "register",
			pat: "pat_chosen",
		});
		expect(restores[1]).not.toHaveProperty("offline");
		expect(mocks.consent).toHaveBeenCalledTimes(1);
	});

	test("a restore reports a trigger that could not follow it once", async () => {
		const triggerError =
			"Event was saved, but its trigger on this computer was not registered: bot login failed";
		backendPlans("existing");
		const plans = mocks.invoke.getMockImplementation();
		mocks.invoke.mockImplementation((command: string, args: unknown) =>
			command === "restore_event" &&
			(args as { dryRun?: boolean }).dryRun === false
				? Promise.resolve({
						plan: { restored: event(), issues: [] },
						event: {
							...event(),
							trigger_state: { failed: { reason: "bot login failed" } },
							trigger_error: triggerError,
						},
						setup_status: `error: ${triggerError}`,
					})
				: plans?.(command, args),
		);
		const state = new EventState(offlineBackend() as never);

		const result = await state.restoreEvent("app-1", "event-1", [0, 0, 1], {
			dryRun: false,
		});

		expect(result.event).toEqual(event());
		expect(result).not.toHaveProperty("setup_status");
		expect(mocks.toast.warning).toHaveBeenCalledTimes(1);
	});

	test("a dry-run restore never asks", async () => {
		const state = new EventState(offlineBackend() as never);

		await state.restoreEvent("app-1", "event-1", [0, 0, 1]);

		expect(calls("restore_event")).toHaveLength(1);
		expect(calls("local_trigger_plan")).toHaveLength(0);
	});
});

describe("adding a token to a trigger on this computer", () => {
	beforeEach(() => {
		vi.resetAllMocks();
		backendPlans("existing", { needsToken: true });
	});

	test("signs the trigger in through sync_local_trigger alone", async () => {
		mocks.consent.mockResolvedValue({ answer: "allow", pat: "pat_chosen" });
		const state = new EventState(onlineBackend() as never);

		await expect(state.repairLocalTrigger("app-1", event())).resolves.toEqual({
			active: true,
		});

		expect(mocks.consent).toHaveBeenCalledWith(
			expect.objectContaining({
				approve: false,
				needsToken: true,
				repair: true,
			}),
		);
		expect(calls("sync_local_trigger")).toEqual([
			{
				appId: "app-1",
				eventId: "event-1",
				registerSink: "register",
				pat: "pat_chosen",
			},
		]);
		expect(calls("upsert_event")).toHaveLength(0);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test.each(["decline", "dismiss"] as const)(
		"a prompt answered with %s rejects as a dismissal and leaves the trigger as it is",
		async (answer) => {
			mocks.consent.mockResolvedValue({ answer });
			const state = new EventState(onlineBackend() as never);

			await expect(
				state.repairLocalTrigger("app-1", event()),
			).rejects.toMatchObject({ isLocalSinkConsentDismissed: true });
			expect(calls("sync_local_trigger")).toHaveLength(0);
		},
	);

	test("a trigger that already holds a token only reads its status", async () => {
		backendPlans("existing");
		const state = new EventState(onlineBackend() as never);

		await expect(state.repairLocalTrigger("app-1", event())).resolves.toEqual({
			active: true,
		});
		expect(calls("local_trigger_status")).toHaveLength(1);
		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("sync_local_trigger")).toHaveLength(0);
	});

	test("asks a signed-out user to sign in instead of offering tokens", async () => {
		const backend = signedOutBackend();
		const state = new EventState(backend as never);

		await expect(
			state.repairLocalTrigger("app-1", event({ board_id: "board-1" })),
		).rejects.toThrow(
			"Sign in to the hub before choosing a token for the trigger of Nightly report",
		);
		expect(backend.boardState.getBoardRunRequirements).not.toHaveBeenCalled();
		expect(mocks.consent).not.toHaveBeenCalled();
		expect(calls("sync_local_trigger")).toHaveLength(0);
	});

	test("refuses to create a trigger that a save should register", async () => {
		backendPlans("new", { needsToken: true });
		const state = new EventState(onlineBackend() as never);

		await expect(state.repairLocalTrigger("app-1", event())).rejects.toThrow(
			"save the event to register one",
		);
		expect(mocks.consent).not.toHaveBeenCalled();
	});
});
