import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "../../../lib/api-error";
import { type IApp, IAppVisibility } from "../../../lib/schema/app/app";
import type { IMetadata } from "../../../lib/schema/bit/bit";
import {
	APP_REFUSES_BOTS,
	canToggleApp,
	libraryAppOptions,
	refusesBots,
} from "./app-options";

function entry(
	id: string,
	visibility: IAppVisibility,
	refuseBots?: boolean,
): [IApp, IMetadata] {
	return [
		{ id, visibility, refuse_bots: refuseBots } as IApp,
		{ name: id.toUpperCase() } as IMetadata,
	];
}

describe("apps a bot can be given", () => {
	const library = [
		entry("crm", IAppVisibility.Private, true),
		entry("notes", IAppVisibility.Private),
		entry("draft", IAppVisibility.Offline, true),
	];

	test("an account bot sees the hub's apps, each marked when it refuses bots", () => {
		expect(libraryAppOptions(library, false)).toEqual([
			{ id: "crm", name: "CRM", refusesBots: true },
			{ id: "notes", name: "NOTES", refusesBots: false },
		]);
	});

	test("a bot on this device also sees local apps, whose own setting counts", () => {
		expect(
			libraryAppOptions(library, true).find((app) => app.id === "draft"),
		).toEqual({ id: "draft", name: "DRAFT", refusesBots: true });
	});

	test("an app that refuses bots can be taken away, never given", () => {
		const [crm, notes] = libraryAppOptions(library, false);
		expect(canToggleApp(crm, false)).toBe(false);
		expect(canToggleApp(crm, true)).toBe(true);
		expect(canToggleApp(notes, false)).toBe(true);
	});
});

describe("refusesBots", () => {
	test("knows the refusal from the hub and from this device's bot store", () => {
		const hub = new ApiResponseError({
			status: 403,
			code: APP_REFUSES_BOTS,
			message: "The app 'CRM' does not allow bots; ask its owner.",
		});
		const device = Object.assign(new Error("Could not attach app crm"), {
			code: APP_REFUSES_BOTS,
		});
		expect(refusesBots(hub)).toBe(true);
		expect(refusesBots(device)).toBe(true);
		expect(refusesBots(new Error("offline"))).toBe(false);
		expect(refusesBots(undefined)).toBe(false);
	});
});
