import type { IBoardListing, IEvent } from "@flow-like/flow-like-ui";
import { IAppType } from "@flow-like/flow-like-ui";
import {
	appTypeMeta,
	detectAppType,
} from "@flow-like/flow-like-ui/lib/app-type";
import { describe, expect, it } from "vitest";

function board(nodeCount: number): IBoardListing {
	return { id: "b", name: "Board", description: "", nodeCount };
}

function event(type: string, active = true): IEvent {
	return { id: `e-${type}`, event_type: type, active } as unknown as IEvent;
}

describe("detectAppType", () => {
	it("returns null for an empty project rather than guessing", () => {
		expect(detectAppType([], [], 0)).toBeNull();
		expect(detectAppType(undefined, undefined, 0)).toBeNull();
		expect(detectAppType([board(0)], [], 0)).toBeNull();
	});

	it("calls a chat or mail trigger an Agent", () => {
		expect(detectAppType([board(3)], [event("simple_chat")], 0)).toBe(
			IAppType.Agent,
		);
		expect(detectAppType([board(3)], [event("email")], 0)).toBe(IAppType.Agent);
		expect(detectAppType([board(3)], [event("telegram")], 0)).toBe(
			IAppType.Agent,
		);
	});

	it("prefers Form over Agent when a form trigger exists", () => {
		expect(
			detectAppType(
				[board(3)],
				[event("simple_chat"), event("generic_form")],
				0,
			),
		).toBe(IAppType.Form);
	});

	it("calls pages without a conversational trigger a Custom Interface", () => {
		expect(detectAppType([board(3)], [], 2)).toBe(IAppType.CustomInterface);
	});

	it("calls scheduled or API-only work a Data Pipeline", () => {
		expect(detectAppType([board(3)], [event("cron")], 0)).toBe(
			IAppType.DataPipeline,
		);
		expect(detectAppType([board(3)], [event("rest")], 0)).toBe(
			IAppType.DataPipeline,
		);
	});

	it("ignores paused triggers when classifying", () => {
		// Only a paused chat trigger — not enough to call it an Agent, but the
		// pages still make it an interface.
		expect(detectAppType([board(3)], [event("simple_chat", false)], 1)).toBe(
			IAppType.CustomInterface,
		);
	});

	it("falls back to Data Focus when only tables exist", () => {
		expect(detectAppType([], [], 0, 4)).toBe(IAppType.DataFocus);
	});

	it("returns null when logic exists but nothing indicates a shape", () => {
		expect(detectAppType([board(5)], [event("quick_action")], 0)).toBeNull();
	});
});

describe("appTypeMeta", () => {
	it("falls back to Unclassified for null and unknown values", () => {
		expect(appTypeMeta(null).label).toBe("Unclassified");
		expect(appTypeMeta(undefined).label).toBe("Unclassified");
		expect(appTypeMeta("Nonsense" as IAppType).label).toBe("Unclassified");
	});

	it("gives every type a distinct silhouette", () => {
		const shapes = Object.values(IAppType).map((type) =>
			JSON.stringify(appTypeMeta(type).shape),
		);
		expect(new Set(shapes).size).toBe(shapes.length);
	});

	it("keeps the unclassified silhouette distinct from every real type", () => {
		const unclassified = JSON.stringify(appTypeMeta(null).shape);
		for (const type of Object.values(IAppType)) {
			expect(JSON.stringify(appTypeMeta(type).shape)).not.toBe(unclassified);
		}
	});
});
