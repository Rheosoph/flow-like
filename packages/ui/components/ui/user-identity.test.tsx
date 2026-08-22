import { afterAll, describe, expect, mock, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import type { UserIdentity } from "../../hooks/use-user-lookup";

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";

let identity: UserIdentity;

// The tag's whole job is to render whatever the lookup settled on, so the lookup
// itself is stubbed and each of its three outcomes is rendered for real.
mock.module("../../hooks/use-user-lookup", () => ({
	useUserIdentity: () => identity,
}));

afterAll(() => mock.restore());

function pending(): UserIdentity {
	return {
		user: null,
		accountId: SUB,
		label: SUB,
		initials: "??",
		isPending: true,
		isResolved: false,
		isError: false,
	};
}

function unresolved(): UserIdentity {
	return { ...pending(), isPending: false };
}

function failed(): UserIdentity {
	return { ...unresolved(), isError: true };
}

function resolved(): UserIdentity {
	return {
		user: { id: SUB, name: "Felix Schultz", created_at: "" },
		accountId: SUB,
		label: "Felix Schultz",
		subtitle: "felix@example.com",
		initials: "FS",
		isPending: false,
		isResolved: true,
		isError: false,
	};
}

async function renderTag(props: Record<string, unknown> = {}) {
	const { UserInlineTag } = await import("./user-identity");
	return renderToStaticMarkup(<UserInlineTag userId={SUB} {...props} />);
}

describe("UserInlineTag", () => {
	test("holds the cell's shape while the directory has not answered", async () => {
		identity = pending();
		const markup = await renderTag();
		expect(markup).not.toContain(SUB);
		expect(markup).toContain("animate-pulse");
	});

	test("falls back to the stored id when no account matches", async () => {
		identity = unresolved();
		const markup = await renderTag();
		expect(markup).toContain(SUB);
		expect(markup).toContain("font-mono");
	});

	test("reads as the person once the directory answers", async () => {
		identity = resolved();
		const markup = await renderTag();
		expect(markup).toContain("Felix Schultz");
		expect(markup).toContain("FS");
		// The opaque id belongs in the hover card, not in the row.
		expect(markup).not.toContain(SUB);
	});

	test("is a button only where the host wired one", async () => {
		for (const state of [pending, unresolved, resolved]) {
			identity = state();
			expect(await renderTag({ onClick: () => {} })).toContain("<button");
			expect(await renderTag()).not.toContain("<button");
		}
	});

	test("falls back to the id whether the account is missing or unreachable", async () => {
		// The reason lives in a tooltip, which Radix only renders once opened; the
		// card below asserts the wording.
		identity = failed();
		expect(await renderTag()).toContain(SUB);
	});

	test("carries the host's chrome onto the trigger in every state", async () => {
		for (const state of [pending, unresolved, resolved]) {
			identity = state();
			expect(await renderTag({ className: "h-6 px-2" })).toContain("h-6 px-2");
		}
	});
});

describe("UserIdentityCard", () => {
	test("always shows the stored id, resolved or not", async () => {
		const { UserIdentityCard } = await import("./user-identity");

		identity = resolved();
		const found = renderToStaticMarkup(<UserIdentityCard userId={SUB} />);
		expect(found).toContain("Felix Schultz");
		expect(found).toContain(SUB);

		identity = unresolved();
		const missing = renderToStaticMarkup(<UserIdentityCard userId={SUB} />);
		expect(missing).toContain(SUB);
		expect(missing).not.toContain("Felix Schultz");

		expect(missing).toContain("No account matches this id");
	});

	test("never claims the account is missing when the lookup itself failed", async () => {
		const { UserIdentityCard } = await import("./user-identity");

		identity = failed();
		const broken = renderToStaticMarkup(<UserIdentityCard userId={SUB} />);
		expect(broken).toContain("Could not resolve this account");
		expect(broken).not.toContain("No account matches this id");
		expect(broken).toContain(SUB);
	});
});
