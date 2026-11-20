import { expect, test } from "bun:test";
import {
	CALDAV_PRESETS,
	accountPolicy,
	calDavCreateRequest,
	calDavFormReady,
	caldavPreset,
	chosenCalendar,
	isCalDavSignInError,
	isDrifted,
	normalizeCalDavAddress,
	normalizeMcpUrl,
	readsWriteTool,
	toolPolicy,
	withToolPolicy,
} from "./bot-connectors-model";

const tool = (
	name: string,
	hints: { read_only_hint?: boolean; destructive_hint?: boolean } = {},
) => ({ name, description: "", schema_hash: "h", ...hints });

test("hints decide a tool's policy until the owner chooses", () => {
	const connector = { tool_policies: {}, default_policy: "ask" as const };
	expect(
		toolPolicy(connector, tool("search", { read_only_hint: true })),
	).toEqual({ policy: "read", chosen: false });
	expect(toolPolicy(connector, tool("create"))).toEqual({
		policy: "ask",
		chosen: false,
	});
	expect(
		toolPolicy(connector, tool("drop", { destructive_hint: true })).policy,
	).toBe("ask");
	const chosen = { ...connector, tool_policies: { search: "never" as const } };
	expect(toolPolicy(chosen, tool("search", { read_only_hint: true }))).toEqual({
		policy: "never",
		chosen: true,
	});
});

test("an account reads without asking and asks to write", () => {
	expect(accountPolicy({ tool_policies: {} }, "read").policy).toBe("read");
	expect(accountPolicy({ tool_policies: {} }, "write").policy).toBe("ask");
	expect(accountPolicy({ tool_policies: { write: "never" } }, "write")).toEqual(
		{ policy: "never", chosen: true },
	);
});

test("always allowing a tool that is not read only is flagged", () => {
	expect(readsWriteTool(tool("create"), "read")).toBe(true);
	expect(readsWriteTool(tool("search", { read_only_hint: true }), "read")).toBe(
		false,
	);
	expect(readsWriteTool(tool("create"), "ask")).toBe(false);
});

test("policy changes keep the others and urls get a scheme", () => {
	expect(withToolPolicy({ a: "read" }, "b", "never")).toEqual({
		a: "read",
		b: "never",
	});
	expect(normalizeMcpUrl(" mcp.example.com/mcp ")).toBe(
		"https://mcp.example.com/mcp",
	);
	expect(normalizeMcpUrl("http://127.0.0.1:8931/mcp")).toBe(
		"http://127.0.0.1:8931/mcp",
	);
	expect(normalizeMcpUrl("  ")).toBe("");
});

test("a tool whose schema changed since it was confirmed is drifted", () => {
	const tool = { name: "a", description: "", schema_hash: "v2" };
	expect(isDrifted({ confirmed_hashes: { a: "v1" } }, tool)).toBe(true);
	expect(isDrifted({ confirmed_hashes: { a: "v2" } }, tool)).toBe(false);
	expect(isDrifted({ confirmed_hashes: {} }, tool)).toBe(false);
	expect(isDrifted({}, tool)).toBe(false);
});

test("presets prefill the address; placeholders keep the form unready until replaced", () => {
	expect(caldavPreset("icloud").address).toBe("https://caldav.icloud.com/");
	expect(CALDAV_PRESETS.map((preset) => preset.id)).toHaveLength(8);
	const form = {
		label: "Nextcloud",
		address: caldavPreset("nextcloud").address,
		username: "ann",
		password: "secret",
	};
	expect(calDavFormReady(form)).toBe(false);
	expect(
		calDavFormReady({
			...form,
			address: "https://cloud.example.com/remote.php/dav/",
		}),
	).toBe(true);
	expect(
		calDavFormReady({ ...form, address: "https://x.test/", password: "" }),
	).toBe(false);
});

test("a CalDAV address needs a host and no credentials in it", () => {
	expect(normalizeCalDavAddress(" cloud.example.com/dav ")).toBe(
		"https://cloud.example.com/dav",
	);
	expect(normalizeCalDavAddress("http://localhost:5232/")).toBe(
		"http://localhost:5232/",
	);
	expect(normalizeCalDavAddress("ftp://x.test/")).toBe("");
	expect(normalizeCalDavAddress("https://ann:pw@x.test/")).toBe("");
});

test("the create request carries the password as the grant only", () => {
	expect(
		calDavCreateRequest({
			label: " Home ",
			address: "https://caldav.icloud.com/",
			username: " ann@example.com ",
			password: "abcd-efgh",
		}),
	).toEqual({
		kind: "caldav",
		label: "Home",
		url: "https://caldav.icloud.com/",
		auth: { type: "basic", username: "ann@example.com" },
		grant: "abcd-efgh",
	});
});

test("the chosen calendar falls back to the first writable one", () => {
	const calendars = [
		{ url: "a/", name: "A", writable: false },
		{ url: "b/", name: "B", writable: true },
	];
	expect(chosenCalendar({ calendars, calendar_url: "a/" })?.name).toBe("A");
	expect(chosenCalendar({ calendars, calendar_url: null })?.name).toBe("B");
	expect(chosenCalendar({})).toBeUndefined();
});

test("a refused sign-in is recognised in the discovery message", () => {
	expect(isCalDavSignInError("PROPFIND caldav.icloud.com answered 401")).toBe(
		true,
	);
	expect(
		isCalDavSignInError("check user name and (app-specific) password"),
	).toBe(true);
	expect(isCalDavSignInError("the server could not be reached")).toBe(false);
});
