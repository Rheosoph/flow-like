import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { IEventPayload } from "../../../../lib/schema/flow/event-payload";
import type { INode } from "../../../../lib/schema/flow/node";
import { allByRole, click, installDom, typeInto } from "../testing/dom-harness";

const dom = installDom();
const { useState } = await import("react");
const { BotAnswersLine } = await import("./bot-answers-line");
const { CellSub } = await import("../primitives/dv-table");
const { DiscordConfig } = await import("../../../interfaces/configs/discord");
const { TelegramConfig } = await import("../../../interfaces/configs/telegram");

afterEach(dom.cleanup);
afterAll(dom.restore);

const SENTENCE = {
	prefix: (prefix: string) =>
		`In groups and servers it answers mentions, replies and every message that starts with ${prefix}.`,
	prefixOnly: (prefix: string) =>
		`In groups and servers it answers every message that starts with ${prefix}.`,
	mentions: "In groups and servers it answers mentions and replies.",
	every:
		"In groups and servers it answers every message: no command prefix is set.",
};

const line = (root: ParentNode) =>
	root.querySelector<HTMLElement>("[data-bot-answers]");
const answers = (root: ParentNode) => line(root)?.textContent ?? null;
const prefixField = (root: ParentNode) =>
	root.querySelector<HTMLInputElement>("input#command_prefix");
/** Read-only mode: the value under the prefix label. */
const savedPrefix = (root: ParentNode) =>
	root.querySelector("label[for=command_prefix]")?.nextElementSibling
		?.textContent ?? null;

describe("the line", () => {
	test("says what a bot answers, from its facts or from an event's type and config", async () => {
		const view = await dom.render(
			<BotAnswersLine
				bot={{
					provider: "telegram",
					open: true,
					savedToken: false,
					prefix: "/",
					mentions: false,
				}}
			/>,
		);
		expect(answers(view.container)).toBe(SENTENCE.prefixOnly("/"));
		expect(line(view.container)?.tagName).toBe("P");
		expect(line(view.container)?.dataset.botAnswers).toBe("prefix_only");

		// As the wizard's table renders it: the caller's own line component.
		await view.rerender(
			<BotAnswersLine
				as={CellSub}
				eventType="discord"
				config={{}}
				className="x"
			/>,
		);
		expect(answers(view.container)).toBe(SENTENCE.mentions);
		expect(line(view.container)?.tagName).toBe("SPAN");
		expect(line(view.container)?.classList.contains("x")).toBe(true);
	});

	test("renders nothing for an event that is no bot or settings a device can't read", async () => {
		const silent: [string, unknown][] = [
			["teams", {}],
			["telegram", null],
			["telegram", { chat_whitelist: "all" }],
			["discord", { command_prefix: 7 }],
			["discord", { command_prefix: "x".repeat(17) }],
		];
		const view = await dom.render(null);
		for (const [eventType, config] of silent) {
			await view.rerender(
				<BotAnswersLine eventType={eventType} config={config} />,
			);
			expect([eventType, config, view.container.innerHTML]).toEqual([
				eventType,
				config,
				"",
			]);
		}
	});
});

const NODE = {
	id: "entry",
	name: "events_chat",
	friendly_name: "Chat",
	description: "",
	category: "events",
	pins: {},
} as INode;

/** The config the editor wrote last: what the Events page saves and a bot reads. */
let written: Partial<IEventPayload> = {};

/** The behaviour section of a bot's editor, holding the working config as the Events page does. */
function Editor({
	Config,
	start,
	editing = true,
}: Readonly<{
	Config: typeof DiscordConfig;
	start: Partial<IEventPayload>;
	editing?: boolean;
}>) {
	const [config, setConfig] = useState(start);
	return (
		<Config
			isEditing={editing}
			appId="app"
			boardId="board"
			nodeId={NODE.id}
			node={NODE}
			config={config}
			onConfigUpdate={(next) => {
				written = next;
				setConfig(next);
			}}
			section="behaviour"
		/>
	);
}

describe("Events editor · Discord", () => {
	test("no saved prefix shows an empty field, and the line follows the form while typing", async () => {
		const view = await dom.render(<Editor Config={DiscordConfig} start={{}} />);
		const field = prefixField(view.container);
		expect(field?.value).toBe("");
		expect(field?.hasAttribute("placeholder")).toBe(false);
		expect(
			view.container.querySelector("label[for=respond_to_mentions]")
				?.textContent,
		).toBe("Respond to Mentions");
		expect(answers(view.container)).toBe(SENTENCE.mentions);

		await typeInto(field as Element, "!");
		expect(written).toEqual({ command_prefix: "!" });
		expect(answers(view.container)).toBe(SENTENCE.prefix("!"));
		await click(
			view.container.querySelector("#respond_to_mentions") as Element,
		);
		expect(answers(view.container)).toBe(SENTENCE.prefixOnly("!"));
		// Clearing the field saves an empty prefix, which a bot reads as none.
		await typeInto(prefixField(view.container) as Element, "");
		expect(written).toEqual({ command_prefix: "", respond_to_mentions: false });
		expect(answers(view.container)).toBe(SENTENCE.every);
	});

	test("read-only shows the saved prefix, nothing when none is saved, and the same line", async () => {
		const view = await dom.render(
			<Editor
				Config={DiscordConfig}
				start={{ command_prefix: "?", respond_to_mentions: false }}
				editing={false}
			/>,
		);
		expect(prefixField(view.container)).toBeNull();
		expect(savedPrefix(view.container)).toBe("?");
		expect(answers(view.container)).toBe(SENTENCE.prefixOnly("?"));

		await view.rerender(
			<Editor key="none" Config={DiscordConfig} start={{}} editing={false} />,
		);
		expect(savedPrefix(view.container)).toBe("");
		expect(answers(view.container)).toBe(SENTENCE.mentions);
	});
});

describe("Events editor · Telegram", () => {
	test("no saved prefix shows an empty field, and the line follows the form while typing", async () => {
		const view = await dom.render(
			<Editor Config={TelegramConfig} start={{}} />,
		);
		const field = prefixField(view.container);
		expect(field?.value).toBe("");
		expect(field?.hasAttribute("placeholder")).toBe(false);
		expect(answers(view.container)).toBe(SENTENCE.every);

		await typeInto(field as Element, "/");
		expect(written).toEqual({ command_prefix: "/" });
		expect(answers(view.container)).toBe(SENTENCE.prefix("/"));
		const [mentions] = allByRole("switch", undefined, view.container);
		await click(mentions);
		expect(written).toEqual({
			command_prefix: "/",
			respond_to_mentions: false,
		});
		expect(answers(view.container)).toBe(SENTENCE.prefixOnly("/"));
		// A prefix a device can't read: the wizard shows no sentence for it either.
		await typeInto(prefixField(view.container) as Element, "x".repeat(17));
		expect(line(view.container)).toBeNull();
	});

	test("read-only shows the saved prefix, nothing when none is saved, and the same line", async () => {
		const view = await dom.render(
			<Editor
				Config={TelegramConfig}
				start={{ command_prefix: "/" }}
				editing={false}
			/>,
		);
		expect(prefixField(view.container)).toBeNull();
		expect(savedPrefix(view.container)).toBe("/");
		expect(answers(view.container)).toBe(SENTENCE.prefix("/"));

		await view.rerender(
			<Editor key="none" Config={TelegramConfig} start={{}} editing={false} />,
		);
		expect(savedPrefix(view.container)).toBe("");
		expect(answers(view.container)).toBe(SENTENCE.every);
	});
});
