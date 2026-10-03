import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { createHash } from "node:crypto";
import type { IRegistryState } from "../../state/backend-state/registry-state";
import {
	type MicroWidgetConsentTarget,
	blockMicroWidgetRuntimeSources,
	grantMicroWidgetConsent,
	microWidgetConsentKey,
	muteMicroWidgetRuntime,
	readMicroWidgetRuntimeBlocks,
	resetMicroWidgetConsentForTests,
	revokeMicroWidgetConsent,
} from "./micro-widget-capability-consent";
import {
	type WidgetAccessRequest,
	WidgetBundleUnavailableError,
	type WidgetGrantRequest,
	type WidgetPolicy,
	WidgetPolicyChangedError,
	type WidgetPolicyRequest,
	type WidgetRuntimeStatus,
	type WidgetSourceLevel,
	isWebWidgetGrant,
	maxWidgetSourceLevel,
	parseWidgetGrantResponse,
	parseWidgetPolicyDescriptor,
} from "./micro-widget-policy";
import {
	MICRO_WIDGET_ACCESS_RETRY_DELAYS_MS,
	MICRO_WIDGET_GRANT_CACHE_LIMIT,
	MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS,
	type MicroWidgetAccess,
	type MicroWidgetFrameMount,
	type MicroWidgetGrant,
	type MicroWidgetGrantClock,
	MicroWidgetGrantController,
	type MicroWidgetGrantControllerInputs,
	forgetMicroWidgetAccess,
	loadMicroWidgetAccess,
	microWidgetGrantCacheSizeForTests,
	requestMicroWidgetAccess,
	resetMicroWidgetGrantCacheForTests,
} from "./use-micro-widget-grant";

const PACKAGE = "com.example.maps";
const WIDGET = "live-map";
const HASH = "b".repeat(64);
const APP = "app1";
const SLOT = { path: "layers[].url", purpose: 1, directives: ["imgSrc"] };
const DECLARED: WidgetPolicy = {
	workers: true,
	csp: { connectSrc: ["https://api.cesium.com"] },
};
const A = "https://a.tiles.example.com";
const B = "https://b.tiles.example.com";
const C = "https://c.tiles.example.com";
const TARGET: MicroWidgetConsentTarget = {
	source: "hub",
	appId: APP,
	packageId: PACKAGE,
	widgetId: WIDGET,
};
const HOUR = 3_600_000;
/** A wall clock reading far from the monotonic one, so a wait measured across both shows. */
const WALL = Date.UTC(2026, 9, 2, 8);

function digest(value: unknown): string {
	return `sha256:${createHash("sha256").update(JSON.stringify(value)).digest("hex")}`;
}

class FakeClock implements MicroWidgetGrantClock {
	/** The monotonic clock, which the timers run on. */
	time = 1_000;
	/** What the wall clock reads at a monotonic time: the same, unless a test moves it. */
	wall: (time: number) => number = (time) => time;
	/** Timers fire this long before the monotonic clock reaches their time. */
	early = 0;
	private nextId = 1;
	private timers = new Map<number, { at: number; callback: () => void }>();

	now = () => this.wall(this.time);

	monotonic = () => this.time;

	setTimeout = (callback: () => void, ms: number) => {
		const id = this.nextId++;
		this.timers.set(id, { at: this.time + ms - this.early, callback });
		return id;
	};

	clearTimeout = (handle: unknown) => {
		this.timers.delete(handle as number);
	};

	get pending() {
		return this.timers.size;
	}

	async advance(ms: number) {
		const target = this.time + ms;
		for (;;) {
			const due = [...this.timers.entries()]
				.filter(([, timer]) => timer.at <= target)
				.sort(([, left], [, right]) => left.at - right.at)[0];
			if (!due) break;
			this.timers.delete(due[0]);
			this.time = due[1].at;
			due[1].callback();
			await flush();
		}
		this.time = target;
		await flush();
	}
}

async function flush() {
	for (let round = 0; round < 30; round++) await Promise.resolve();
}

interface BackendOptions {
	levels?: Record<string, WidgetSourceLevel>;
	reject?: Record<string, string>;
	runtimeStatus?: WidgetRuntimeStatus;
	describeRuntime?: (request: WidgetPolicyRequest) => unknown;
	mint?: (request: WidgetGrantRequest, call: number) => unknown;
}

function describeResponse(
	request: WidgetPolicyRequest,
	options: BackendOptions,
) {
	const runtimeSources = request.runtimeSources ?? [];
	const requested = [
		...new Set(runtimeSources.flatMap((entry) => entry.sources)),
	].sort();
	const rejected = requested
		.filter((source) => options.reject?.[source])
		.map((source) => ({
			slot: SLOT.path,
			source,
			code: options.reject?.[source] ?? "",
		}));
	const accepted = requested.filter((source) => !options.reject?.[source]);
	const status: WidgetRuntimeStatus =
		runtimeSources.length === 0 ? "none" : (options.runtimeStatus ?? "ok");
	const withRuntime = status === "ok" && accepted.length > 0;
	const policy: WidgetPolicy = withRuntime
		? { ...DECLARED, csp: { ...DECLARED.csp, imgSrc: accepted } }
		: DECLARED;
	const level = (source: string) => options.levels?.[source] ?? "external";
	const runtimeLevels = withRuntime ? accepted.map(level) : [];
	return {
		source: "hub",
		packageId: PACKAGE,
		packageVersion: "1.0.0",
		bundleHash: HASH,
		widgetId: WIDGET,
		preview: false,
		status: "ok",
		policy,
		policyDigest: digest(policy),
		networkInputs: [SLOT],
		platformStorage: [],
		engine: { wildcardSources: true, runtimeSources: true, localMedia: true },
		runtime: {
			status,
			declaredDigest: digest(DECLARED),
			...(withRuntime ? { runtimeDigest: digest(accepted) } : {}),
			rejected,
			...(status === "invalid" ? { invalidReason: "too-many-sources" } : {}),
			...(status === "unavailable" ? { invalidReason: "engine" } : {}),
		},
		network: {
			level: maxWidgetSourceLevel(["external", ...runtimeLevels]),
			catalogVersion: 1,
			pslVersion: "2026-09-15_10-18-26_UTC",
			stale: false,
			purposes: [
				{
					reason: "Loads globe terrain from Cesium ion",
					level: "external",
					sources: [
						{
							source: "https://api.cesium.com",
							directives: ["connectSrc"],
							origin: "declared",
							kind: "service",
							level: "external",
							host: "api.cesium.com",
							emphasis: "cesium.com",
						},
					],
				},
				{
					reason: "Loads map tiles given to it at runtime",
					level: maxWidgetSourceLevel(["external", ...runtimeLevels]),
					inputs: [SLOT.path],
					sources: withRuntime
						? accepted.map((source) => ({
								source,
								directives: ["imgSrc"],
								origin: "runtime",
								slot: SLOT.path,
								kind: "exact",
								level: level(source),
								host: new URL(source).host,
								emphasis: "example.com",
							}))
						: [],
				},
			],
		},
	};
}

function stubBackend(options: BackendOptions = {}) {
	const calls = {
		describe: [] as WidgetPolicyRequest[],
		mint: [] as WidgetGrantRequest[],
	};
	const registry = {
		describeWidgetPolicy: async (request: WidgetPolicyRequest) => {
			calls.describe.push(structuredClone(request));
			const raw =
				request.runtimeSources?.length && options.describeRuntime
					? options.describeRuntime(request)
					: describeResponse(request, options);
			return parseWidgetPolicyDescriptor(raw, { packageId: PACKAGE });
		},
		mintWidgetGrant: async (request: WidgetGrantRequest) => {
			calls.mint.push(structuredClone(request));
			const answer =
				options.mint?.(request, calls.mint.length) ??
				mintAnswer(request, calls.mint.length);
			return parseWidgetGrantResponse(answer, isWebWidgetGrant);
		},
	} as unknown as IRegistryState;
	return { calls, registry };
}

/** The stub's grant for the `call`-th mint: an hour long, with the runtime sources that were asked for. */
function mintAnswer(request: WidgetGrantRequest, call: number) {
	return {
		grant: `h.p${call}.s`,
		expiresIn: 3600,
		policyDigest: request.policyDigest,
		runtime: request.runtimeSources?.length
			? Buffer.from(
					JSON.stringify(
						Object.fromEntries(
							request.runtimeSources.map(({ slot, sources }) => [
								slot,
								sources,
							]),
						),
					),
				).toString("base64url")
			: null,
	};
}

function layers(...urls: string[]) {
	return { layers: urls.map((url) => ({ url: `${url}/t/1.png?sig=x` })) };
}

function inputsFor(
	registry: IRegistryState,
	props: Record<string, unknown>,
	overrides: Partial<MicroWidgetGrantControllerInputs> = {},
): MicroWidgetGrantControllerInputs {
	return {
		registry,
		active: true,
		packageId: PACKAGE,
		packageVersion: "1.0.0",
		bundleHash: HASH,
		widgetId: WIDGET,
		preview: false,
		appId: APP,
		legacyAllowed: false,
		legacyPolicy: {},
		props,
		propsKey: JSON.stringify(props),
		...overrides,
	};
}

let clock: FakeClock;
const controllers: MicroWidgetGrantController[] = [];
const restorers: (() => void)[] = [];

function installLocalStorage(): Map<string, string> {
	const items = new Map<string, string>();
	const previous = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			getItem: (key: string) => items.get(key) ?? null,
			setItem: (key: string, value: string) => void items.set(key, value),
			removeItem: (key: string) => void items.delete(key),
			key: (index: number) => [...items.keys()][index] ?? null,
			get length() {
				return items.size;
			},
		},
	});
	restorers.push(() => {
		if (previous) Object.defineProperty(globalThis, "localStorage", previous);
		else Reflect.deleteProperty(globalThis, "localStorage");
	});
	return items;
}

function mount(
	backend: ReturnType<typeof stubBackend>,
	props: Record<string, unknown>,
	overrides: Partial<MicroWidgetGrantControllerInputs> = {},
) {
	const controller = new MicroWidgetGrantController(clock);
	controllers.push(controller);
	let current = inputsFor(backend.registry, props, overrides);
	controller.setInputs(current);
	controller.activate();
	return {
		controller,
		get grant(): MicroWidgetGrant {
			return controller.getSnapshot();
		},
		setProps(next: Record<string, unknown>) {
			current = { ...current, props: next, propsKey: JSON.stringify(next) };
			controller.setInputs(current);
		},
	};
}

function frameOf(grant: MicroWidgetGrant): MicroWidgetFrameMount {
	if (grant.state.status !== "ready") {
		throw new Error(`expected a ready frame, got ${grant.state.status}`);
	}
	return grant.state.frame;
}

function runtimeSourcesOf(request: WidgetPolicyRequest | undefined) {
	return request?.runtimeSources?.flatMap((entry) => entry.sources) ?? [];
}

function grantRuntime(...sources: string[]) {
	grantMicroWidgetConsent(
		TARGET,
		{
			policy: DECLARED,
			levels: { "https://api.cesium.com": "external" },
			runtime: sources.map((source) => ({
				directive: "imgSrc" as const,
				source,
				level: "external" as const,
				slot: SLOT.path,
			})),
		},
		"session",
	);
}

beforeEach(() => {
	clock = new FakeClock();
	resetMicroWidgetConsentForTests();
	resetMicroWidgetGrantCacheForTests();
});

afterEach(() => {
	for (const controller of controllers.splice(0)) controller.deactivate();
	for (const restore of restorers.splice(0).reverse()) restore();
});

describe("mount", () => {
	test("describes the extracted origins and mints with exactly the approved request", async () => {
		const backend = stubBackend();
		const view = mount(backend, layers(A, B));
		await flush();

		expect(backend.calls.describe).toEqual([
			{
				packageId: PACKAGE,
				packageVersion: "1.0.0",
				bundleHash: HASH,
				widgetId: WIDGET,
				preview: false,
				appId: APP,
			},
			{
				packageId: PACKAGE,
				packageVersion: "1.0.0",
				bundleHash: HASH,
				widgetId: WIDGET,
				preview: false,
				appId: APP,
				runtimeSources: [{ slot: SLOT.path, sources: [A, B] }],
			},
		]);
		const prompt = view.grant.prompt;
		expect(view.grant.state.status).toBe("pending");
		expect(prompt?.mode).toBe("mount");
		expect(prompt?.declared.policy).toEqual(DECLARED);
		expect(prompt?.declared.covered).toBe(false);
		expect(prompt?.runtime.pending.map((entry) => entry.source)).toEqual([
			A,
			B,
		]);
		expect(prompt?.runtime.slots).toEqual([SLOT.path]);
		expect(prompt?.hasRuntimeCheckbox).toBe(true);
		expect(prompt?.includeRuntimeDefault).toBe(true);
		expect(prompt?.includeRuntime).toBe(true);
		expect(prompt?.newSources).toEqual([A, "https://api.cesium.com", B]);
		expect(prompt?.subject.policy.csp?.imgSrc).toEqual([A, B]);

		view.controller.actions.allowOnce();
		await flush();
		expect(backend.calls.mint).toHaveLength(1);
		expect(backend.calls.mint[0]).toMatchObject({
			policyDigest: prompt?.descriptor?.policyDigest,
			appId: APP,
			runtimeSources: [{ slot: SLOT.path, sources: [A, B] }],
		});
		const frame = frameOf(view.grant);
		expect(frame.grant).toBe("h.p1.s");
		expect(frame.runtime).not.toBeNull();
		expect(frame.policy.csp?.imgSrc).toEqual([A, B]);
		expect(view.grant.prompt).toBeNull();
	});

	test("an empty first extraction waits for the first props patch", async () => {
		const backend = stubBackend();
		const view = mount(backend, { layers: [] });
		await flush();
		expect(backend.calls.describe).toHaveLength(1);
		expect(view.grant.state.status).toBe("describing");

		await clock.advance(200);
		view.setProps(layers(A));
		await flush();
		expect(runtimeSourcesOf(backend.calls.describe[1])).toEqual([A]);
		expect(view.grant.prompt?.runtime.pending).toHaveLength(1);
	});

	test("without a props patch the grace window ends declared-only", async () => {
		const backend = stubBackend();
		const view = mount(backend, { layers: [] });
		await flush();
		await clock.advance(499);
		expect(view.grant.state.status).toBe("describing");
		await clock.advance(1);
		expect(backend.calls.describe).toHaveLength(1);
		expect(view.grant.state.status).toBe("pending");
		expect(view.grant.prompt?.runtime.pending).toEqual([]);
		expect(view.grant.prompt?.hasRuntimeCheckbox).toBe(false);
	});

	test("a widget outside a project describes and mints without an app id", async () => {
		const backend = stubBackend();
		const view = mount(backend, {}, { appId: null });
		await flush();
		await clock.advance(1_000);
		view.controller.actions.allowOnce();
		await flush();
		expect(backend.calls.describe).toHaveLength(1);
		expect(backend.calls.describe[0]).not.toHaveProperty("appId");
		expect(backend.calls.mint).toHaveLength(1);
		expect(backend.calls.mint[0]).not.toHaveProperty("appId");
	});

	test("an unchecked runtime box grants declared-only, blocks the addresses and mints the declared digest", async () => {
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.controller.actions.setIncludeRuntime(false);
		expect(view.grant.prompt?.includeRuntime).toBe(false);
		expect(view.grant.prompt?.subject.policy).toEqual(DECLARED);

		view.controller.actions.allowOnce();
		await flush();
		expect(backend.calls.mint).toHaveLength(1);
		expect(backend.calls.mint[0].policyDigest).toBe(digest(DECLARED));
		expect(backend.calls.mint[0].runtimeSources).toBeUndefined();
		expect(backend.calls.mint[0].appId).toBe(APP);
		expect([...readMicroWidgetRuntimeBlocks(TARGET)]).toEqual([A]);
		expect(frameOf(view.grant).runtime).toBeNull();
	});

	test("the runtime box starts unchecked when anyone could receive what the widget sends", async () => {
		const backend = stubBackend({ levels: { [A]: "broad" } });
		const view = mount(backend, layers(A));
		await flush();
		expect(view.grant.prompt?.runtime.level).toBe("broad");
		expect(view.grant.prompt?.includeRuntimeDefault).toBe(false);
		expect(view.grant.prompt?.includeRuntime).toBe(false);
	});

	test("runtime-only prompts have no box and Don't allow mounts without the addresses", async () => {
		grantRuntime();
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		const prompt = view.grant.prompt;
		expect(prompt?.declared.covered).toBe(true);
		expect(prompt?.hasRuntimeCheckbox).toBe(false);
		expect(prompt?.canStopAsking).toBe(true);

		view.controller.actions.dontAllow();
		await flush();
		expect(view.grant.state.status).toBe("ready");
		expect(frameOf(view.grant).runtime).toBeNull();
		expect(backend.calls.mint.at(-1)?.policyDigest).toBe(digest(DECLARED));
		expect([...readMicroWidgetRuntimeBlocks(TARGET)]).toEqual([A]);
	});

	test("Don't allow with a pending declared part blocks the widget", async () => {
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.controller.actions.dontAllow();
		await flush();
		expect(view.grant.state.status).toBe("blocked");
		expect(backend.calls.mint).toEqual([]);
		view.controller.actions.runRestricted();
		expect(frameOf(view.grant)).toMatchObject({
			grant: null,
			notice: "baseline",
		});
	});

	test("covered runtime sources mint without a prompt", async () => {
		grantRuntime(A);
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		expect(view.grant.prompt).toBeNull();
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
	});

	test("session blocks and muting keep addresses out of the request", async () => {
		grantRuntime();
		blockMicroWidgetRuntimeSources(TARGET, [A]);
		const blocked = stubBackend();
		mount(blocked, layers(A, B));
		await flush();
		expect(runtimeSourcesOf(blocked.calls.describe[1])).toEqual([B]);

		muteMicroWidgetRuntime(TARGET);
		const muted = stubBackend();
		const view = mount(muted, layers(A, B));
		await flush();
		expect(muted.calls.describe).toHaveLength(1);
		expect(frameOf(view.grant).runtime).toBeNull();
	});

	test("a frozen prompt keeps its request while newer addresses arrive", async () => {
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.setProps(layers(A, B));
		await clock.advance(250);
		expect(runtimeSourcesOf(backend.calls.describe.at(-1))).toEqual([A, B]);
		const prompt = view.grant.prompt;
		expect(prompt?.newerAvailable).toBe(true);
		expect(prompt?.runtime.pending.map((entry) => entry.source)).toEqual([A]);

		view.controller.actions.allowOnce();
		await flush();
		expect(runtimeSourcesOf(backend.calls.mint[0])).toEqual([A]);
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
		await clock.advance(250);
		expect(view.grant.runtimeRequest?.sources).toEqual([B]);
	});

	test("showing the newer request swaps the prompt content", async () => {
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.setProps(layers(A, B));
		await clock.advance(250);
		view.controller.actions.showNewerPrompt();
		expect(view.grant.prompt?.newerAvailable).toBe(false);
		expect(
			view.grant.prompt?.runtime.pending.map((entry) => entry.source),
		).toEqual([A, B]);
	});
});

describe("updates while the widget runs", () => {
	async function running(options: BackendOptions = {}, ...allowed: string[]) {
		grantRuntime(...allowed);
		const backend = stubBackend(options);
		const view = mount(backend, layers(...allowed));
		await flush();
		expect(view.grant.state.status).toBe("ready");
		return { backend, view };
	}

	test("props changes are debounced by 250 ms with a 1 s max wait", async () => {
		const { backend, view } = await running({}, A);
		const describes = () => backend.calls.describe.length;
		const before = describes();
		for (let step = 0; step < 6; step++) {
			view.setProps(layers(A, step % 2 === 0 ? B : C));
			await clock.advance(200);
		}
		expect(describes()).toBe(before + 1);
		view.setProps(layers(A, B));
		await clock.advance(249);
		expect(describes()).toBe(before + 1);
	});

	test("covered additions mint again and swap the frame with the union", async () => {
		grantRuntime(A, B);
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);

		view.setProps(layers(B));
		await clock.advance(250);
		expect(runtimeSourcesOf(backend.calls.describe.at(-1))).toEqual([A, B]);
		expect(runtimeSourcesOf(backend.calls.mint.at(-1))).toEqual([A, B]);
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A, B]);
		expect(view.grant.runtimeRequest).toBeNull();
	});

	test("after hello the frame is replaced at most every 5 s and the latest wins", async () => {
		grantRuntime(A, B, C);
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.controller.actions.onFrameHello();
		const first = frameOf(view.grant).grant;

		view.setProps(layers(A, B));
		await clock.advance(250);
		expect(backend.calls.mint).toHaveLength(2);
		expect(frameOf(view.grant).grant).toBe(first);

		view.setProps(layers(A, B, C));
		await clock.advance(250);
		expect(backend.calls.mint).toHaveLength(3);
		expect(frameOf(view.grant).grant).toBe(first);

		await clock.advance(4_500);
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A, B, C]);
		expect(frameOf(view.grant).grant).toBe("h.p3.s");
	});

	test("uncovered additions keep the frame and raise the banner; Review, Allow and Don't allow", async () => {
		const { backend, view } = await running({}, A);
		const frame = frameOf(view.grant);
		view.setProps(layers(A, B));
		await clock.advance(250);
		expect(frameOf(view.grant)).toEqual(frame);
		expect(view.grant.runtimeRequest).toEqual({
			sources: [B],
			count: 1,
			level: "external",
		});
		expect(view.grant.prompt).toBeNull();

		view.controller.actions.review();
		expect(view.grant.runtimeRequest).toBeNull();
		expect(view.grant.prompt?.mode).toBe("runtime");
		expect(view.grant.prompt?.hasRuntimeCheckbox).toBe(false);
		expect(
			view.grant.prompt?.runtime.pending.map((entry) => entry.source),
		).toEqual([B]);
		expect(
			view.grant.prompt?.runtime.allowed.map((entry) => entry.source),
		).toEqual([A]);

		view.controller.actions.dontAllow();
		await flush();
		expect(view.grant.prompt).toBeNull();
		expect(view.grant.runtimeRequest).toBeNull();
		expect(frameOf(view.grant)).toEqual(frame);
		expect([...readMicroWidgetRuntimeBlocks(TARGET)]).toEqual([B]);

		view.setProps(layers(A, B, C));
		await clock.advance(250);
		expect(runtimeSourcesOf(backend.calls.describe.at(-1))).toEqual([A, C]);
		view.controller.actions.review();
		view.controller.actions.allowOnce();
		await flush();
		expect(runtimeSourcesOf(backend.calls.mint.at(-1))).toEqual([A, C]);
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A, C]);
	});

	test("stop asking mutes runtime sources for the project", async () => {
		const { backend, view } = await running({}, A);
		view.setProps(layers(A, B));
		await clock.advance(250);
		view.controller.actions.review();
		view.controller.actions.stopAsking();
		await flush();
		const describes = backend.calls.describe.length;
		view.setProps(layers(A, B, C));
		await clock.advance(1_000);
		expect(backend.calls.describe).toHaveLength(describes);
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
	});

	test("a dismissed banner stays hidden until a new address or its backoff ends", async () => {
		const { view } = await running({}, A);
		view.setProps(layers(A, B));
		await clock.advance(250);
		view.controller.actions.dismissRuntimeRequest();
		expect(view.grant.runtimeRequest).toBeNull();
		await clock.advance(29_000);
		expect(view.grant.runtimeRequest).toBeNull();
		await clock.advance(1_000);
		expect(view.grant.runtimeRequest?.sources).toEqual([B]);

		view.controller.actions.dismissRuntimeRequest();
		await clock.advance(59_000);
		expect(view.grant.runtimeRequest).toBeNull();
		view.setProps(layers(A, B, C));
		await clock.advance(250);
		expect(view.grant.runtimeRequest?.sources).toEqual([B, C]);
	});

	describe("a dismissed banner and the clocks", () => {
		async function dismissed() {
			const { view } = await running({}, A);
			view.setProps(layers(A, B));
			await clock.advance(250);
			view.controller.actions.dismissRuntimeRequest();
			expect(view.grant.runtimeRequest).toBeNull();
			return view;
		}

		test("it returns when its backoff ends on a wall clock that lags the timers", async () => {
			// 40 ppm slow: a millisecond short of the 30 s the timer waited.
			clock.wall = (time) => WALL + Math.floor(time * (1 - 40e-6));
			const view = await dismissed();
			await clock.advance(30_000);
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);
		});

		test("it returns when its backoff ends on a wall clock stepped back meanwhile", async () => {
			let step = 0;
			clock.wall = (time) => WALL + time + step;
			const view = await dismissed();
			await clock.advance(10_000);
			step = -50;
			await clock.advance(20_000);
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);
		});

		test("it stays hidden for its backoff when the wall clock jumps ahead", async () => {
			let slept = 0;
			clock.wall = (time) => WALL + time + slept;
			const view = await dismissed();
			slept = 14 * HOUR;
			view.controller.actions.dismissNotice("runtime-skipped");
			expect(view.grant.runtimeRequest).toBeNull();

			await clock.advance(30_000);
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);
		});

		test("it returns with its timer, also when that fires a moment early", async () => {
			clock.early = 1;
			const view = await dismissed();
			await clock.advance(29_999);
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);
			expect(clock.pending).toBe(0);
		});
	});

	test("server rejections are skipped for the rest of the mount and reported", async () => {
		grantRuntime(A);
		const backend = stubBackend({ reject: { [B]: "reserved-host" } });
		const view = mount(backend, layers(A, B));
		await flush();
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
		expect(view.grant.notices).toEqual([
			{
				kind: "runtime-skipped",
				rejected: [{ slot: SLOT.path, source: B, code: "reserved-host" }],
				issues: [],
				invalidReason: null,
			},
		]);
		const describes = backend.calls.describe.length;
		view.setProps({ layers: [...layers(A, B).layers, { url: "x" }] });
		await clock.advance(250);
		expect(backend.calls.describe).toHaveLength(describes);
		view.controller.actions.dismissNotice("runtime-skipped");
		expect(view.grant.notices).toEqual([]);
	});
});

describe("fallbacks", () => {
	for (const legacyAllowed of [true, false]) {
		test(`a missing bundle shows its load error with legacy fallback ${legacyAllowed ? "enabled" : "disabled"}`, async () => {
			const backend = stubBackend();
			const detail = "Could not install com.example.maps@1.0.0: access denied";
			backend.registry.describeWidgetPolicy = async () => {
				throw new WidgetBundleUnavailableError(detail);
			};
			const view = mount(backend, {}, { legacyAllowed });
			await flush();
			expect(view.grant.state).toEqual({
				status: "error",
				reason: "bundle_unavailable",
				detail,
			});
			expect(view.grant.prompt).toBeNull();
			expect(backend.calls.mint).toEqual([]);
		});
	}

	test("a backend without widget policy support still permits a legacy baseline frame", async () => {
		const backend = stubBackend();
		backend.registry.describeWidgetPolicy = undefined;
		const view = mount(backend, {}, { legacyAllowed: true });
		await flush();
		expect(frameOf(view.grant)).toMatchObject({ kind: "legacy", policy: {} });
	});

	test("a second policy conflict with runtime sources runs declared-only", async () => {
		grantRuntime(A);
		const backend = stubBackend({
			mint: (request, call) => {
				if (request.runtimeSources?.length)
					throw new WidgetPolicyChangedError();
				return {
					grant: `h.p${call}.s`,
					expiresIn: 3600,
					policyDigest: request.policyDigest,
					runtime: null,
				};
			},
		});
		const view = mount(backend, layers(A));
		await flush();
		expect(backend.calls.describe).toHaveLength(3);
		expect(backend.calls.mint.map((call) => call.policyDigest)).toEqual([
			backend.calls.mint[0].policyDigest,
			backend.calls.mint[0].policyDigest,
			digest(DECLARED),
		]);
		expect(frameOf(view.grant)).toMatchObject({
			grant: "h.p3.s",
			runtime: null,
		});
		view.setProps(layers(A, B));
		await clock.advance(1_000);
		expect(backend.calls.describe).toHaveLength(3);
	});

	test("a policy conflict waits for the second describe instead of minting the stale digest again", async () => {
		grantRuntime(A);
		const fresh = `sha256:${"f".repeat(64)}`;
		const runtimeDigests: string[] = [];
		const backend = stubBackend({
			describeRuntime: (request) => {
				const response = describeResponse(request, {});
				const policyDigest =
					runtimeDigests.length === 0 ? response.policyDigest : fresh;
				runtimeDigests.push(policyDigest);
				return { ...response, policyDigest };
			},
			mint: (request) => {
				if (request.policyDigest === runtimeDigests[0])
					throw new WidgetPolicyChangedError();
				return undefined;
			},
		});
		const view = mount(backend, layers(A));
		await flush();
		expect(backend.calls.describe).toHaveLength(3);
		expect(backend.calls.mint.map((call) => call.policyDigest)).toEqual([
			runtimeDigests[0],
			fresh,
		]);
		expect(runtimeSourcesOf(backend.calls.mint[1])).toEqual([A]);
		expect(frameOf(view.grant).runtime).not.toBeNull();
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
	});

	test("an API without the describe POST runs declared-only with a notice", async () => {
		grantRuntime(A);
		const backend = stubBackend({
			describeRuntime: () => {
				throw Object.assign(new Error("Method Not Allowed"), { status: 405 });
			},
		});
		const view = mount(backend, layers(A));
		await flush();
		expect(frameOf(view.grant).runtime).toBeNull();
		expect(view.grant.notices).toEqual([{ kind: "runtime-unavailable" }]);
	});

	test("unavailable runtime sources retry after a doubling backoff", async () => {
		grantRuntime(A, B, C);
		const backend = stubBackend({ runtimeStatus: "unavailable" });
		const view = mount(backend, layers(A));
		await flush();
		expect(frameOf(view.grant).runtime).toBeNull();
		expect(view.grant.notices).toEqual([{ kind: "runtime-unavailable" }]);
		const describes = () => backend.calls.describe.length;
		expect(describes()).toBe(2);

		view.setProps(layers(A, B));
		await clock.advance(10_000);
		expect(describes()).toBe(2);
		await clock.advance(20_000);
		view.setProps(layers(A, C));
		await clock.advance(250);
		expect(describes()).toBe(3);
		await clock.advance(40_000);
		view.setProps(layers(A, B, C));
		await clock.advance(250);
		expect(describes()).toBe(3);
		await clock.advance(20_000);
		view.setProps(layers(B));
		await clock.advance(250);
		expect(describes()).toBe(4);
	});

	test("an invalid runtime set runs declared-only and reports why", async () => {
		grantRuntime();
		const backend = stubBackend({ runtimeStatus: "invalid" });
		const view = mount(backend, layers(A));
		await flush();
		expect(frameOf(view.grant).runtime).toBeNull();
		expect(view.grant.notices).toEqual([
			{
				kind: "runtime-skipped",
				rejected: [],
				issues: [],
				invalidReason: "too-many-sources",
			},
		]);
	});

	test("host issues are reported without values", async () => {
		grantRuntime();
		const backend = stubBackend();
		const view = mount(backend, {
			layers: [{ url: "https://tiles.example.com:8443/a.png" }],
		});
		await flush();
		await clock.advance(500);
		expect(view.grant.notices).toEqual([
			{
				kind: "runtime-skipped",
				rejected: [],
				issues: [{ slot: SLOT.path, code: "port", count: 1 }],
				invalidReason: null,
			},
		]);
	});

	test("the grant cache keeps the 256 most recent grants", async () => {
		for (let index = 0; index <= MICRO_WIDGET_GRANT_CACHE_LIMIT; index++) {
			const backend = stubBackend();
			const packageId = `com.example.p${index}`;
			grantMicroWidgetConsent(
				{ ...TARGET, packageId },
				{
					policy: DECLARED,
					levels: { "https://api.cesium.com": "external" },
					runtime: [],
				},
				"session",
			);
			const registry = {
				describeWidgetPolicy: async (request: WidgetPolicyRequest) =>
					parseWidgetPolicyDescriptor({
						...describeResponse(request, {}),
						packageId,
					}),
				mintWidgetGrant: backend.registry.mintWidgetGrant,
			} as unknown as IRegistryState;
			mount({ calls: backend.calls, registry }, {}, { packageId });
		}
		await flush();
		await clock.advance(500);
		expect(
			controllers.every((view) => view.getSnapshot().state.status === "ready"),
		).toBe(true);
		expect(microWidgetGrantCacheSizeForTests()).toBe(
			MICRO_WIDGET_GRANT_CACHE_LIMIT,
		);
	});
});

describe("grant deadlines", () => {
	/** The stub's grants live an hour and count as expired the refresh margin earlier. */
	const LIFETIME = 3_600_000 - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS;

	test("a grant expires on the wall clock, so time the device slept counts", async () => {
		grantRuntime(A);
		const backend = stubBackend();
		let wall = Date.UTC(2026, 9, 1, 17, 30);
		const wallClock = spyOn(Date, "now").mockImplementation(() => wall);
		const monotonic = spyOn(performance, "now").mockImplementation(() => 5_000);
		restorers.push(() => {
			wallClock.mockRestore();
			monotonic.mockRestore();
		});
		const mountOnTheDefaultClock = async () => {
			const controller = new MicroWidgetGrantController();
			controllers.push(controller);
			controller.setInputs(inputsFor(backend.registry, layers(A)));
			controller.activate();
			await flush();
			return controller;
		};

		const mounted = await mountOnTheDefaultClock();
		expect(frameOf(mounted.getSnapshot()).grant).toBe("h.p1.s");
		wall += LIFETIME - 1;
		mounted.actions.onFrameLoad();
		await flush();
		expect(backend.calls.mint).toHaveLength(1);

		wall += 1;
		mounted.actions.onFrameLoad();
		await flush();
		expect(backend.calls.mint).toHaveLength(2);
		expect(frameOf(mounted.getSnapshot()).grant).toBe("h.p2.s");

		wall += LIFETIME;
		const later = await mountOnTheDefaultClock();
		expect(backend.calls.mint).toHaveLength(3);
		expect(frameOf(later.getSnapshot()).grant).toBe("h.p3.s");

		// The monotonic clock stood still, but the grant from before the sleep is past its deadline.
		mounted.actions.onFrameLoad();
		await flush();
		expect(backend.calls.mint).toHaveLength(4);
		expect(frameOf(mounted.getSnapshot()).grant).toBe("h.p4.s");
	});

	async function running(backend = stubBackend()) {
		grantRuntime(A);
		const view = mount(backend, layers(A));
		await flush();
		expect(frameOf(view.grant).grant).toBe("h.p1.s");
		return { backend, view };
	}

	/** The second mint, the one that renews the first grant, fails. */
	const failingRenewal = (failure: unknown) =>
		stubBackend({
			mint: (_request, call) => {
				if (call === 2) throw failure;
				return undefined;
			},
		});
	const offline = () => new TypeError("Failed to fetch");
	const unavailable = () =>
		Object.assign(new Error("Service Unavailable"), { status: 503 });

	test("a running frame's grant is not renewed within its deadline", async () => {
		const { backend, view } = await running();
		await clock.advance(LIFETIME - 1);
		expect(view.controller.actions.renewGrant()).toBe(false);
		await flush();
		expect(backend.calls.mint).toHaveLength(1);
	});

	test("past the deadline it is minted again once, and the frame runs on until the new grant arrived", async () => {
		const { backend, view } = await running();
		await clock.advance(LIFETIME);
		expect(view.controller.actions.renewGrant()).toBe(true);
		expect(view.controller.actions.renewGrant()).toBe(true);
		expect(frameOf(view.grant).grant).toBe("h.p1.s");

		await flush();
		expect(backend.calls.mint).toHaveLength(2);
		expect(frameOf(view.grant).grant).toBe("h.p2.s");
		expect(view.controller.actions.renewGrant()).toBe(false);
	});

	test("a renewal that fails leaves the running frame, and the next one mints again", async () => {
		for (const failure of [offline(), unavailable()]) {
			resetMicroWidgetConsentForTests();
			resetMicroWidgetGrantCacheForTests();
			const { backend, view } = await running(failingRenewal(failure));
			const frame = frameOf(view.grant);
			await clock.advance(LIFETIME);
			expect(view.controller.actions.renewGrant()).toBe(true);
			await flush();
			expect(backend.calls.mint).toHaveLength(2);
			expect(frameOf(view.grant)).toEqual(frame);

			expect(view.controller.actions.renewGrant()).toBe(true);
			await flush();
			expect(backend.calls.mint).toHaveLength(3);
			expect(frameOf(view.grant).grant).toBe("h.p3.s");
		}
	});

	test("a frame without a grant has nothing to renew", async () => {
		grantRuntime();
		const backend = stubBackend({
			mint: (request) => ({
				grant: null,
				expiresIn: 60,
				policyDigest: request.policyDigest,
			}),
		});
		const view = mount(backend, {});
		await flush();
		await clock.advance(500);
		expect(frameOf(view.grant).grant).toBeNull();

		expect(view.controller.actions.renewGrant()).toBe(false);
		await flush();
		expect(backend.calls.mint).toHaveLength(1);
	});

	describe("while new addresses wait for review", () => {
		async function reviewPending(backend = stubBackend()) {
			const { view } = await running(backend);
			view.setProps(layers(A, B));
			await clock.advance(250);
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);
			await clock.advance(LIFETIME);
			return { backend, view };
		}

		for (const [trigger, fire] of [
			["a frame load", (grant: MicroWidgetGrant) => grant.onFrameLoad()],
			["a renewal", (grant: MicroWidgetGrant) => grant.renewGrant()],
		] as const) {
			test(`${trigger} past the deadline mints the running frame's grant again, and the banner stays`, async () => {
				const { backend, view } = await reviewPending();
				fire(view.grant);
				await flush();
				expect(backend.calls.mint).toHaveLength(2);
				expect(runtimeSourcesOf(backend.calls.mint[1])).toEqual([A]);
				expect(frameOf(view.grant).grant).toBe("h.p2.s");
				expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
				expect(view.grant.runtimeRequest?.sources).toEqual([B]);

				view.controller.actions.review();
				await clock.advance(LIFETIME);
				fire(view.grant);
				await flush();
				expect(frameOf(view.grant).grant).toBe("h.p3.s");
				expect(view.grant.prompt?.mode).toBe("runtime");
			});
		}

		test("a renewal that fails leaves the running frame and the banner", async () => {
			const { backend, view } = await reviewPending(failingRenewal(offline()));
			expect(view.controller.actions.renewGrant()).toBe(true);
			await flush();
			expect(backend.calls.mint).toHaveLength(2);
			expect(frameOf(view.grant).grant).toBe("h.p1.s");
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);

			view.controller.actions.review();
			view.controller.actions.dontAllow();
			await flush();
			expect(frameOf(view.grant).grant).toBe("h.p1.s");
		});

		test("a renewal that the backend answers with a changed policy brings the request for review back", async () => {
			const { backend, view } = await reviewPending(
				stubBackend({
					mint: (_request, call) => {
						if (call === 2) throw new WidgetPolicyChangedError();
						return undefined;
					},
				}),
			);
			expect(view.controller.actions.renewGrant()).toBe(true);
			await flush();
			expect(backend.calls.mint).toHaveLength(3);
			expect(frameOf(view.grant).grant).toBe("h.p3.s");
			expect(view.grant.runtimeRequest).toBeNull();

			await clock.advance(250);
			expect(view.grant.runtimeRequest?.sources).toEqual([B]);
			expect(frameOf(view.grant).grant).toBe("h.p3.s");
		});

		test("a newer frame waiting for its swap is not replaced by the running frame's new grant", async () => {
			let slept = 0;
			clock.wall = (time) => WALL + time + slept;
			grantRuntime(A, B);
			const backend = stubBackend();
			const view = mount(backend, layers(A));
			await flush();
			view.controller.actions.onFrameHello();
			view.setProps(layers(A, B));
			await clock.advance(250);
			view.setProps(layers(A, B, C));
			await clock.advance(250);
			expect(frameOf(view.grant).grant).toBe("h.p1.s");
			expect(view.grant.runtimeRequest?.sources).toEqual([C]);

			slept = HOUR;
			view.controller.actions.onFrameLoad();
			await flush();
			await clock.advance(4_500);
			expect(backend.calls.mint).toHaveLength(2);
			expect(frameOf(view.grant).grant).toBe("h.p2.s");
			expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A, B]);
		});
	});

	describe("a frame that loaded past the deadline", () => {
		for (const pending of [false, true]) {
			const when = pending ? " while new addresses wait for review" : "";
			const loaded = async (failure: unknown) => {
				const { view } = await running(failingRenewal(failure));
				if (pending) {
					view.setProps(layers(A, B));
					await clock.advance(250);
				}
				await clock.advance(LIFETIME);
				view.controller.actions.onFrameLoad();
				await flush();
				return view;
			};

			test(`shows why its re-mint failed${when}`, async () => {
				const view = await loaded(offline());
				expect(view.grant.state).toEqual({
					status: "error",
					reason: "mint_failed",
					detail: "Failed to fetch",
				});
				expect(view.grant.runtimeRequest).toBeNull();
				expect(view.controller.actions.renewGrant()).toBe(false);
			});

			test(`runs at baseline with a notice when grants became unavailable${when}`, async () => {
				const view = await loaded(unavailable());
				expect(frameOf(view.grant)).toMatchObject({
					grant: null,
					notice: "unavailable",
				});
				expect(view.grant.runtimeRequest).toBeNull();
				expect(view.controller.actions.renewGrant()).toBe(false);
			});
		}
	});
});

describe("waits are measured on the monotonic clock", () => {
	let slept: number;

	beforeEach(() => {
		slept = 0;
		clock.wall = (time) => WALL + time + slept;
	});

	test("props changes are extracted after 1 s at the latest when the wall clock is stepped back", async () => {
		grantRuntime(A);
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		const before = backend.calls.describe.length;
		view.setProps(layers(A, B));
		await clock.advance(200);
		slept -= HOUR;
		for (let step = 0; step < 5; step++) {
			view.setProps(layers(A, step % 2 === 0 ? C : B));
			await clock.advance(200);
		}
		expect(backend.calls.describe).toHaveLength(before + 1);
	});

	test("unavailable runtime sources wait out their backoff wherever the wall clock moves", async () => {
		const failing = stubBackend({
			describeRuntime: () => {
				throw Object.assign(new Error("Bad Gateway"), { status: 502 });
			},
		});
		for (const backend of [
			stubBackend({ runtimeStatus: "unavailable" }),
			failing,
		]) {
			resetMicroWidgetConsentForTests();
			grantRuntime(A, B);
			const view = mount(backend, layers(A));
			await flush();
			expect(backend.calls.describe).toHaveLength(2);

			slept += 14 * HOUR;
			view.setProps(layers(A, B));
			await clock.advance(250);
			expect(backend.calls.describe).toHaveLength(2);

			slept -= 15 * HOUR;
			await clock.advance(30_000);
			view.setProps(layers(B));
			await clock.advance(250);
			expect(backend.calls.describe).toHaveLength(3);
		}
	});

	test("after hello the frame is replaced at most every 5 s when the wall clock jumps ahead", async () => {
		grantRuntime(A, B, C);
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.controller.actions.onFrameHello();
		const first = frameOf(view.grant).grant;

		slept += 14 * HOUR;
		view.setProps(layers(A, B));
		await clock.advance(250);
		expect(backend.calls.mint).toHaveLength(2);
		expect(frameOf(view.grant).grant).toBe(first);
		await clock.advance(4_750);
		expect(frameOf(view.grant).grant).toBe("h.p2.s");

		view.controller.actions.onFrameHello();
		view.setProps(layers(A, B, C));
		await clock.advance(250);
		expect(frameOf(view.grant).grant).toBe("h.p2.s");
		await clock.advance(4_750);
		expect(frameOf(view.grant).grant).toBe("h.p3.s");
	});

	test("a grant from before a sleep is re-minted on the next frame load, however short the device was awake", async () => {
		grantRuntime(A);
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		const load = async () => {
			view.controller.actions.onFrameLoad();
			await flush();
		};

		slept += HOUR;
		await load();
		expect(backend.calls.mint).toHaveLength(2);

		slept += HOUR;
		await load();
		expect(backend.calls.mint).toHaveLength(3);
	});

	test("a grant that is past its deadline on arrival is re-minted on frame load once a minute, also when the wall clock runs slow", async () => {
		clock.wall = (time) => WALL + Math.floor(time / 2);
		grantRuntime(A);
		const backend = stubBackend({
			mint: (request, call) => ({
				...mintAnswer(request, call),
				expiresIn: MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS / 1000,
			}),
		});
		const view = mount(backend, layers(A));
		await flush();
		const load = async () => {
			view.controller.actions.onFrameLoad();
			await flush();
		};

		await load();
		expect(backend.calls.mint).toHaveLength(2);
		await clock.advance(59_999);
		await load();
		expect(backend.calls.mint).toHaveLength(2);

		await clock.advance(1);
		await load();
		expect(backend.calls.mint).toHaveLength(3);
	});
});

describe("what a decision applies to", () => {
	function storedRecord(items: Map<string, string>) {
		return JSON.parse(items.get(microWidgetConsentKey(TARGET)) ?? "null");
	}

	test("Always allow on a prompt that hid a declared part allowed this time persists only the addresses it listed", async () => {
		const items = installLocalStorage();
		grantRuntime(A);
		const backend = stubBackend();
		const view = mount(backend, layers(A, B));
		await flush();
		const prompt = view.grant.prompt;
		expect(prompt?.declared.covered).toBe(true);
		expect(prompt?.runtime.pending.map((entry) => entry.source)).toEqual([B]);
		expect(prompt?.runtime.allowed.map((entry) => entry.source)).toEqual([A]);

		view.controller.actions.allowForProject(prompt?.key);
		await flush();
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A, B]);
		const stored = storedRecord(items);
		expect(stored.policy).toEqual({});
		expect(stored.levels).toEqual({});
		expect(stored.runtime.map((entry: { s: string }) => entry.s)).toEqual([B]);
	});

	test("Always allow on a prompt that showed the declared part persists it with the listed addresses", async () => {
		const items = installLocalStorage();
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		view.controller.actions.allowForProject(view.grant.prompt?.key);
		await flush();
		const stored = storedRecord(items);
		expect(stored.policy).toEqual(DECLARED);
		expect(stored.levels).toEqual({ "https://api.cesium.com": "external" });
		expect(stored.runtime.map((entry: { s: string }) => entry.s)).toEqual([A]);
	});

	test("a decision for a prompt that no longer shows is ignored", async () => {
		grantRuntime();
		const backend = stubBackend();
		const view = mount(backend, layers(A));
		await flush();
		const shown = view.grant.prompt?.key;
		expect(view.grant.prompt?.declared.covered).toBe(true);

		revokeMicroWidgetConsent(TARGET);
		expect(view.grant.prompt?.declared.covered).toBe(false);
		expect(view.grant.prompt?.key).not.toBe(shown);
		view.controller.actions.allowOnce(shown);
		view.controller.actions.dontAllow(shown);
		await flush();
		expect(view.grant.state.status).toBe("pending");
		expect(backend.calls.mint).toEqual([]);
		expect(readMicroWidgetRuntimeBlocks(TARGET).size).toBe(0);

		view.controller.actions.allowOnce(view.grant.prompt?.key);
		await flush();
		expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
	});
});

describe("instances sharing one dialog", () => {
	async function pair(backend: ReturnType<typeof stubBackend>) {
		const first = mount(backend, layers(A));
		const second = mount(backend, layers(A));
		await flush();
		expect(second.grant.prompt?.key).toBe(first.grant.prompt?.key);
		return [first, second] as const;
	}

	function expectDeclaredOnly(views: readonly { grant: MicroWidgetGrant }[]) {
		for (const view of views) {
			expect(view.grant.prompt).toBeNull();
			expect(frameOf(view.grant).runtime).toBeNull();
			expect(frameOf(view.grant).policy).toEqual(DECLARED);
		}
	}

	test("Don't allow on runtime addresses stops every instance asking for them", async () => {
		grantRuntime();
		const backend = stubBackend();
		const views = await pair(backend);
		views[0].controller.actions.dontAllow(views[0].grant.prompt?.key);
		await flush();
		expectDeclaredOnly(views);
		expect(
			backend.calls.mint.every((call) => call.runtimeSources === undefined),
		).toBe(true);
	});

	test("Stop asking mutes every instance", async () => {
		grantRuntime();
		const backend = stubBackend();
		const views = await pair(backend);
		views[0].controller.actions.stopAsking(views[0].grant.prompt?.key);
		await flush();
		expectDeclaredOnly(views);
		const describes = backend.calls.describe.length;
		views[1].setProps(layers(A, B));
		await clock.advance(1_000);
		expect(backend.calls.describe).toHaveLength(describes);
	});

	test("an unchecked runtime box answers for every instance", async () => {
		const backend = stubBackend();
		const views = await pair(backend);
		views[0].controller.actions.setIncludeRuntime(false);
		views[0].controller.actions.allowOnce(views[0].grant.prompt?.key);
		await flush();
		expectDeclaredOnly(views);
		expect(
			backend.calls.mint.every((call) => call.runtimeSources === undefined),
		).toBe(true);
	});

	test("refusing new addresses while both run clears the other instance's banner", async () => {
		grantRuntime(A);
		const backend = stubBackend();
		const views = await pair(backend);
		for (const view of views) view.setProps(layers(A, B));
		await clock.advance(250);
		expect(views[1].grant.runtimeRequest?.sources).toEqual([B]);

		views[0].controller.actions.review();
		views[0].controller.actions.dontAllow(views[0].grant.prompt?.key);
		await flush();
		for (const view of views) {
			expect(view.grant.runtimeRequest).toBeNull();
			expect(view.grant.prompt).toBeNull();
			expect(frameOf(view.grant).policy.csp?.imgSrc).toEqual([A]);
		}
	});
});

describe("sandbox access", () => {
	const target = { packageId: PACKAGE, packageVersion: "1.0.0", appId: APP };
	const TOKEN = "h.access.s";

	function accessBackend(
		answer: (
			request: WidgetAccessRequest,
			call: number,
		) => Promise<{ access: string | null; expiresIn: number }> = async () => ({
			access: TOKEN,
			expiresIn: 3600,
		}),
	) {
		const calls: WidgetAccessRequest[] = [];
		const registry = {
			getWidgetAccess: (request: WidgetAccessRequest) => {
				calls.push(structuredClone(request));
				return answer(request, calls.length);
			},
		} as unknown as IRegistryState;
		return { calls, registry };
	}

	test("a backend without access tokens loads every sandbox anonymously", async () => {
		const backend = stubBackend();
		const anonymous = { access: null, deadline: Number.POSITIVE_INFINITY };
		expect(
			await loadMicroWidgetAccess(backend.registry, target, clock),
		).toEqual(anonymous);
		expect(await loadMicroWidgetAccess(null, target, clock)).toEqual(anonymous);
	});

	test("one request per package version and project; new frames reuse a token while half its lifetime is left", async () => {
		const backend = accessBackend();
		const access = {
			access: TOKEN,
			deadline: 1_000 + HOUR - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS,
		};
		const [first, second] = await Promise.all([
			loadMicroWidgetAccess(backend.registry, target, clock),
			loadMicroWidgetAccess(backend.registry, target, clock),
		]);
		expect([first, second]).toEqual([access, access]);
		expect(backend.calls).toEqual([
			{ packageId: PACKAGE, packageVersion: "1.0.0", appId: APP },
		]);

		await clock.advance(HOUR / 2);
		expect(
			await loadMicroWidgetAccess(backend.registry, target, clock),
		).toEqual(access);
		expect(backend.calls).toHaveLength(1);

		await clock.advance(1);
		await loadMicroWidgetAccess(backend.registry, target, clock);
		expect(backend.calls).toHaveLength(2);

		await loadMicroWidgetAccess(
			backend.registry,
			{ ...target, packageVersion: "1.1.0" },
			clock,
		);
		await loadMicroWidgetAccess(
			backend.registry,
			{ ...target, appId: null },
			clock,
		);
		expect(backend.calls.slice(2)).toEqual([
			{ packageId: PACKAGE, packageVersion: "1.1.0", appId: APP },
			{ packageId: PACKAGE, packageVersion: "1.0.0" },
		]);
	});

	test("a refusal rejects and is asked again on the next frame", async () => {
		const refusal = new Error("403 Forbidden");
		const backend = accessBackend(async (_request, call) => {
			if (call === 1) throw refusal;
			return { access: null, expiresIn: 3600 };
		});
		await expect(
			loadMicroWidgetAccess(backend.registry, target, clock),
		).rejects.toBe(refusal);
		expect(
			(await loadMicroWidgetAccess(backend.registry, target, clock)).access,
		).toBeNull();
		expect(backend.calls).toHaveLength(2);
	});

	test("anonymous access is reused while it has the refresh margin left", async () => {
		const backend = accessBackend(async () => ({
			access: null,
			expiresIn: 3600,
		}));
		await loadMicroWidgetAccess(backend.registry, target, clock);
		await clock.advance(HOUR - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS);
		expect(
			await loadMicroWidgetAccess(backend.registry, target, clock),
		).toEqual({
			access: null,
			deadline: 1_000 + HOUR - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS,
		});
		expect(backend.calls).toHaveLength(1);

		await clock.advance(1);
		await loadMicroWidgetAccess(backend.registry, target, clock);
		expect(backend.calls).toHaveLength(2);
	});

	test("a short-lived token is never reused past its deadline", async () => {
		const backend = accessBackend(async () => ({
			access: TOKEN,
			expiresIn: 400,
		}));
		const { deadline } = await loadMicroWidgetAccess(
			backend.registry,
			target,
			clock,
		);
		expect(deadline).toBe(
			1_000 + 400_000 - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS,
		);
		await clock.advance(deadline - clock.time);
		await loadMicroWidgetAccess(backend.registry, target, clock);
		expect(backend.calls).toHaveLength(1);

		await clock.advance(1);
		await loadMicroWidgetAccess(backend.registry, target, clock);
		expect(backend.calls).toHaveLength(2);
	});

	test("deadlines run on the wall clock, so time the device slept counts", async () => {
		const backend = accessBackend();
		let wall = Date.UTC(2026, 9, 1, 9);
		const wallClock = spyOn(Date, "now").mockImplementation(() => wall);
		const monotonic = spyOn(performance, "now").mockImplementation(() => 5_000);
		restorers.push(() => {
			wallClock.mockRestore();
			monotonic.mockRestore();
		});
		expect(await loadMicroWidgetAccess(backend.registry, target)).toEqual({
			access: TOKEN,
			deadline: wall + HOUR - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS,
		});

		wall += HOUR / 2 + 1;
		await loadMicroWidgetAccess(backend.registry, target);
		expect(backend.calls).toHaveLength(2);
	});

	test("forgetting a token drops only the entry that still holds it", async () => {
		const backend = accessBackend(async (_request, call) => ({
			access: `h.access${call}.s`,
			expiresIn: 3600,
		}));
		const load = async () =>
			(await loadMicroWidgetAccess(backend.registry, target, clock)).access;
		expect(await load()).toBe("h.access1.s");
		forgetMicroWidgetAccess(target, "h.access2.s");
		forgetMicroWidgetAccess({ ...target, appId: null }, "h.access1.s");
		expect(await load()).toBe("h.access1.s");

		forgetMicroWidgetAccess(target, "h.access1.s");
		expect(await load()).toBe("h.access2.s");
		expect(backend.calls).toHaveLength(2);
	});

	test("forgetting the grant cache forgets access tokens too", async () => {
		const backend = accessBackend();
		await loadMicroWidgetAccess(backend.registry, target, clock);
		resetMicroWidgetGrantCacheForTests();
		await loadMicroWidgetAccess(backend.registry, target, clock);
		expect(backend.calls).toHaveLength(2);
	});

	describe("a request that failed", () => {
		const failure = (status: number, code?: string) =>
			Object.assign(new Error(`HTTP ${status}`), { status, code });

		function ask(backend: ReturnType<typeof accessBackend>) {
			const granted: MicroWidgetAccess[] = [];
			const failed: unknown[] = [];
			const cancel = requestMicroWidgetAccess(
				backend.registry,
				target,
				clock,
				(access) => granted.push(access),
				(error) => failed.push(error),
			);
			return { granted, failed, cancel };
		}

		test("is asked again after growing delays and stands once they are used up", async () => {
			const unavailable = failure(503);
			const backend = accessBackend(async () => {
				throw unavailable;
			});
			const { granted, failed } = ask(backend);
			await flush();

			expect(MICRO_WIDGET_ACCESS_RETRY_DELAYS_MS).toEqual([
				2_000, 5_000, 15_000,
			]);
			for (const [
				retry,
				delay,
			] of MICRO_WIDGET_ACCESS_RETRY_DELAYS_MS.entries()) {
				await clock.advance(delay - 1);
				expect(backend.calls).toHaveLength(retry + 1);
				expect(failed).toEqual([]);
				await clock.advance(1);
				expect(backend.calls).toHaveLength(retry + 2);
			}
			expect(failed).toHaveLength(1);
			expect(failed[0]).toBe(unavailable);
			expect(granted).toEqual([]);
			expect(clock.pending).toBe(0);
		});

		test("settles with the answer of a later attempt", async () => {
			const backend = accessBackend(async (_request, call) => {
				if (call === 1) throw new TypeError("Failed to fetch");
				if (call === 2) throw failure(429, "RATE_LIMITED");
				return { access: TOKEN, expiresIn: 3600 };
			});
			const { granted, failed } = ask(backend);
			await flush();
			await clock.advance(2_000);
			expect(backend.calls).toHaveLength(2);
			expect(granted).toEqual([]);

			await clock.advance(5_000);
			expect(granted).toEqual([
				{
					access: TOKEN,
					deadline: 8_000 + HOUR - MICRO_WIDGET_GRANT_REFRESH_MARGIN_MS,
				},
			]);
			expect(failed).toEqual([]);
			expect(clock.pending).toBe(0);
		});

		test("stands at once when the API refused the viewer", async () => {
			for (const refusal of [
				failure(403, "FORBIDDEN"),
				failure(404, "NOT_FOUND"),
			]) {
				const backend = accessBackend(async () => {
					throw refusal;
				});
				const { granted, failed } = ask(backend);
				await flush();
				expect(failed).toHaveLength(1);
				expect(failed[0]).toBe(refusal);
				expect(clock.pending).toBe(0);

				await clock.advance(60_000);
				expect(backend.calls).toHaveLength(1);
				expect(granted).toEqual([]);
			}
		});

		test("is asked again whenever the failure is not a refusal", async () => {
			const timeout = Object.assign(new Error("Request timed out"), {
				name: "RequestTimeoutError",
			});
			for (const error of [
				failure(401, "UNAUTHORIZED"),
				failure(404),
				failure(408),
				failure(500, "ERROR"),
				new TypeError("Load failed"),
				timeout,
				"boom",
			]) {
				resetMicroWidgetGrantCacheForTests();
				const backend = accessBackend(async (_request, call) => {
					if (call === 1) throw error;
					return { access: null, expiresIn: 3600 };
				});
				const { granted, failed } = ask(backend);
				await flush();
				expect(granted).toEqual([]);
				expect(failed).toEqual([]);

				await clock.advance(MICRO_WIDGET_ACCESS_RETRY_DELAYS_MS[0]);
				expect(granted).toHaveLength(1);
				expect(failed).toEqual([]);
			}
		});

		test("frames waiting for one package version share each attempt", async () => {
			const backend = accessBackend(async (_request, call) => {
				if (call === 1) throw failure(502);
				return { access: TOKEN, expiresIn: 3600 };
			});
			const frames = [ask(backend), ask(backend), ask(backend)];
			await flush();
			expect(backend.calls).toHaveLength(1);
			await clock.advance(2_000);
			expect(backend.calls).toHaveLength(2);
			for (const frame of frames) {
				expect(frame.granted.map(({ access }) => access)).toEqual([TOKEN]);
			}
		});

		test("cancelled, it neither settles nor leaves a timer", async () => {
			let answer: (response: {
				access: string | null;
				expiresIn: number;
			}) => void = () => {};
			const slow = ask(
				accessBackend(
					() =>
						new Promise((resolve) => {
							answer = resolve;
						}),
				),
			);
			await flush();
			slow.cancel();
			answer({ access: TOKEN, expiresIn: 3600 });
			await flush();
			expect(slow.granted).toEqual([]);

			resetMicroWidgetGrantCacheForTests();
			const failing = accessBackend(async () => {
				throw failure(503);
			});
			const waiting = ask(failing);
			await flush();
			expect(clock.pending).toBe(1);
			waiting.cancel();
			expect(clock.pending).toBe(0);
			await clock.advance(60_000);
			expect(failing.calls).toHaveLength(1);
			expect(waiting.failed).toEqual([]);

			let fail: (error: unknown) => void = () => {};
			const doomed = ask(
				accessBackend(
					() =>
						new Promise((_resolve, reject) => {
							fail = reject;
						}),
				),
			);
			await flush();
			doomed.cancel();
			fail(failure(503));
			await flush();
			expect(clock.pending).toBe(0);
			expect(doomed.failed).toEqual([]);
		});
	});
});
