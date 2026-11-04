import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { Window } from "happy-dom";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type {
	BotIdentity,
	BotOwnerIdentity,
	UserIdentity,
} from "../../hooks/use-user-lookup";
import { BotIdentityCard } from "./bot-identity-card";

createI18n({ language: "en" });

const OWNER = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";
const BOT = "bot_tz4a98xxat96iws9zmbrgj3a";

function render(element: ReactElement): Document {
	const document = new Window().document as unknown as Document;
	document.body.innerHTML = renderToStaticMarkup(element);
	return document;
}

function owner(extra: Partial<BotOwnerIdentity> = {}): BotOwnerIdentity {
	return {
		label: "Anna Weber",
		subtitle: "@anna",
		accountId: OWNER,
		initials: "AW",
		isPending: false,
		...extra,
	};
}

function botIdentity(extra: Partial<BotIdentity> = {}): UserIdentity {
	return {
		user: { id: BOT, name: "Scout", created_at: "" },
		accountId: BOT,
		label: "Scout",
		subtitle: "Anna Weber's bot",
		initials: "SC",
		isPending: false,
		isResolved: true,
		isError: false,
		bot: {
			ownerId: OWNER,
			system: null,
			appearance: { kind: "mochi" },
			deleted: false,
			isOwn: false,
			description: "Finds sources and summarises them",
			owner: owner(),
			...extra,
		},
	};
}

function ownerSection(document: Document): Element | null | undefined {
	return document.querySelector('[data-slot="bot-identity-card"]')?.children[1];
}

describe("BotIdentityCard", () => {
	test("introduces the bot, then the person it acts for", () => {
		const document = render(<BotIdentityCard identity={botIdentity()} />);

		const text = document.body.textContent ?? "";
		expect(text).toContain("Scout");
		expect(text).toContain("Bot");
		expect(text).toContain("Finds sources and summarises them");

		const section = ownerSection(document);
		expect(section?.textContent).toContain("Acts for");
		expect(section?.textContent).toContain("Anna Weber");
		expect(section?.textContent).toContain("@anna");
		expect(section?.querySelector("a")?.getAttribute("href")).toBe(
			`/profile?sub=${OWNER}`,
		);
		expect(text).toContain(BOT);
	});

	test("labels a built-in bot and a deleted one", () => {
		const builtIn = render(
			<BotIdentityCard identity={botIdentity({ system: "flux" })} />,
		);
		expect(builtIn.body.textContent).toContain("Built in");

		const deleted = render(
			<BotIdentityCard identity={botIdentity({ deleted: true })} />,
		);
		expect(deleted.body.textContent).toContain("Deleted bot");
	});

	test("marks the reader's own bot, and names a signed-out owner as them", () => {
		const own = render(
			<BotIdentityCard identity={botIdentity({ isOwn: true })} />,
		);
		expect(ownerSection(own)?.textContent).toContain("Anna Weber");
		expect(ownerSection(own)?.textContent).toContain("You");

		const local = render(
			<BotIdentityCard
				identity={botIdentity({
					isOwn: true,
					ownerId: "local",
					owner: owner({
						label: "local",
						initials: "LO",
						subtitle: undefined,
						accountId: undefined,
					}),
				})}
			/>,
		);
		expect(ownerSection(local)?.textContent).toContain("You");
		expect(ownerSection(local)?.textContent).not.toContain("local");
		expect(ownerSection(local)?.textContent).not.toContain("LO");
		expect(ownerSection(local)?.querySelector("a")).toBeNull();
	});

	test("waits for the owner instead of showing their raw id", () => {
		const document = render(
			<BotIdentityCard
				identity={botIdentity({
					owner: owner({ label: OWNER, accountId: undefined, isPending: true }),
				})}
			/>,
		);

		const section = ownerSection(document);
		expect(section?.querySelector('[data-slot="skeleton"]')).not.toBeNull();
		expect(section?.textContent).not.toContain(OWNER);
	});

	test("leaves out what the host switched off", () => {
		const document = render(
			<BotIdentityCard
				identity={botIdentity()}
				showId={false}
				showProfileLink={false}
			/>,
		);

		expect(document.body.textContent).not.toContain(BOT);
		expect(document.querySelector("a")).toBeNull();
	});

	test("renders nothing for a person", () => {
		const { bot: _bot, ...person } = botIdentity();
		expect(renderToStaticMarkup(<BotIdentityCard identity={person} />)).toBe(
			"",
		);
	});
});
