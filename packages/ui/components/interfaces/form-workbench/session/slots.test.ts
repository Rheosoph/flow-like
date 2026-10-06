import { describe, expect, test } from "bun:test";
import type { FileSlot, FormSessionState, SessionEffect } from "../contracts";
import { fixture, fixtureClock } from "../testing/fixtures";
import { createTx } from "./reduce-tx";
import {
	mapSlots,
	pendingSlotIds,
	scheduleUploads,
	settleSlots,
	slotHolders,
	uploadNeeds,
} from "./slots";

const ids = (slots: readonly FileSlot[]) => slots.map((slot) => slot.id);
const types = (effects: readonly SessionEffect[]) =>
	effects.map((effect) => effect.type);

function txOf(state: FormSessionState) {
	return createTx(state, fixtureClock("next-files"));
}

describe("slotHolders", () => {
	test("the rail, next files, left-out files, this session's runs and Undo hold Files", () => {
		const state = fixture("next-files");
		const held = slotHolders(state);
		expect(held.has("slot-invoice-0918")).toBe(true);
		expect(held.has("slot-invoice-0927")).toBe(true);
		expect(held.has("slot-pick-0917")).toBe(true);
		const withUndo: FormSessionState = {
			...state,
			rail: {
				...state.rail,
				values: { ...state.rail.values, invoice_file: null },
			},
			undo: {
				seq: 9,
				kind: "fileRemoved",
				values: state.rail.values,
				nextFiles: {},
				activePresetId: null,
				deletedPreset: null,
			},
		};
		expect(slotHolders(withUndo).has("slot-invoice-0918")).toBe(true);
	});

	test("reminders and saved runs hold no File", () => {
		const held = slotHolders(fixture("pick-again"));
		for (const id of held) expect(id.startsWith("history-")).toBe(false);
	});

	test("a run of this session holds its files after it ended", () => {
		const held = slotHolders(fixture("done"));
		expect(held.has("slot-invoice-0917")).toBe(true);
	});
});

describe("uploadNeeds", () => {
	test("files of waiting runs first, then the rail in form order, then next files in list order", () => {
		const state = fixture("next-files");
		const order = [...uploadNeeds(state).keys()];
		expect(order[0]).toBe("slot-invoice-0918");
		expect(order.slice(1, 4)).toEqual([
			"slot-invoice-0919",
			"slot-invoice-0920",
			"slot-invoice-0921",
		]);
		expect(order).not.toContain("slot-pick-0917");
	});

	test("a sending run needs its files; an ended run does not", () => {
		const uploading = fixture("uploading");
		expect([...uploadNeeds(uploading).keys()]).toContain("slot-ls-77120");
		const done = fixture("done");
		expect([...uploadNeeds(done).keys()]).toContain("slot-invoice-0917");
		const ended: FormSessionState = {
			...done,
			rail: {
				...done.rail,
				values: { ...done.rail.values, invoice_file: null },
			},
		};
		expect([...uploadNeeds(ended).keys()]).not.toContain("slot-invoice-0917");
	});
});

describe("settleSlots", () => {
	test("an upload nothing needs is aborted and waits again; a File nothing holds is released", () => {
		const before = fixture("next-files");
		const after: FormSessionState = {
			...before,
			rail: { ...before.rail, nextFiles: {}, leftOut: {} },
		};
		const tx = txOf(before);
		const settled = settleSlots(before, after, tx);
		expect(tx.effects).toContainEqual({
			type: "abortUpload",
			slotId: "slot-invoice-0919",
		});
		const released = tx.effects.find(
			(effect) => effect.type === "releaseFiles",
		);
		expect(released?.type === "releaseFiles" && released.slotIds).toContain(
			"slot-invoice-0927",
		);
		expect(released?.type === "releaseFiles" && released.slotIds).toContain(
			"slot-pick-0917",
		);
		expect(settled).toBe(after);
	});

	test("a slot still held elsewhere goes back to waiting instead of being released", () => {
		const before = fixture("next-files");
		const current = before.rail.values.invoice_file as FileSlot;
		const after: FormSessionState = {
			...before,
			rail: {
				...before.rail,
				values: { ...before.rail.values, invoice_file: null },
			},
			undo: {
				seq: 3,
				kind: "fileRemoved",
				values: before.rail.values,
				nextFiles: before.rail.nextFiles,
				activePresetId: null,
				deletedPreset: null,
			},
		};
		const tx = txOf(before);
		const settled = settleSlots(before, after, tx);
		expect(types(tx.effects)).toEqual(["abortUpload"]);
		const kept = settled.undo?.values.invoice_file as FileSlot;
		expect(kept.id).toBe(current.id);
		expect(kept.state).toBe("waiting");
		expect(kept.progress).toBeNull();
	});

	test("files picked in the step and kept by nobody are released at once", () => {
		const state = fixture("idle");
		const tx = txOf(state);
		tx.picked.push("refused-1");
		settleSlots(state, state, tx);
		expect(tx.effects).toEqual([
			{ type: "releaseFiles", slotIds: ["refused-1"] },
		]);
	});
});

describe("scheduleUploads", () => {
	test("at most two in flight, in upload order", () => {
		const base = fixture("next-files");
		const waiting = mapSlots(base, (slot) =>
			slot.state === "sending"
				? { ...slot, state: "waiting", progress: null }
				: slot,
		);
		const tx = txOf(waiting);
		const next = scheduleUploads(waiting, tx);
		expect(tx.effects).toEqual([
			{
				type: "upload",
				slotId: "slot-invoice-0918",
				name: "invoice_file",
				mode: "flowpath",
			},
			{
				type: "upload",
				slotId: "slot-invoice-0919",
				name: "invoice_file",
				mode: "flowpath",
			},
		]);
		expect((next.rail.values.invoice_file as FileSlot).state).toBe("sending");
		expect((next.rail.values.invoice_file as FileSlot).progress).toBe(0);
	});

	test("nothing starts while two are in flight, or on a host that sends files inline", () => {
		const busy = fixture("next-files");
		const tx = txOf(busy);
		expect(scheduleUploads(busy, tx)).toBe(busy);
		expect(tx.effects).toEqual([]);
		const hosted = fixture("hosted-files");
		expect(scheduleUploads(hosted, txOf(hosted))).toBe(hosted);
	});
});

describe("mapSlots", () => {
	test("keeps the state when no slot changes", () => {
		const state = fixture("series");
		expect(mapSlots(state, (slot) => slot)).toBe(state);
	});

	test("updates a slot in the rail, next files, runs and Undo alike", () => {
		const state = fixture("uploading");
		const next = mapSlots(state, (slot) =>
			slot.id === "slot-ls-77120" ? { ...slot, progress: 0.9 } : slot,
		);
		const rail = next.rail.values.supporting_documents as readonly FileSlot[];
		const copy = next.runs[0].copy.values
			.supporting_documents as readonly FileSlot[];
		expect(rail[0].progress).toBe(0.9);
		expect(copy[0].progress).toBe(0.9);
		expect(ids(rail)).toEqual(["slot-ls-77120", "slot-po-48213"]);
	});
});

describe("pendingSlotIds", () => {
	test("files of a copy that are not sent yet", () => {
		const state = fixture("uploading");
		expect(pendingSlotIds(state.form.fields, state.rail.values)).toEqual([
			"slot-ls-77120",
			"slot-po-48213",
		]);
	});
});
