import { describe, expect, test } from "vitest";
import type { IBackendState } from "../../../state/backend-state";
import { CreatedArtifactJournal } from "../../flowpilot/board-edit-guard";
import { type CreateAppToolOptions, createAppTool } from "./app-provisioning";

describe("app provisioning recovery", () => {
	test("replays profile association after partial creation without creating another app", async () => {
		let created = 0;
		let profileWrites = 0;
		const backend = {
			appState: {
				getAppAuthoritative: async () => ({ id: "created-app" }),
				createApp: async () => {
					created++;
					return { id: "created-app" };
				},
			},
			userState: {
				getSettingsProfile: async () => ({}),
				updateProfileApp: async () => {
					profileWrites++;
					if (profileWrites === 1) throw new Error("profile unavailable");
				},
			},
		} as unknown as Pick<IBackendState, "appState" | "userState">;
		const options: CreateAppToolOptions = {
			authenticated: false,
			conversationId: "conversation",
			requestId: "request",
			journal: new CreatedArtifactJournal(60_000, 10, Date.now, null),
			assertActive() {},
			rememberTarget() {},
			referenceApp() {},
			invalidate() {},
			handleUpgrade: () => false,
		};
		expect(
			await createAppTool(backend, { name: "Intake" }, options),
		).toMatchObject({
			status: "partial",
			app_id: "created-app",
			provisioning: { app_created: true, profile_registered: false },
		});
		expect(
			await createAppTool(backend, { name: "Intake" }, options),
		).toMatchObject({
			status: "ok",
			already_created: true,
			provisioning: { app_created: true, profile_registered: true },
		});
		expect(created).toBe(1);
		expect(profileWrites).toBe(2);
	});
});
