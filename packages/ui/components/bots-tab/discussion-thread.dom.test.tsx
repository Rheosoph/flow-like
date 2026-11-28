import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { type ReactElement, act, createElement } from "react";
import type { DiscussionThreadMessage as ThreadMessage } from "./discussion-thread-model";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";
import type { CrewDirectory } from "./use-crew-directory";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const actualAvatar = { ...(await import("../identity/identity-avatar")) };
mock.module("../identity/identity-avatar", () => ({
	IdentityAvatar: ({ subject }: { subject: string }) =>
		createElement("span", { "data-avatar": subject }),
}));

const { createRoot } = await import("react-dom/client");
const { DiscussionEnd, DiscussionHeader, DiscussionRoundTrack } = await import(
	"./discussion-thread"
);
const { discussionPlaces, roomDiscussions } = await import(
	"./discussion-thread-model"
);

const NAMES: Record<string, string> = { bot_ada: "Ada", bot_story: "Story" };
const directory = ((id: string) => ({
	name: NAMES[id] ?? id,
	isBot: id in NAMES,
	isMe: false,
	isResolved: true,
})) as unknown as CrewDirectory;

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

beforeAll(() => installDomGlobals());
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../identity/identity-avatar", () => actualAvatar);
	restoreDomGlobals();
});

async function show(element: ReactElement) {
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () => root.render(element));
}

async function hide() {
	await act(async () => root.unmount());
	container.remove();
}

function exchange(
	count: number,
	last: Partial<ThreadMessage> = {},
): ThreadMessage[] {
	const list: ThreadMessage[] = [
		{ id: "ask", senderId: "user-a", time: 0, mentions: [] },
	];
	for (let at = 1; at <= count; at++) {
		list.push({
			id: `m${at}`,
			senderId: at % 2 === 1 ? "bot_ada" : "bot_story",
			time: at,
			replyTo: list.at(-1)?.id,
			hops: at,
			mentions: at === 1 ? ["bot_story"] : [],
			...(at === count ? last : {}),
		});
	}
	return list;
}

function placeOf(list: ThreadMessage[], id: string) {
	const place = discussionPlaces(
		roomDiscussions(list, (sender) => sender in NAMES),
	).get(id);
	if (!place) throw new Error(`no discussion place for ${id}`);
	return place;
}

describe("a discussion between bots in a room", () => {
	test("the header names the pair and the round budget", async () => {
		await show(
			createElement(DiscussionHeader, {
				discussion: placeOf(exchange(2), "m1").discussion,
				directory,
			}),
		);
		expect(container.textContent).toContain("Discussion · up to 5 rounds");
		expect(
			[...container.querySelectorAll("[data-avatar]")].map((node) =>
				node.getAttribute("data-avatar"),
			),
		).toEqual(["bot_ada", "bot_story"]);
		await hide();
	});

	test("each message shows its round, with the track filled up to it", async () => {
		await show(
			createElement(DiscussionRoundTrack, {
				place: placeOf(exchange(3), "m3"),
			}),
		);
		expect(container.textContent).toBe("Round 2 of 5");
		const halves = [
			...container.querySelectorAll(
				"[data-slot=discussion-round] span span span",
			),
		];
		expect(halves).toHaveLength(10);
		expect(halves[2]?.className).toContain("bg-primary");
		expect(halves[1]?.className).toContain("bg-muted-foreground");
		expect(halves[3]?.className).toContain("bg-foreground/10");
		await hide();
	});

	test("an agreement shows the outcome and when it was reached", async () => {
		const list = exchange(4, {
			discussionOutcome: { agreed: true, summary: "Ship it on Friday." },
		});
		await show(
			createElement(DiscussionEnd, {
				place: placeOf(list, "m4"),
				directory,
				active: new Set<string>(),
			}),
		);
		expect(container.textContent).toContain("Agreed after 2 of 5 rounds");
		expect(container.textContent).toContain("Ship it on Friday.");
		expect(container.textContent).toContain("Ada and Story");
		await hide();
	});

	test("an open discussion says whose turn it is, live while that bot works", async () => {
		await show(
			createElement(DiscussionEnd, {
				place: placeOf(exchange(2), "m2"),
				directory,
				active: new Set<string>(),
				now: 2,
			}),
		);
		expect(container.textContent).toBe("Ada's turn");
		await hide();

		await show(
			createElement(DiscussionEnd, {
				place: placeOf(exchange(2), "m2"),
				directory,
				active: new Set(["bot_ada"]),
				now: 10 * 60_000,
			}),
		);
		expect(container.textContent).toBe("Ada is replying");
		await hide();

		await show(
			createElement(DiscussionEnd, {
				place: placeOf(exchange(2), "m2"),
				directory,
				active: new Set<string>(),
				now: 10 * 60_000,
			}),
		);
		expect(container.textContent).toBe("");
		await hide();
	});

	test("used-up rounds say so, and a waiting proposal leaves it to its card", async () => {
		await show(
			createElement(DiscussionEnd, {
				place: placeOf(exchange(10), "m10"),
				directory,
				active: new Set<string>(),
			}),
		);
		expect(container.textContent).toContain("All 5 rounds used");
		await hide();

		await show(
			createElement(DiscussionEnd, {
				place: placeOf(
					exchange(3, { discussionProposal: { rounds: 9, reason: "More." } }),
					"m3",
				),
				directory,
				active: new Set<string>(),
			}),
		);
		expect(container.textContent).toBe("");
		await hide();
	});
});
