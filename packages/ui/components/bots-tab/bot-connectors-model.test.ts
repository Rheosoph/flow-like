import { expect, test } from "bun:test";
import {
	accountPolicy,
	isDrifted,
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
