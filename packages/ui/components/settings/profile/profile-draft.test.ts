import { describe, expect, test } from "bun:test";
import {
	IConnectionMode,
	type ISettingsProfile,
	IThemes,
} from "../../../types";
import {
	ProfileDraftController,
	profileDraftSession,
	releaseProfileDraftSession,
} from "./profile-draft";
import { profileSettingsPatch } from "./profile-settings-request";
import {
	parseProfileTheme,
	profileThemeCss,
	themeSelection,
} from "./profile-theme";

const pause = (ms = 25) => new Promise((resolve) => setTimeout(resolve, ms));
const profile = (id = "personal"): ISettingsProfile => ({
	hub_profile: {
		id,
		name: "Original",
		bits: [],
		created: "2026-09-05T10:00:00Z",
		updated: "2026-09-05T10:00:00Z",
	},
	execution_settings: { gpu_mode: true, max_context_size: 32000 },
	created: "2026-09-05T10:00:00Z",
	updated: "2026-09-05T10:00:00Z",
});
const rename = (draft: ProfileDraftController, name: string) =>
	draft.update({
		hub_profile: {
			...profile().hub_profile,
			...draft.getSnapshot().profile?.hub_profile,
			name,
		},
	});

describe("workspace profile autosave", () => {
	test("a failed navigation save is recovered only in the same account and hub session", async () => {
		const key = "web:test-hub:account-one";
		const first = profileDraftSession(key, async () => {
			throw new Error("Offline");
		});
		first.setSource(profile());
		rename(first, "Recover this draft");
		await first.flushAll();
		releaseProfileDraftSession(key, first);
		const otherAccount = profileDraftSession(
			"web:test-hub:account-two",
			async () => {},
		);
		otherAccount.setSource(profile());
		expect(otherAccount.getSnapshot().profile?.hub_profile.name).toBe(
			"Original",
		);
		const writes: string[] = [];
		const returned = profileDraftSession(key, async (value) => {
			writes.push(value.hub_profile.name);
		});
		returned.setSource(profile());
		expect(returned.getSnapshot().profile?.hub_profile.name).toBe(
			"Recover this draft",
		);
		await returned.flush();
		expect(writes).toEqual(["Recover this draft"]);
		releaseProfileDraftSession(key, returned);
		releaseProfileDraftSession("web:test-hub:account-two", otherAccount);
	});

	test("debounces the edited value without writing the previous profile", async () => {
		const writes: ISettingsProfile[] = [];
		const draft = new ProfileDraftController(async (value) => {
			writes.push(value);
		}, 10);
		draft.setSource(profile());
		rename(draft, "Changed");
		expect(writes).toHaveLength(0);
		expect(draft.getSnapshot().status).toBe("pending");
		await pause();
		expect(writes.map((value) => value.hub_profile.name)).toEqual(["Changed"]);
		expect(draft.getSnapshot().status).toBe("saved");
	});

	test("serializes a newer edit behind an outstanding save", async () => {
		const writes: string[] = [];
		let finishFirst: () => void = () => {};
		const first = new Promise<void>((resolve) => {
			finishFirst = resolve;
		});
		const draft = new ProfileDraftController(async (value) => {
			writes.push(value.hub_profile.name);
			if (writes.length === 1) await first;
		}, 10);
		draft.setSource(profile());
		rename(draft, "First edit");
		await pause();
		rename(draft, "Latest edit");
		await pause();
		expect(writes).toEqual(["First edit"]);
		expect(draft.getSnapshot().status).toBe("saving");
		finishFirst();
		await draft.flush();
		expect(writes).toEqual(["First edit", "Latest edit"]);
		expect(draft.getSnapshot().status).toBe("saved");
	});

	test("refetches and profile switches preserve independent unsaved drafts", async () => {
		const writes: string[] = [];
		const draft = new ProfileDraftController(async (value) => {
			writes.push(`${value.hub_profile.id}:${value.hub_profile.name}`);
		}, 1000);
		draft.setSource(profile("one"));
		rename(draft, "First draft");
		draft.setSource(profile("two"));
		rename(draft, "Second draft");
		draft.setSource(profile("one"));
		expect(draft.getSnapshot().profile?.hub_profile.name).toBe("First draft");
		await draft.flush("one");
		await draft.flush("two");
		expect(writes).toEqual(["one:First draft", "two:Second draft"]);
	});

	test("retains a failed edit and allows an explicit retry", async () => {
		let failing = true;
		const writes: string[] = [];
		const draft = new ProfileDraftController(async (value) => {
			if (failing) throw new Error("Connection lost");
			writes.push(value.hub_profile.name);
		}, 1000);
		draft.setSource(profile());
		rename(draft, "Kept draft");
		await expect(draft.flush()).rejects.toThrow("Connection lost");
		draft.setSource(profile());
		expect(draft.getSnapshot()).toMatchObject({
			status: "error",
			error: "Connection lost",
			profile: { hub_profile: { name: "Kept draft" } },
		});
		failing = false;
		await draft.flush();
		expect(writes).toEqual(["Kept draft"]);
		expect(draft.hasUnsaved()).toBe(false);
	});

	test("preserves the runtime's zero value for its default context limit", async () => {
		const writes: ISettingsProfile[] = [];
		const draft = new ProfileDraftController(async (value) => {
			writes.push(value);
		}, 1000);
		draft.setSource(profile());
		draft.update({
			execution_settings: { gpu_mode: false, max_context_size: 0 },
		});
		await draft.flush();
		expect(writes[0].execution_settings).toEqual({
			gpu_mode: false,
			max_context_size: 0,
		});
	});

	test("blocks invalid fields without replacing the saved profile", async () => {
		const writes: unknown[] = [];
		const draft = new ProfileDraftController(async (value) => {
			writes.push(value);
		}, 10);
		draft.setSource(profile());
		rename(draft, " ");
		await expect(draft.flush()).rejects.toThrow("Enter a profile name");
		rename(draft, "Valid name");
		draft.update({
			execution_settings: { gpu_mode: true, max_context_size: 1.5 },
		});
		await expect(draft.flush()).rejects.toThrow("whole number");
		await pause();
		expect(writes).toHaveLength(0);
	});

	test("navigation flushes pending edits and deletion cancels abandoned timers", async () => {
		const writes: string[] = [];
		const draft = new ProfileDraftController(async (value) => {
			writes.push(value.hub_profile.name);
		}, 10);
		draft.setSource(profile());
		rename(draft, "Navigation draft");
		draft.flushAll();
		await pause();
		expect(writes).toEqual(["Navigation draft"]);
		rename(draft, "Deleted draft");
		draft.forget("personal");
		await pause();
		expect(writes).toEqual(["Navigation draft"]);
	});
});

describe("profile settings request and theme round trips", () => {
	test("sends editor settings, excludes device settings and app membership, and explicitly clears the default theme", () => {
		const source = profile();
		source.hub_profile.settings = { connection_mode: IConnectionMode.Straight };
		source.hub_profile.apps = [
			{ app_id: "app", favorite: false, pinned: false },
		];
		const request = JSON.parse(JSON.stringify(profileSettingsPatch(source)));
		expect(request.settings).toEqual({ connection_mode: "straight" });
		expect(request.theme).toBeNull();
		expect(request).not.toHaveProperty("execution_settings");
		expect(request).not.toHaveProperty("apps");
	});

	test("custom themes reopen with the custom option and editable original colors", () => {
		const custom = {
			id: "My colors",
			light: { background: "#ffffff", primaryForeground: "#eeeeee" },
			dark: { background: "#111111" },
		};
		expect(themeSelection(custom)).toBe("CUSTOM");
		expect(parseProfileTheme(profileThemeCss(custom), custom.id)).toEqual(
			custom,
		);
		expect(themeSelection(null)).toBe(IThemes.FLOW_LIKE);
	});

	test("rejects incomplete and misleading custom theme imports", () => {
		expect(() =>
			parseProfileTheme(":root { --background: #fff; }", "Mine"),
		).toThrow("both");
		expect(() =>
			parseProfileTheme(
				":root { --background: #fff; } .dark { --background: #000; }",
				IThemes.FLOW_LIKE,
			),
		).toThrow("different");
	});
});
