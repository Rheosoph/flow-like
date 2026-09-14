import { afterEach, describe, expect, test, vi } from "vitest";
import {
	readRecentApps,
	recordRecentApp,
	recentAppsKey,
} from "@flow-like/flow-like-ui/lib/recent-apps";

afterEach(() => vi.unstubAllGlobals());
function storage() {
	const values = new Map<string, string>();
	vi.stubGlobal("localStorage", {
		getItem: (key: string) => values.get(key) ?? null,
		setItem: (key: string, value: string) => values.set(key, value),
	});
	vi.stubGlobal("window", { dispatchEvent: vi.fn() });
	vi.stubGlobal(
		"CustomEvent",
		class {
			constructor(
				public type: string,
				public options: unknown,
			) {}
		},
	);
	return values;
}
describe("recently used apps", () => {
	test("records actual opens, reorders repeat use, and isolates account and profile scopes", () => {
		storage();
		const scope = ["https://hub.example", "profile", "account-a"];
		recordRecentApp(scope, "app-a", new Date("2026-09-10"));
		recordRecentApp(scope, "app-b", new Date("2026-09-11"));
		recordRecentApp(scope, "app-a", new Date("2026-09-12"));
		expect(readRecentApps(scope)).toEqual([
			{
				appId: "app-a",
				lastOpenedAt: "2026-09-12T00:00:00.000Z",
				openCount: 2,
			},
			{
				appId: "app-b",
				lastOpenedAt: "2026-09-11T00:00:00.000Z",
				openCount: 1,
			},
		]);
		expect(readRecentApps([scope[0], scope[1], "account-b"])).toEqual([]);
		expect(readRecentApps([scope[0], "profile-b", scope[2]])).toEqual([]);
	});
	test("corrupt local data is ignored and unavailable storage does not prevent opening", () => {
		const values = storage();
		values.set(recentAppsKey(["local"]), '{"not":"an array"}');
		expect(readRecentApps(["local"])).toEqual([]);
		vi.stubGlobal("localStorage", {
			getItem: () => {
				throw Error("blocked");
			},
			setItem: () => {
				throw Error("blocked");
			},
		});
		expect(() => recordRecentApp(["local"], "app")).not.toThrow();
	});
});
