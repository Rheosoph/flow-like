import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { BackendContext, type IBackendState } from "../state/backend-state";
import type { IUserLookup } from "../state/backend-state/types";
import { type UserIdentity, useUserIdentity } from "./use-user-lookup";

createI18n({ language: "en" });

const READER = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";
const OWNER = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";
const BOT = "bot_tz4a98xxat96iws9zmbrgj3a";

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

function person(id: string, name: string, extra: Partial<IUserLookup> = {}) {
	return { id, name, created_at: "", ...extra } satisfies IUserLookup;
}

function bot(
	extra: Partial<IUserLookup> = {},
	details: Partial<NonNullable<IUserLookup["bot"]>> = {},
): IUserLookup {
	return {
		id: BOT,
		name: "Scout",
		created_at: "",
		...extra,
		bot: {
			owner_id: OWNER,
			system: null,
			appearance: { kind: "mochi" },
			deleted: false,
			...details,
		},
	};
}

/** Seeds the batched lookup cache by id; `local` is whoever is signed in. */
function identityOf(
	id: string,
	seeded: Record<string, IUserLookup | null>,
): UserIdentity {
	const client = new QueryClient();
	for (const [key, value] of Object.entries(seeded)) {
		client.setQueryData(["lookupUserBatched", key], value);
	}

	let identity: UserIdentity | undefined;
	function Probe() {
		identity = useUserIdentity(id);
		return null;
	}

	renderToStaticMarkup(
		<QueryClientProvider client={client}>
			<BackendContext.Provider value={backend}>
				<Probe />
			</BackendContext.Provider>
		</QueryClientProvider>,
	);
	if (!identity) throw new Error("the probe never rendered");
	return identity;
}

describe("useUserIdentity for people", () => {
	test("reads a person exactly as before", () => {
		const identity = identityOf(OWNER, {
			[OWNER]: person(OWNER, "Anna Weber", {
				email: "anna@example.com",
				avatar_url: "https://example.com/anna.png",
			}),
		});

		expect(identity.label).toBe("Anna Weber");
		expect(identity.subtitle).toBe("anna@example.com");
		expect(identity.avatarUrl).toBe("https://example.com/anna.png");
		expect(identity.initials).toBe("AW");
		expect(identity.bot).toBeUndefined();
		expect(identity.isResolved).toBe(true);
	});
});

describe("useUserIdentity for bots", () => {
	test("names the bot and whose it is", () => {
		const identity = identityOf(BOT, {
			[BOT]: bot(),
			[OWNER]: person(OWNER, "Anna Weber", {
				avatar_url: "https://example.com/anna.png",
			}),
			local: person(READER, "Felix Schultz"),
		});

		expect(identity.label).toBe("Scout");
		expect(identity.subtitle).toBe("Anna Weber's bot");
		expect(identity.initials).toBe("SC");
		expect(identity.avatarUrl).toBeUndefined();
		expect(identity.bot).toMatchObject({
			ownerId: OWNER,
			isOwn: false,
			deleted: false,
			appearance: { kind: "mochi" },
			owner: {
				label: "Anna Weber",
				avatarUrl: "https://example.com/anna.png",
				initials: "AW",
				isPending: false,
			},
		});
	});

	test("calls the reader's own bot theirs", () => {
		const identity = identityOf(BOT, {
			[BOT]: bot(),
			[OWNER]: person(OWNER, "Anna Weber"),
			local: person(OWNER, "Anna Weber"),
		});

		expect(identity.subtitle).toBe("Your bot");
		expect(identity.bot?.isOwn).toBe(true);
	});

	test("gives an unnamed built-in bot its name in the reader's language", () => {
		const identity = identityOf(BOT, {
			[BOT]: bot({ name: undefined }, { system: "flux" }),
			[OWNER]: person(OWNER, "Anna Weber"),
			local: person(OWNER, "Anna Weber"),
		});

		expect(identity.label).toBe("Flux");
		expect(identity.bot?.system).toBe("flux");
		expect(identity.bot?.description).toBe(
			"Builds and fixes your apps and workflows",
		);
	});

	test("keeps a renamed built-in bot's own name", () => {
		const identity = identityOf(BOT, {
			[BOT]: bot({ name: "Helper" }, { system: "general" }),
			[OWNER]: person(OWNER, "Anna Weber"),
			local: person(OWNER, "Anna Weber"),
		});

		expect(identity.label).toBe("Helper");
	});

	test("keeps a deleted bot's name and says it is gone", () => {
		const identity = identityOf(BOT, {
			[BOT]: bot({}, { deleted: true }),
			[OWNER]: person(OWNER, "Anna Weber"),
			local: person(READER, "Felix Schultz"),
		});

		expect(identity.label).toBe("Scout");
		expect(identity.subtitle).toBe("Deleted bot");
		expect(identity.bot?.deleted).toBe(true);
	});

	test("says nothing about the owner until both lookups land", () => {
		const identity = identityOf(BOT, { [BOT]: bot() });

		expect(identity.label).toBe("Scout");
		expect(identity.subtitle).toBeUndefined();
		expect(identity.bot?.owner.isPending).toBe(true);
		expect(identity.bot?.isOwn).toBe(false);
	});

	test("still reads as a bot when its owner cannot be found", () => {
		const identity = identityOf(BOT, {
			[BOT]: bot(),
			[OWNER]: null,
			local: person(READER, "Felix Schultz"),
		});

		expect(identity.subtitle).toBe("Bot");
		expect(identity.bot?.owner.label).toBe(OWNER);
	});
});
