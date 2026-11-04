import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import { act, createElement } from "react";
import type { Root } from "react-dom/client";
import {
	domWindow,
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
const rendered: { root: Root; client: QueryClient; container: HTMLElement }[] =
	[];

beforeAll(() => {
	installDomGlobals();
	mock.module("@flow-like/locales", () => ({
		...actualLocales,
		useTranslation: () => ({ t: translate, i18n: fakeI18n }),
	}));
});

afterEach(async () => {
	const mounted = rendered.splice(0);
	await act(async () => {
		for (const { root } of mounted) root.unmount();
	});
	for (const { client, container } of mounted) {
		client.clear();
		container.remove();
	}
	await settle();
});

afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

const names: Record<string, string> = { me: "Me", anna: "Anna", paul: "Paul" };
const directory = (id: string) => ({
	id,
	name: names[id] ?? id,
	isMe: id === "me",
	isBot: false,
	isPending: false,
});

async function render(node: (create: typeof createElement) => unknown) {
	const { createRoot } = await import("react-dom/client");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const client = new QueryClient({
		defaultOptions: { queries: { enabled: false, retry: false } },
	});
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	rendered.push({ root, client, container });
	await act(async () => {
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				node(createElement) as never,
			),
		);
	});
	await settle();
	return container;
}

function callState(overrides: Record<string, unknown> = {}) {
	const calls = { join: 0, leave: 0, mute: 0 };
	return {
		calls,
		state: {
			live: {
				callId: "call_1",
				startedAt: Date.now() - 65_000,
				participants: [
					{ sub: "anna", muted: false, speaking: true },
					{ sub: "paul", muted: true, speaking: false },
				],
			},
			joined: false,
			join: async () => {
				calls.join++;
			},
			leave: async () => {
				calls.leave++;
			},
			toggleMute: () => {
				calls.mute++;
			},
			...overrides,
		},
	};
}

const label = (el: Element | null | undefined) =>
	el?.getAttribute("aria-label");

describe("ConversationCallBar", () => {
	test("renders nothing without a live call", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state } = callState({ live: null });
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		expect(
			container.querySelector("[data-slot=conversation-call-bar]"),
		).toBeNull();
	});

	test("offers Join, with a speaking ring and the clock, while you are out", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state, calls } = callState();
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const people = [...container.querySelectorAll("li")];
		expect(people.map((p) => label(p))).toEqual([
			"Anna is speaking",
			"Paul is muted",
		]);
		expect(people[0].className).toContain("ring-primary");
		expect(people[1].className).not.toContain("ring-primary");
		expect(
			container.querySelector("[data-slot=call-duration]")?.textContent,
		).toBe("1:05");
		const join = [...container.querySelectorAll("button")].find(
			(b) => b.textContent === "Join",
		);
		expect(join).toBeDefined();
		await act(async () => {
			join?.click();
		});
		expect(calls.join).toBe(1);
		expect(container.querySelector("[aria-label='Leave call']")).toBeNull();
	});

	test("offers mute and leave once you are in", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state, calls } = callState({
			joined: true,
			live: {
				callId: "call_1",
				startedAt: null,
				participants: [{ sub: "me", muted: true, speaking: false }],
			},
		});
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const mute = container.querySelector<HTMLButtonElement>(
			"[aria-label='Unmute microphone']",
		);
		expect(mute?.getAttribute("aria-pressed")).toBe("true");
		await act(async () => {
			mute?.click();
			container
				.querySelector<HTMLButtonElement>("[aria-label='Leave call']")
				?.click();
		});
		expect(calls.mute).toBe(1);
		expect(calls.leave).toBe(1);
	});
});

describe("CallLine", () => {
	const call = (extra = {}) => ({
		type: "call" as const,
		id: "call_1",
		senderId: "anna",
		createdAt: "2026-11-01T09:00:00.000Z",
		time: 0,
		...extra,
	});

	test("shows Join while that call is live and you are not in it", async () => {
		const { CallLine } = await import("./conversation-call");
		let joined = 0;
		const container = await render((c) =>
			c(CallLine, {
				call: call(),
				directory,
				thread: { liveCallId: "call_1", joined: false, onJoin: () => joined++ },
			}),
		);
		expect(container.textContent).toContain("Anna started a call");
		const join = container.querySelector("button");
		expect(join?.textContent).toBe("Join");
		await act(async () => {
			join?.click();
		});
		expect(joined).toBe(1);
	});

	test("has no Join for another call or once you are in", async () => {
		const { CallLine } = await import("./conversation-call");
		for (const thread of [
			{ liveCallId: "call_2", joined: false, onJoin: () => {} },
			{ liveCallId: "call_1", joined: true, onJoin: () => {} },
			undefined,
		]) {
			const container = await render((c) =>
				c(CallLine, { call: call({ senderId: "me" }), directory, thread }),
			);
			expect(container.textContent).toContain("You started a call");
			expect(container.querySelector("button")).toBeNull();
		}
	});

	test("says how long an ended call lasted", async () => {
		const { CallLine } = await import("./conversation-call");
		const texts = [];
		for (const durationMs of [12 * 60_000, 30_000, 75 * 60_000]) {
			const container = await render((c) =>
				c(CallLine, {
					call: call({ durationMs }),
					directory,
					thread: { liveCallId: "call_1", joined: false, onJoin: () => {} },
				}),
			);
			texts.push(container.textContent);
			expect(container.querySelector("button")).toBeNull();
		}
		expect(texts).toEqual([
			"Call ended · 12 min",
			"Call ended · under 1 min",
			"Call ended · 1 h 15 min",
		]);
	});
});

describe("ConversationCallBar listening", () => {
	const joinedState = (overrides: Record<string, unknown> = {}) => {
		const changes: boolean[] = [];
		const state = callState({
			joined: true,
			live: {
				callId: "call_1",
				startedAt: null,
				participants: [
					{ sub: "me", muted: false, speaking: false, listening: false },
					{ sub: "anna", muted: false, speaking: false, listening: true },
				],
			},
			botsPresent: true,
			canListen: true,
			listening: false,
			setListening: (on: boolean) => changes.push(on),
			...overrides,
		}).state;
		return { state, changes };
	};
	const toggle = (container: Element) =>
		container.querySelector<HTMLButtonElement>("[role=switch]");

	test("has no toggle when the conversation has no bots", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state } = joinedState({ botsPresent: false });
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		expect(toggle(container)).toBeNull();
	});

	test("is off by default and turns listening on when clicked", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state, changes } = joinedState();
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const control = toggle(container);
		expect(control?.getAttribute("aria-checked")).toBe("false");
		expect(control?.disabled).toBe(false);
		expect(control?.getAttribute("aria-label")).toBe("Let bots listen");
		await act(async () => {
			control?.click();
		});
		expect(changes).toEqual([true]);
	});

	test("is disabled with a hint when there is no speech model", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state, changes } = joinedState({ canListen: false });
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const control = toggle(container);
		expect(control?.disabled).toBe(true);
		expect(
			container
				.querySelector("[data-slot=call-listen-toggle]")
				?.getAttribute("title"),
		).toBe("Add a speech-to-text model to your profile to let bots listen.");
		await act(async () => {
			control?.click();
		});
		expect(changes).toEqual([]);
	});

	test("shows it on, and marks who is being transcribed", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state } = joinedState({ listening: true });
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		expect(toggle(container)?.getAttribute("aria-checked")).toBe("true");
		const people = [...container.querySelectorAll("li")];
		expect(people.map((p) => label(p))).toEqual([
			"Me",
			"Anna lets bots listen",
		]);
		expect(
			people.map((p) => p.querySelector("[data-slot=call-listening-badge]")),
		).toEqual([null, expect.anything()]);
	});

	test("listeners in someone else's call are shown before you join", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state } = joinedState({ joined: false });
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		expect(toggle(container)).toBeNull();
		expect(
			container.querySelectorAll("[data-slot=call-listening-badge]"),
		).toHaveLength(1);
	});
});

describe("camera and screen controls", () => {
	test("toggles camera and screen independently while microphone controls remain available", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		let camera = 0;
		let screen = 0;
		const { state, calls } = callState({
			joined: true,
			cameraOn: true,
			screenSharing: false,
			canUseCamera: true,
			canShareScreen: true,
			mediaPending: null,
			toggleCamera: async () => {
				camera++;
			},
			toggleScreenShare: async () => {
				screen++;
			},
		});
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const cameraButton = container.querySelector<HTMLButtonElement>(
			"[aria-label='Turn camera off']",
		);
		const screenButton = container.querySelector<HTMLButtonElement>(
			"[aria-label='Share screen']",
		);
		expect(cameraButton?.getAttribute("aria-pressed")).toBe("true");
		expect(screenButton?.getAttribute("aria-pressed")).toBe("false");
		await act(async () => {
			cameraButton?.click();
			screenButton?.click();
			container
				.querySelector<HTMLButtonElement>("[aria-label='Mute microphone']")
				?.click();
		});
		expect([camera, screen, calls.mute]).toEqual([1, 1, 1]);
	});

	test("unsupported screen capture is disabled with an explanation", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state } = callState({
			joined: true,
			canUseCamera: true,
			canShareScreen: false,
		});
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const control = container.querySelector<HTMLButtonElement>(
			"[aria-label='Share screen']",
		);
		expect(control?.disabled).toBe(true);
		expect(control?.title).toContain("supported desktop browser");
	});

	test("shows capture failures without removing the active call", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const { state } = callState({
			joined: true,
			mediaError: { source: "screen", reason: "denied" },
		});
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		expect(container.querySelector("[role=alert]")?.textContent).toContain(
			"cancelled or denied",
		);
		expect(container.querySelector("[aria-label='Leave call']")).not.toBeNull();
	});

	test("shows camera and screen together, with silent video playback to avoid doubled audio", async () => {
		const { ConversationCallBar } = await import("./conversation-call");
		const camera = new domWindow.MediaStream() as unknown as MediaStream;
		const screen = new domWindow.MediaStream() as unknown as MediaStream;
		const { state } = callState({
			joined: true,
			videos: [
				{
					id: "camera",
					sub: "me",
					kind: "camera",
					stream: camera,
					local: true,
				},
				{
					id: "screen",
					sub: "anna",
					kind: "screen",
					stream: screen,
					local: false,
				},
			],
		});
		const container = await render((c) =>
			c(ConversationCallBar, { call: state as never, me: "me", directory }),
		);
		const videos = [...container.querySelectorAll("video")];
		expect(videos).toHaveLength(2);
		expect(videos.map((v) => v.muted)).toEqual([true, true]);
		expect(videos.map((v) => v.srcObject)).toEqual([screen, camera]);
		expect(videos.map((v) => v.getAttribute("aria-label"))).toEqual([
			"Anna's screen",
			"Your camera",
		]);
		expect(videos[1].className).toContain("-scale-x-100");
	});
});
