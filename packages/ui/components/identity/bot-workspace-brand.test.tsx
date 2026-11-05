import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { renderToStaticMarkup } from "react-dom/server";
import { botKeys } from "../../hooks/use-bots";
import type { IApp } from "../../lib/schema/app/app";
import { BackendContext, type IBackendState } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	BotWorkspaceAvatar,
	BotWorkspaceBlock,
	useWorkspaceBot,
} from "./bot-workspace-brand";

createI18n({ language: "en" });

const BOT: IBot = {
	id: "bot_joerg",
	owner_id: "owner",
	system: null,
	preset_id: null,
	name: "Joerg",
	description: null,
	instructions: null,
	appearance: { kind: "mochi" },
	model: { primary: null, fallbacks: [] },
	classification: "internal" as IBot["classification"],
	capabilities: [],
	profile_id: null,
	location: "server",
	device_id: null,
	created_at: "",
	updated_at: "",
};

function Sidebar({ app }: Readonly<{ app: Pick<IApp, "bot_id"> }>) {
	const bot = useWorkspaceBot(app);
	return (
		<nav>
			{bot && <BotWorkspaceAvatar bot={bot} size={22} />}
			{bot && <BotWorkspaceBlock bot={bot} onNavigate={() => {}} />}
			<span data-testid="plain">General</span>
		</nav>
	);
}

function render(app: Pick<IApp, "bot_id">): Document {
	const client = new QueryClient();
	client.setQueryData(botKeys.list(), [BOT]);
	const document = new Window().document as unknown as Document;
	document.body.innerHTML = renderToStaticMarkup(
		<QueryClientProvider client={client}>
			<BackendContext.Provider value={{ botState: {} } as IBackendState}>
				<Sidebar app={app} />
			</BackendContext.Provider>
		</QueryClientProvider>,
	);
	return document;
}

describe("bot workspace branding", () => {
	test("a bot workspace app shows the avatar, badge and identity block", () => {
		const document = render({ bot_id: BOT.id });

		expect(document.querySelector(".fl-bot")).not.toBeNull();
		expect(
			document.querySelector('[data-slot="bot-workspace-badge"]'),
		).not.toBeNull();
		const block = document.querySelector('[data-slot="bot-workspace-block"]');
		expect(block?.textContent).toContain("Joerg");
		expect(block?.textContent).toContain("Bot workspace");
		expect(block?.textContent).toContain("Cloud");
		expect(block?.textContent).toContain("Open chat");
		expect(block?.textContent).toContain("Bot settings");
	});

	test("a plain app renders nothing extra", () => {
		for (const app of [{}, { bot_id: null }]) {
			const document = render(app);
			expect(document.querySelector(".fl-bot")).toBeNull();
			expect(document.querySelector('[data-slot="bot-workspace-block"]')).toBe(
				null,
			);
		}
	});
});
