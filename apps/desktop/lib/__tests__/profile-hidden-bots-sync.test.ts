import { describe, expect, it } from "vitest";
import {
	type OnlineProfile,
	mergeRemoteProfileMetadata,
	toLocalProfile,
} from "../profile-sync";

const profile: OnlineProfile = {
	id: "work",
	name: "Work",
	hub: "https://hub.example",
	created_at: "2026-11-11T10:00:00Z",
	updated_at: "2026-11-11T10:01:00Z",
};

describe("profile hidden bots synchronization", () => {
	it("creates a local profile with the bots the hub hides, and none for a hub without the field", () => {
		expect(
			toLocalProfile({ ...profile, hidden_bots: ["bot-a", "general"] })
				.hub_profile.hidden_bots,
		).toEqual(["bot-a", "general"]);
		expect(toLocalProfile(profile).hub_profile.hidden_bots).toEqual([]);
	});

	it("takes another device's change, including showing every bot again", () => {
		const local = toLocalProfile({ ...profile, hidden_bots: ["bot-a"] });
		mergeRemoteProfileMetadata(local, {
			...profile,
			hidden_bots: ["bot-a", "bot-b"],
		});
		expect(local.hub_profile.hidden_bots).toEqual(["bot-a", "bot-b"]);
		mergeRemoteProfileMetadata(local, { ...profile, hidden_bots: [] });
		expect(local.hub_profile.hidden_bots).toEqual([]);
		mergeRemoteProfileMetadata(local, { ...profile, hidden_bots: null });
		expect(local.hub_profile.hidden_bots).toEqual([]);
	});

	it("keeps the bots hidden here when the hub's answer omits the field", () => {
		const local = toLocalProfile({ ...profile, hidden_bots: ["bot-a"] });
		mergeRemoteProfileMetadata(local, {
			...profile,
			name: "Renamed",
			updated_at: "2026-11-11T10:02:00Z",
		});
		expect(local.hub_profile.name).toBe("Renamed");
		expect(local.hub_profile.hidden_bots).toEqual(["bot-a"]);
	});
});
