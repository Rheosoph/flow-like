import { IConnectionMode } from "@flow-like/flow-like-ui/types";
import { describe, expect, it } from "vitest";
import {
	type OnlineProfile,
	mergeRemoteProfileMetadata,
} from "../profile-sync";

const localRevision = "2026-09-05T10:00:00.000Z";
const remote: OnlineProfile = {
	id: "workspace",
	name: "Updated workspace",
	hub: "hub.example.invalid",
	created_at: localRevision,
	updated_at: "2026-09-05T10:00:01.000Z",
	icon: "https://media.example.invalid/previous.webp",
	home_default_id: "team-default",
};

function pendingProfile() {
	return {
		hub_profile: {
			id: "workspace",
			name: "Workspace",
			hub: remote.hub,
			bits: [],
			settings: { connection_mode: IConnectionMode.Simplebezier },
			icon: "/cache/replacement.png",
			created: localRevision,
			updated: localRevision,
		},
		execution_settings: { gpu_mode: false, max_context_size: 8192 },
		updated: localRevision,
	};
}

describe("profile image sync retries", () => {
	it("keeps the failed upload revision and local image while pulling metadata", () => {
		const local = pendingProfile();
		mergeRemoteProfileMetadata(local, remote, true);
		expect(local.hub_profile.updated).toBe(localRevision);
		expect(local.updated).toBe(localRevision);
		expect(local.hub_profile.icon).toBe("/cache/replacement.png");
		expect(local.hub_profile.name).toBe("Updated workspace");
		expect(local.execution_settings).toEqual({
			gpu_mode: false,
			max_context_size: 8192,
		});
	});

	it("advances the revision after a subsequent upload succeeds", () => {
		const local = pendingProfile();
		mergeRemoteProfileMetadata(local, remote, true);
		mergeRemoteProfileMetadata(local, {
			...remote,
			updated_at: "2026-09-05T10:01:00.000Z",
		});
		expect(local.hub_profile.updated).toBe("2026-09-05T10:01:00.000Z");
		expect(local.updated).toBe(local.hub_profile.updated);
	});
});
