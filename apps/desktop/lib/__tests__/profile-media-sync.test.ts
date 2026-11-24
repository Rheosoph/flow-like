import { IConnectionMode } from "@flow-like/flow-like-ui/types";
import { describe, expect, it } from "vitest";
import {
	type OnlineProfile,
	mergeRemoteProfileMetadata,
	signedUrlExpiresAt,
	storedSignedImageIsCurrent,
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

describe("signed profile images", () => {
	const s3 = (date: string) =>
		`https://bucket.s3.eu-west-1.amazonaws.com/media/users/u/icon.webp?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Date=${date}&X-Amz-Expires=300&X-Amz-Signature=abc`;
	const signedAt = Date.UTC(2026, 10, 23, 12, 0, 0);

	it("reads the expiry of S3, GCS and Azure signatures", () => {
		expect(signedUrlExpiresAt(s3("20261123T120000Z"))).toBe(signedAt + 300_000);
		expect(
			signedUrlExpiresAt(
				"https://storage.googleapis.com/b/icon.webp?X-Goog-Date=20261123T120000Z&X-Goog-Expires=600",
			),
		).toBe(signedAt + 600_000);
		expect(
			signedUrlExpiresAt(
				"https://a.blob.core.windows.net/c/icon.webp?sv=2022&se=2026-11-23T12%3A05%3A00Z&sig=x",
			),
		).toBe(signedAt + 300_000);
		expect(signedUrlExpiresAt("https://cdn.example.invalid/icon.webp")).toBe(
			undefined,
		);
		expect(signedUrlExpiresAt("/cache/icon.png")).toBe(undefined);
	});

	it("keeps a stored URL for the same image until its signature runs low", () => {
		const stored = s3("20261123T120000Z");
		const renewed = s3("20261123T120100Z");
		expect(storedSignedImageIsCurrent(stored, renewed, signedAt + 60_000)).toBe(
			true,
		);
		expect(
			storedSignedImageIsCurrent(stored, renewed, signedAt + 180_000),
		).toBe(false);
	});

	it("replaces a stored URL for another image or without a readable expiry", () => {
		const stored = s3("20261123T120000Z");
		expect(
			storedSignedImageIsCurrent(
				stored,
				stored.replace("icon.webp", "other.webp"),
				signedAt,
			),
		).toBe(false);
		expect(
			storedSignedImageIsCurrent(
				"https://cdn.example.invalid/icon.webp",
				"https://cdn.example.invalid/icon.webp",
				signedAt,
			),
		).toBe(false);
		expect(storedSignedImageIsCurrent(undefined, stored, signedAt)).toBe(false);
	});
});
