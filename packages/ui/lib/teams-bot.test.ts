import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "./api-error";
import {
	EVENT_DEFINITIONS,
	sinkSupportsEventExecution,
} from "./event-definitions";
import { getEventGuide } from "./event-sections";
import type { IEvent } from "./schema/flow/event";
import { IEventExecutionMode } from "./schema/flow/event";
import {
	TEAMS_AUTH_MODES,
	type TeamsBotConnection,
	type TeamsBotSetup,
	isTeamsBotLocked,
	teamsFailure,
	teamsFailureDetail,
	teamsPackageFilename,
	teamsSetupChangesConnectedBot,
	teamsSetupFromConnection,
	validateTeamsSetup,
	withTeamsAuthMode,
} from "./teams-bot";

const id = "12345678-1234-1234-1234-123456789012";
const other = "22345678-1234-1234-1234-123456789012";
const input: TeamsBotSetup = {
	mode: "customer_teams",
	name: "Support",
	description: "",
	customer_tenant_id: id,
	home_tenant_id: id,
	client_id: id,
	client_secret: "secret-value",
	allowed_responders: [],
};
const saved = {
	...input,
	configured: true,
	managed_available: true,
	status: "ready",
	endpoint: "https://example.com/messages",
	connection_id: id,
	secret_expires_at: null,
} as TeamsBotConnection;

describe("Teams bot setup", () => {
	test("offers the three ownership paths in a stable order", () => {
		expect([...TEAMS_AUTH_MODES]).toEqual([
			"flow_like_managed",
			"customer_teams",
			"customer_azure",
		]);
	});

	test("accepts managed setup without customer bot credentials", () => {
		expect(
			validateTeamsSetup({
				...input,
				mode: "flow_like_managed",
				client_id: "",
				home_tenant_id: "",
				client_secret: "",
			}),
		).toEqual([]);
	});

	test("reports every invalid field so each can show its own error", () => {
		expect(validateTeamsSetup(input)).toEqual([]);
		for (const key of [
			"customer_tenant_id",
			"home_tenant_id",
			"client_id",
			"client_secret",
		] as const) {
			expect(validateTeamsSetup({ ...input, [key]: "" })).toEqual([key]);
		}
		expect(
			validateTeamsSetup({
				...input,
				name: " ",
				customer_tenant_id: "tenant",
				allowed_responders: ["person@example.com"],
			}),
		).toEqual(["name", "customer_tenant_id", "allowed_responders"]);
		expect(
			validateTeamsSetup({ ...input, description: "x".repeat(4001) }),
		).toEqual(["description"]);
	});

	test("keeps a saved secret only for the same bot identity", () => {
		expect(teamsSetupFromConnection(saved).client_secret).toBe("");
		const blank = { ...input, client_secret: "" };
		expect(
			validateTeamsSetup(blank, {
				...saved,
				configured: false,
				status: "setup_failed",
			}),
		).toEqual([]);
		expect(
			validateTeamsSetup(blank, {
				...saved,
				configured: false,
				status: "disconnected",
			}),
		).toEqual(["client_secret"]);
		expect(validateTeamsSetup(blank, saved)).toEqual([]);
		expect(validateTeamsSetup({ ...blank, client_id: other }, saved)).toEqual([
			"client_secret",
		]);
	});

	test("asks for confirmation only when a connected bot's identity or tenant changes", () => {
		const unchanged = { ...input, client_secret: "" };
		expect(teamsSetupChangesConnectedBot(unchanged, saved)).toBe(false);
		expect(
			teamsSetupChangesConnectedBot(
				{ ...unchanged, name: "Renamed", allowed_responders: [other] },
				saved,
			),
		).toBe(false);
		for (const change of [
			{ client_secret: "new-secret" },
			{ client_id: other },
			{ home_tenant_id: other },
			{ customer_tenant_id: other },
			{ mode: "customer_azure" as const },
		])
			expect(
				teamsSetupChangesConnectedBot({ ...unchanged, ...change }, saved),
			).toBe(true);
		expect(
			teamsSetupChangesConnectedBot(input, {
				...saved,
				status: "setup_failed",
			}),
		).toBe(false);
		expect(
			teamsSetupChangesConnectedBot(
				{ ...unchanged, mode: "flow_like_managed", client_id: "" },
				{ ...saved, mode: "flow_like_managed", client_id: "" },
			),
		).toBe(false);
	});

	test("switching to the managed path clears customer credentials", () => {
		expect(withTeamsAuthMode(input, "flow_like_managed")).toEqual({
			...input,
			mode: "flow_like_managed",
			home_tenant_id: "",
			client_id: "",
			client_secret: "",
		});
		expect(withTeamsAuthMode(input, "customer_azure")).toEqual({
			...input,
			mode: "customer_azure",
		});
	});

	test("locks the management path while a bot exists", () => {
		expect(isTeamsBotLocked(undefined)).toBe(false);
		expect(isTeamsBotLocked(saved)).toBe(true);
		expect(isTeamsBotLocked({ ...saved, status: "disconnected" })).toBe(false);
		expect(isTeamsBotLocked({ ...saved, status: "not_configured" })).toBe(
			false,
		);
	});

	test("uses the backend package filename", () => {
		expect(
			teamsPackageFilename({ filename: "support-bot.zip", base64: "" }),
		).toBe("support-bot.zip");
		expect(teamsPackageFilename({ filename: "support-bot", base64: "" })).toBe(
			"support-bot.zip",
		);
		expect(teamsPackageFilename({ filename: " ", base64: "" })).toBe(
			"teams-app.zip",
		);
	});

	test("classifies API refusals by status and keeps the server's detail", () => {
		const refusal = (status: number) =>
			new ApiResponseError({ status, code: "X", message: `detail ${status}` });
		expect(teamsFailure(refusal(403), "setup")).toBe("forbidden");
		expect(teamsFailure(refusal(409), "rotate")).toBe("busy");
		expect(teamsFailure(refusal(422), "setup")).toBe("managed_limit");
		expect(teamsFailure(refusal(429), "setup")).toBe("rate_limited");
		expect(teamsFailure(refusal(502), "disconnect")).toBe("unavailable");
		expect(teamsFailure(refusal(400), "setup", "flow_like_managed")).toBe(
			"managed_rejected",
		);
		expect(teamsFailure(refusal(400), "setup", "customer_teams")).toBe(
			"rejected",
		);
		expect(teamsFailure(refusal(400), "download")).toBe("unknown");
		expect(teamsFailure(new Error("offline"), "setup")).toBe("unknown");
		expect(teamsFailureDetail(refusal(429))).toBe("detail 429");
		expect(teamsFailureDetail(new Error("offline"))).toBe("offline");
	});

	test("uses the Chat Event and cannot execute locally", () => {
		const definition = EVENT_DEFINITIONS.events_chat;
		expect(definition.eventTypes).toContain("teams");
		expect(definition.configs.teams).toEqual({ sink_type: "teams" });
		const availability = definition.sinkAvailability?.teams;
		expect(
			sinkSupportsEventExecution(
				availability,
				IEventExecutionMode.Remote,
				true,
			),
		).toBe(true);
		expect(
			sinkSupportsEventExecution(availability, IEventExecutionMode.Local, true),
		).toBe(false);
	});

	test("the guide binds the flow and ticks activation like inbound email", () => {
		const event = { event_type: "teams", active: true } as IEvent;
		const steps = getEventGuide(event);
		const ids = steps.map((step) => step.id);
		expect(ids).toContain("bind-flow");
		expect(ids).toContain("teams-connect");
		const activate = steps.find((step) => step.id === "activate");
		expect(activate?.auto?.({}, event)).toBe(true);
		expect(activate?.auto?.({}, { ...event, active: false })).toBe(false);
	});
});
