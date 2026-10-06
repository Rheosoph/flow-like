import { describe, expect, test } from "bun:test";
import { FORM_LIMITS, type FileSlot, type SessionEffect } from "../contracts";
import { type FixtureName, fixture, fixtureClock } from "../testing/fixtures";
import {
	type SessionDriver,
	fileNames,
	focusedKey,
	pickedFiles,
	sessionDriver,
} from "./reduce-driver";

const from = (name: FixtureName) =>
	sessionDriver(fixture(name), fixtureClock(name));

const ofType = <T extends SessionEffect["type"]>(
	effects: readonly SessionEffect[],
	type: T,
) =>
	effects.filter(
		(effect): effect is Extract<SessionEffect, { type: T }> =>
			effect.type === type,
	);

const slotOf = (driver: SessionDriver, name: string) =>
	driver.state.rail.values[name] as FileSlot | null;

describe("picking (spec M3)", () => {
	test("one picked file replaces only the current one; next files stay", () => {
		const driver = from("series");
		const next = driver.state.rail.nextFiles.invoice_file;
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([{ name: "other.pdf", size: 5 }]),
			mode: "replace",
		});
		expect(slotOf(driver, "invoice_file")?.name).toBe("other.pdf");
		expect(driver.state.rail.nextFiles.invoice_file).toBe(next);
		expect(focusedKey(driver.state)).toBe("invoice_file");
	});

	test("up to 50 next files: 'Only the first 50 files were added.'", () => {
		const driver = from("idle");
		const files = Array.from(
			{ length: FORM_LIMITS.nextFiles + 3 },
			(_, index) => ({
				name: `doc-${index + 1}.pdf`,
				size: 1000 + index,
			}),
		);
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles(files),
			mode: "replace",
		});
		expect(slotOf(driver, "invoice_file")?.name).toBe("doc-1.pdf");
		const next = driver.state.rail.nextFiles.invoice_file;
		expect(next).toHaveLength(FORM_LIMITS.nextFiles);
		expect(next[0].name).toBe("doc-2.pdf");
		expect(next[8].name).toBe("doc-10.pdf");
		expect(driver.state.view.message?.message).toEqual({
			kind: "nextFilesCapped",
		});
		const released = ofType(driver.last, "releaseFiles")[0];
		expect(released?.slotIds).toEqual(["pick-doc-52.pdf", "pick-doc-53.pdf"]);
	});

	test("when no list opens, the left-out note is a dock message with 'Add it'", () => {
		const driver = from("reopen");
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([
				{ name: "invoice-RE-2026-0917.pdf", size: 1284096 },
				{ name: "invoice-RE-2026-0918.pdf", size: 1198080 },
			]),
			mode: "replace",
		});
		expect(slotOf(driver, "invoice_file")?.name).toBe(
			"invoice-RE-2026-0918.pdf",
		);
		expect(driver.state.rail.nextFiles.invoice_file).toEqual([]);
		expect(driver.state.view.list).toBeNull();
		const entry = driver.state.view.message;
		expect(entry?.message.kind).toBe("leftOut");
		expect(entry?.undo).toBe(false);
		expect(entry?.expiresAt).toBeNull();
		driver.command({ type: "addLeftOut", name: "invoice_file" });
		expect(
			driver.state.rail.nextFiles.invoice_file.map((slot) => slot.name),
		).toEqual(["invoice-RE-2026-0917.pdf"]);
		expect(slotOf(driver, "invoice_file")?.name).toBe(
			"invoice-RE-2026-0918.pdf",
		);
		expect(driver.state.rail.leftOut).toEqual({});
		expect(driver.state.view.message).toBeNull();
	});

	test("a several-files field: files join; a picked file replaces its Pick again reminder", () => {
		const driver = from("pick-again");
		driver.command({
			type: "pickFiles",
			name: "supporting_documents",
			files: pickedFiles([{ name: "PO-48213.pdf", size: 96256 }]),
			mode: "append",
		});
		const slots = driver.state.rail.values.supporting_documents as FileSlot[];
		expect(slots.map((slot) => [slot.name, slot.state])).toEqual([
			["Lieferschein LS-77120.pdf", "reminder"],
			["PO-48213.pdf", "sending"],
		]);
		driver.command({
			type: "pickFiles",
			name: "supporting_documents",
			files: pickedFiles([{ name: "x.pdf", size: 1 }], "second"),
			mode: "replace",
		});
		expect(fileNames(driver.state.rail.values.supporting_documents)).toBe(
			"x.pdf",
		);
	});

	test("above 35 MB the app host warns", () => {
		const driver = from("idle");
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([{ name: "big.pdf", size: 40 * 1024 * 1024 }]),
			mode: "replace",
		});
		expect(driver.state.view.message?.message).toEqual({
			kind: "fileWarning",
			name: "big.pdf",
		});
		expect(slotOf(driver, "invoice_file")?.name).toBe("big.pdf");
	});
});

describe("removing (spec M2, M3)", () => {
	test("⌫ on the current file: the next moves in, '{name} removed.' with Undo", () => {
		const driver = from("series");
		const current = slotOf(driver, "invoice_file") as FileSlot;
		driver.command({
			type: "removeFile",
			name: "invoice_file",
			slotId: current.id,
		});
		expect(slotOf(driver, "invoice_file")?.name).toBe(
			"invoice-RE-2026-0923.pdf",
		);
		expect(driver.state.rail.nextFiles.invoice_file).toHaveLength(4);
		expect(driver.state.view.message?.message).toEqual({
			kind: "fileRemoved",
			name: current.name,
		});
		expect(focusedKey(driver.state)).toBe("invoice_file");
		expect(ofType(driver.last, "releaseFiles")).toEqual([]);
		driver.command({ type: "undo" });
		expect(slotOf(driver, "invoice_file")?.id).toBe(current.id);
		expect(driver.state.rail.nextFiles.invoice_file).toHaveLength(5);
	});

	test("one next file, then all of them: '5 next files removed.' with Undo", () => {
		const driver = from("next-files");
		const [first] = driver.state.rail.nextFiles.invoice_file;
		driver.command({ type: "moveList", delta: 1 });
		driver.command({
			type: "removeNextFile",
			name: "invoice_file",
			slotId: first.id,
		});
		expect(driver.state.rail.nextFiles.invoice_file).toHaveLength(8);
		expect(ofType(driver.last, "abortUpload")).toEqual([
			{ type: "abortUpload", slotId: first.id },
		]);
		driver.command({ type: "clearNextFiles", name: "invoice_file" });
		expect(driver.state.rail.nextFiles.invoice_file).toEqual([]);
		expect(driver.state.view.list).toBeNull();
		expect(driver.state.view.message?.message).toEqual({
			kind: "nextFilesRemoved",
			count: 8,
		});
		driver.command({ type: "undo" });
		expect(driver.state.rail.nextFiles.invoice_file).toHaveLength(8);
	});

	test("a File nobody holds any more is released once Undo is gone", () => {
		const driver = from("idle");
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([{ name: "a.pdf", size: 1 }]),
			mode: "replace",
		});
		driver.command({
			type: "removeFile",
			name: "invoice_file",
			slotId: "pick-a.pdf",
		});
		expect(ofType(driver.last, "abortUpload")).toEqual([
			{ type: "abortUpload", slotId: "pick-a.pdf" },
		]);
		expect(ofType(driver.last, "releaseFiles")).toEqual([]);
		driver.command({ type: "setValue", key: "vendor_name", value: "x" });
		expect(ofType(driver.last, "releaseFiles")).toEqual([
			{ type: "releaseFiles", slotIds: ["pick-a.pdf"] },
		]);
	});
});

describe("uploads (spec F, M3; PLAN §3.2)", () => {
	test("progress and failure; Try again sends the file again", () => {
		const driver = from("idle");
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([{ name: "a.pdf", size: 1 }]),
			mode: "replace",
		});
		expect(driver.last).toContainEqual({
			type: "upload",
			slotId: "pick-a.pdf",
			name: "invoice_file",
			mode: "flowpath",
		});
		driver.input({
			type: "uploadProgress",
			slotId: "pick-a.pdf",
			progress: 0.5,
		});
		expect(slotOf(driver, "invoice_file")?.progress).toBe(0.5);
		driver.failUpload(
			"pick-a.pdf",
			"This file could not be prepared for the flow.",
		);
		expect(slotOf(driver, "invoice_file")).toMatchObject({
			state: "failed",
			error: "This file could not be prepared for the flow.",
		});
		driver.command({ type: "retryFile", slotId: "pick-a.pdf" });
		expect(slotOf(driver, "invoice_file")?.state).toBe("sending");
		expect(ofType(driver.last, "upload")).toHaveLength(1);
		driver.sendUploads();
		expect(slotOf(driver, "invoice_file")?.state).toBe("sent");
		expect(slotOf(driver, "invoice_file")?.ref?.kind).toBe("flowpath");
	});

	test("a press while the file sends, then the file removed from the form: the run still starts with it", () => {
		const driver = from("small");
		driver.command({ type: "setValue", key: "order", value: "1" });
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "r.jpg", size: 10 }]),
			mode: "replace",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		driver.command({
			type: "removeFile",
			name: "receipt",
			slotId: "pick-r.jpg",
		});
		expect(driver.state.rail.values.receipt).toBeNull();
		expect(ofType(driver.last, "abortUpload")).toEqual([]);
		expect(driver.openUploads.map((upload) => upload.slotId)).toEqual([
			"pick-r.jpg",
		]);
		driver.sendUploads();
		const [run] = driver.state.runs;
		expect(run.status).toBe("starting");
		expect(fileNames(run.copy.values.receipt)).toBe("r.jpg");
		expect((run.copy.values.receipt as FileSlot).state).toBe("sent");
	});

	test("a run with two files waits for both", () => {
		const driver = from("idle");
		driver.command({ type: "setValue", key: "vendor_name", value: "V" });
		driver.command({
			type: "setValue",
			key: "invoice_date",
			value: "2026-09-18",
		});
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([{ name: "i.pdf", size: 1 }]),
			mode: "replace",
		});
		driver.command({
			type: "pickFiles",
			name: "supporting_documents",
			files: pickedFiles([{ name: "d.pdf", size: 1 }]),
			mode: "append",
		});
		driver.command({ type: "run", leaveAsIs: false, from: "button" });
		expect(driver.state.runs[0].pendingSlotIds).toEqual([
			"pick-i.pdf",
			"pick-d.pdf",
		]);
		driver.sendUpload("pick-i.pdf");
		expect(driver.state.runs[0].status).toBe("sending");
		driver.sendUpload("pick-d.pdf");
		expect(driver.state.runs[0].status).toBe("starting");
	});
});

describe("what each host can send (spec F)", () => {
	test("a hosted link takes one file per run, the first by name", () => {
		const driver = from("small");
		const hosted = {
			...driver.state,
			form: {
				...driver.state.form,
				host: {
					...driver.state.form.host,
					nextFiles: false,
					uploads: "inline" as const,
				},
			},
		};
		const link = sessionDriver(hosted, fixtureClock("small"));
		link.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([
				{ name: "b.jpg", size: 1 },
				{ name: "a.jpg", size: 1 },
			]),
			mode: "replace",
		});
		expect(slotOf(link, "receipt")?.name).toBe("a.jpg");
		expect(slotOf(link, "receipt")?.state).toBe("sent");
		expect(link.state.view.message?.message).toEqual({
			kind: "oneFileOnly",
			name: "a.jpg",
		});
		expect(link.state.rail.nextFiles).toEqual({});
		expect(ofType(link.last, "releaseFiles")[0]?.slotIds).toEqual([
			"pick-b.jpg",
		]);
	});

	test("a FlowPath field this host cannot fill takes no file", () => {
		const driver = from("hosted-files");
		driver.command({
			type: "pickFiles",
			name: "invoice_file",
			files: pickedFiles([{ name: "a.pdf", size: 1 }]),
			mode: "replace",
		});
		expect(slotOf(driver, "invoice_file")).toBeNull();
		expect(driver.last).toEqual([
			{ type: "releaseFiles", slotIds: ["pick-a.pdf"] },
		]);
	});

	test("an inline host refuses a file above its limit", () => {
		const base = fixture("small");
		const host = {
			...base.form.host,
			uploads: "inline" as const,
			inlineFileLimitBytes: 1000,
			inlineRoomBytes: 5000,
		};
		const driver = sessionDriver(
			{ ...base, form: { ...base.form, host } },
			fixtureClock("small"),
		);
		driver.command({
			type: "pickFiles",
			name: "receipt",
			files: pickedFiles([{ name: "huge.jpg", size: 2000 }]),
			mode: "replace",
		});
		expect(slotOf(driver, "receipt")).toBeNull();
		expect(driver.state.view.message?.message).toEqual({
			kind: "fileRefused",
			name: "huge.jpg",
			limitBytes: 1000,
			host: "app",
		});
		expect(ofType(driver.last, "releaseFiles")[0]?.slotIds).toEqual([
			"pick-huge.jpg",
		]);
	});
});
