import { afterEach, beforeEach, describe, expect, test } from "bun:test";
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
	MICRO_WIDGET_GRANT_CACHE_LIMIT,
	type MicroWidgetFrameMount,
	type MicroWidgetGrant,
	type MicroWidgetGrantClock,
	MicroWidgetGrantController,
	type MicroWidgetGrantControllerInputs,
	microWidgetGrantCacheSizeForTests,
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

function digest(value: unknown): string {
	return `sha256:${createHash("sha256").update(JSON.stringify(value)).digest("hex")}`;
}

class FakeClock implements MicroWidgetGrantClock {
	time = 1_000;
	private nextId = 1;
	private timers = new Map<number, { at: number; callback: () => void }>();

	now = () => this.time;

	setTimeout = (callback: () => void, ms: number) => {
		const id = this.nextId++;
		this.timers.set(id, { at: this.time + ms, callback });
		return id;
	};

	clearTimeout = (handle: unknown) => {
		this.timers.delete(handle as number);
	};

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
			const answer = options.mint?.(request, calls.mint.length) ?? {
				grant: `h.p${calls.mint.length}.s`,
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
			return parseWidgetGrantResponse(answer, isWebWidgetGrant);
		},
	} as unknown as IRegistryState;
	return { calls, registry };
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
