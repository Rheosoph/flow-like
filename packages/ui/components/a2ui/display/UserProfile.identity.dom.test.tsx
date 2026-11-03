import { describe, expect, test } from "bun:test";
import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
	type LocalPrincipalResolver,
	LocalPrincipalsProvider,
} from "../../../hooks/use-user-lookup";
import {
	BackendContext,
	type IBackendState,
} from "../../../state/backend-state";
import type { IUserLookup } from "../../../state/backend-state/types";

createI18n({ language: "en" });

// Radix picks its layout effect when first imported, so the component loads under a document.
const documentDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"document",
);
Object.assign(globalThis, { document: new Window().document });
const { A2UIUserProfile } = await import("./UserProfile");
if (documentDescriptor)
	Object.defineProperty(globalThis, "document", documentDescriptor);
else Reflect.deleteProperty(globalThis, "document");

const OWNER = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";
const BOT = "bot_tz4a98xxat96iws9zmbrgj3a";
const LOCAL_BOT = "bot_local_1";
const VARIANTS = ["avatar", "chip", "row", "detailed", "card"] as const;

const ANNA: IUserLookup = { id: OWNER, name: "Anna Weber", created_at: "" };

function botLookup(id: string, deleted = false): IUserLookup {
	return {
		id,
		name: "Scout",
		description: "Reads the news",
		created_at: "",
		bot: {
			owner_id: OWNER,
			system: null,
			appearance: { kind: "mochi" },
			deleted,
		},
	};
}

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

function renderProfile(
	value: string,
	variant: string,
	seeded: Record<string, IUserLookup | null>,
	resolver?: LocalPrincipalResolver,
	avatarSize?: string,
): Document {
	const client = new QueryClient();
	for (const [key, user] of Object.entries(seeded)) {
		client.setQueryData(["lookupUserBatched", key], user);
	}
	const element: ReactElement = (
		<QueryClientProvider client={client}>
			<BackendContext.Provider value={backend}>
				<LocalPrincipalsProvider value={resolver ?? null}>
					<A2UIUserProfile
						componentId="p"
						surfaceId="s"
						renderChild={() => null}
						component={
							{
								type: "userProfile",
								value,
								variant,
								avatarSize,
								showHover: false,
							} as never
						}
					/>
				</LocalPrincipalsProvider>
			</BackendContext.Provider>
		</QueryClientProvider>
	);
	const document = new Window().document as unknown as Document;
	document.body.innerHTML = renderToStaticMarkup(element);
	return document;
}

const seededAccount = {
	[BOT]: botLookup(BOT),
	[OWNER]: ANNA,
	local: { id: "someone-else", name: "Other", created_at: "" },
};

function ownerBadge(document: Document) {
	return document.querySelector('[role="img"] > [data-slot="avatar"]');
}

describe("A2UIUserProfile bot identity", () => {
	for (const variant of VARIANTS) {
		test(`draws an account bot in the ${variant} variant with its owner badge`, () => {
			const document = renderProfile(BOT, variant, seededAccount);

			expect(document.querySelector(".fl-bot")).not.toBeNull();
			expect(ownerBadge(document)).not.toBeNull();
			if (variant !== "avatar") {
				expect(document.body.textContent).toContain("Scout");
			}
			if (variant === "row" || variant === "detailed" || variant === "card") {
				expect(document.body.textContent).toContain("Anna Weber's bot");
			}
		});
	}

	test("keeps the owner badge at the smallest avatar size", () => {
		const document = renderProfile(BOT, "chip", seededAccount, undefined, "xs");
		const badge = ownerBadge(document);

		expect(badge).not.toBeNull();
		expect(badge?.getAttribute("style")).toContain("width:10px");
	});

	test("resolves a local bot through the app-wide resolver", () => {
		const resolver: LocalPrincipalResolver = (id) =>
			id === LOCAL_BOT ? botLookup(LOCAL_BOT) : undefined;
		const document = renderProfile(
			LOCAL_BOT,
			"row",
			{ local: { id: OWNER, name: "Anna Weber", created_at: "" } },
			resolver,
		);

		expect(document.querySelector(".fl-bot")).not.toBeNull();
		expect(ownerBadge(document)).not.toBeNull();
		expect(document.body.textContent).toContain("Scout");
	});

	test("renders a person as before, without a bot", () => {
		const document = renderProfile(OWNER, "row", { [OWNER]: ANNA });

		expect(document.querySelector(".fl-bot")).toBeNull();
		expect(document.querySelector('[role="img"]')).toBeNull();
		expect(document.body.textContent).toContain("Anna Weber");
	});

	test("greys a deleted bot", () => {
		const document = renderProfile(BOT, "chip", {
			...seededAccount,
			[BOT]: botLookup(BOT, true),
		});
		const root = document.querySelector('[role="img"]');

		expect(root?.hasAttribute("data-bot-deleted")).toBe(true);
		expect(root?.className).toContain("grayscale");
		expect(ownerBadge(document)).not.toBeNull();
	});
});
