import { describe, expect, it } from "vitest";
import { type OnlineProfile, toLocalProfile } from "../profile-sync";

const profile: OnlineProfile = {
	id: "personal",
	name: "Personal",
	hub: "https://hub.example",
	created_at: "2026-09-05T10:00:00Z",
	updated_at: "2026-09-05T10:01:00Z",
};

describe("profile home synchronization", () => {
	it("preserves the selected default and embedded route query parameters", () => {
		const home_layout = {
			version: 1 as const,
			widgets: [
				{
					id: "embed",
					type: "app-embed",
					size: { columns: 6, rows: 4 },
					appearance: { variant: "card", accent: "neutral" },
					config: { appId: "app", path: "/reports?status=open&tag=a&tag=b" },
				},
			],
		};
		const local = toLocalProfile({
			...profile,
			home_layout,
			home_default_id: "template",
		});
		expect(local.hub_profile.home_layout).toEqual(home_layout);
		expect(local.hub_profile.home_default_id).toBe("template");
	});

	it("keeps explicit reset null while preserving the profile default association", () => {
		const local = toLocalProfile({
			...profile,
			home_layout: null,
			home_default_id: "template",
		});
		expect(local.hub_profile.home_layout).toBeNull();
		expect(local.hub_profile.home_default_id).toBe("template");
	});

	it("old profiles inherit the main default without creating a custom layout", () => {
		const local = toLocalProfile(profile);
		expect(local.hub_profile.home_layout).toBeNull();
		expect(local.hub_profile.home_default_id).toBeNull();
	});
});
