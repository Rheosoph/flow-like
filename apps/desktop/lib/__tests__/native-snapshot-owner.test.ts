import { beforeEach, expect, test, vi } from "vitest";

const state = vi.hoisted(() => ({ backend: null as any }));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackendStore: { getState: () => state },
}));
import {
	nativeSnapshotOwner,
	matchesNativeSnapshotOwner,
} from "../native-snapshot-owner";

beforeEach(() => {
	state.backend = {
		profile: { id: "one", hub: "HTTPS://Hub.Example/", secure: true },
		auth: { user: { profile: { sub: "alice", iss: "issuer" } } },
	};
});

test("native ownership compares values independent of serialized property order", () => {
	expect(
		matchesNativeSnapshotOwner({
			hub: "https://hub.example",
			issuer: "issuer",
			profileId: "one",
			subject: "alice",
		}),
	).toBe(true);
});

test("profile and account changes invalidate previously captured editor ownership", () => {
	const captured = nativeSnapshotOwner();
	state.backend.profile.id = "two";
	expect(matchesNativeSnapshotOwner(captured)).toBe(false);
	state.backend.profile.id = "one";
	state.backend.auth.user.profile.sub = "bob";
	expect(matchesNativeSnapshotOwner(captured)).toBe(false);
});

test("offline drafts have an explicit anonymous owner and preserve the configured hub scheme", () => {
	state.backend = {
		profile: { id: "offline", hub: "localhost:9000", secure: false },
		auth: { user: null },
	};
	expect(nativeSnapshotOwner()).toEqual({
		profileId: "offline",
		hub: "http://localhost:9000",
		subject: null,
		issuer: null,
	});
	state.backend = null;
	expect(nativeSnapshotOwner()).toBeUndefined();
});
