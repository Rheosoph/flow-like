import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { BotIdentity, UserIdentity } from "../../hooks/use-user-lookup";
import { BackendContext, type IBackendState } from "../../state/backend-state";
import type { IUserLookup } from "../../state/backend-state/types";
import { IdentityAvatar, ResolvedIdentityAvatar } from "./identity-avatar";

createI18n({ language: "en" });

const OWNER = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";
const BOT = "bot_tz4a98xxat96iws9zmbrgj3a";

function render(element: ReactElement): Document {
	const document = new Window().document as unknown as Document;
	document.body.innerHTML = renderToStaticMarkup(element);
	return document;
}

function person(): UserIdentity {
	return {
		user: { id: OWNER, name: "Anna Weber", created_at: "" },
		accountId: OWNER,
		label: "Anna Weber",
		initials: "AW",
		isPending: false,
		isResolved: true,
		isError: false,
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
			owner: { label: "Anna Weber", initials: "AW", isPending: false },
			...extra,
		},
	};
}

function ownerBadge(document: Document): Element | null {
	return document.querySelector('[role="img"] > [data-slot="avatar"]');
}

describe("ResolvedIdentityAvatar", () => {
	test("draws a person as the avatar every identity surface uses", () => {
		const document = render(
			<ResolvedIdentityAvatar identity={person()} size={24} />,
		);

		const avatar = document.querySelector('[data-slot="avatar"]');
		expect(avatar?.getAttribute("style")).toContain("width:24px");
		expect(avatar?.textContent).toBe("AW");
		expect(document.querySelector(".fl-bot")).toBeNull();
	});

	test("draws a bot with its owner in the corner and says whose it is", () => {
		const document = render(
			<ResolvedIdentityAvatar identity={botIdentity()} size={40} />,
		);

		const root = document.querySelector('[role="img"]');
		expect(root?.getAttribute("aria-label")).toBe("Scout, Anna Weber's bot");
		expect(root?.getAttribute("style")).toContain("width:40px");
		expect(root?.querySelector(".fl-bot")?.getAttribute("aria-hidden")).toBe(
			"true",
		);

		const badge = ownerBadge(document);
		expect(badge?.getAttribute("style")).toContain("width:16px");
		expect(badge?.className).toContain("border-background");
		expect(badge?.textContent).toBe("A");
	});

	test("gives a large badge both of the owner's initials", () => {
		const document = render(
			<ResolvedIdentityAvatar identity={botIdentity()} size={128} />,
		);

		expect(ownerBadge(document)?.textContent).toBe("AW");
	});

	test("keeps a legible, initial-free owner badge on the smallest bot", () => {
		const document = render(
			<ResolvedIdentityAvatar identity={botIdentity()} size={16} />,
		);

		expect(document.querySelector(".fl-bot")).not.toBeNull();
		const badge = ownerBadge(document);
		expect(badge?.getAttribute("style")).toContain("width:10px");
		expect(badge?.textContent).toBe("");
	});

	test("holds the bot still unless asked to move", () => {
		const document = render(
			<ResolvedIdentityAvatar identity={botIdentity()} size={32} />,
		);

		expect(document.querySelector(".fl-bot")?.getAttribute("data-motion")).toBe(
			"off",
		);
	});

	test("mutes a deleted bot", () => {
		const document = render(
			<ResolvedIdentityAvatar
				identity={botIdentity({ deleted: true })}
				size={32}
			/>,
		);

		const root = document.querySelector('[role="img"]');
		expect(root?.hasAttribute("data-bot-deleted")).toBe(true);
		expect(root?.className).toContain("grayscale");
	});

	test("leaves the badge blank while the owner is still loading", () => {
		const document = render(
			<ResolvedIdentityAvatar
				identity={botIdentity({
					owner: { label: OWNER, initials: "32", isPending: true },
				})}
				size={40}
			/>,
		);

		expect(ownerBadge(document)?.textContent).toBe("");
	});
});

describe("IdentityAvatar", () => {
	const backend = {
		userState: {
			lookupUser: async () => {
				throw new Error("every lookup in these tests is seeded");
			},
			lookupUsers: async () => {
				throw new Error("every lookup in these tests is seeded");
			},
		},
	} as unknown as IBackendState;

	function renderSubject(
		subject: string,
		seeded: Record<string, IUserLookup | null>,
	): Document {
		const client = new QueryClient();
		for (const [key, value] of Object.entries(seeded)) {
			client.setQueryData(["lookupUserBatched", key], value);
		}
		return render(
			<QueryClientProvider client={client}>
				<BackendContext.Provider value={backend}>
					<IdentityAvatar subject={subject} size={32} />
				</BackendContext.Provider>
			</QueryClientProvider>,
		);
	}

	test("resolves a bot's sub to the bot acting for its owner", () => {
		const document = renderSubject(BOT, {
			[BOT]: {
				id: BOT,
				name: "Scout",
				created_at: "",
				bot: {
					owner_id: OWNER,
					system: null,
					appearance: { kind: "mochi" },
					deleted: false,
				},
			},
			[OWNER]: { id: OWNER, name: "Anna Weber", created_at: "" },
			local: { id: OWNER, name: "Anna Weber", created_at: "" },
		});

		const root = document.querySelector('[role="img"]');
		expect(root?.getAttribute("aria-label")).toBe("Scout, Your bot");
		expect(root?.getAttribute("title")).toBe("Scout, Your bot");
		expect(ownerBadge(document)?.textContent).toBe("A");
	});

	test("holds its place while the directory has not answered", () => {
		const document = renderSubject(BOT, {});

		expect(document.querySelector('[data-slot="skeleton"]')).not.toBeNull();
		expect(document.querySelector('[role="img"]')).toBeNull();
	});
});
