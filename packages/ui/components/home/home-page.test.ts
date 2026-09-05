import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("./home-editor", () => ({ HomeEditor: () => null }));
vi.mock("./catalog", () => ({ createDefaultHomeLayout: vi.fn() }));
vi.mock("../../state/backend-state", () => ({
	useBackend: vi.fn(),
	useBackendReady: vi.fn(),
}));
vi.mock("../../hooks/use-invoke", () => ({ useInvoke: vi.fn() }));

import {
	homeDefaultsCacheKey,
	homeDefaultsQueryKey,
	readCachedHomeDefaults,
} from "./home-page";

afterEach(() => vi.unstubAllGlobals());

describe("home default cache ownership", () => {
	it("separates the same profile ID across accounts, hubs and default targets", () => {
		const values = [
			["https://one.example", "user:alice", "shared-id", "main"],
			["https://one.example", "user:bob", "shared-id", "main"],
			["http://one.example", "user:alice", "shared-id", "main"],
			["https://one.example", "user:alice", "other-id", "main"],
			["https://one.example", "user:alice", "shared-id", "template"],
		] as const;
		expect(
			new Set(
				values.map(([origin, viewer, profile, target]) =>
					homeDefaultsCacheKey(origin, viewer, profile, target),
				),
			).size,
		).toBe(values.length);
		expect(
			new Set(
				values.map(([origin, viewer, profile, target]) =>
					JSON.stringify(homeDefaultsQueryKey(origin, viewer, profile, target)),
				),
			).size,
		).toBe(values.length);
	});

	it("does not treat a corrupt cache record as a published layout", () => {
		vi.stubGlobal("localStorage", {
			getItem: () =>
				JSON.stringify({
					main: { id: "main", revision: "r1", layout: {} },
					profile: null,
				}),
		});
		expect(readCachedHomeDefaults("key")).toBeUndefined();
	});

	it("retains an explicit empty default without replacing it with the bundle", () => {
		const saved = {
			main: { id: "main", revision: "r1", layout: { version: 1, widgets: [] } },
			profile: null,
		};
		vi.stubGlobal("localStorage", { getItem: () => JSON.stringify(saved) });
		expect(readCachedHomeDefaults("key")).toEqual(saved);
	});
});
