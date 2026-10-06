import { describe, expect, test } from "bun:test";
import type {
	FileSlot,
	RunEntry,
	StoredRunRecord,
	WorkbenchField,
} from "../contracts";
import { repeatBlocker } from "../run/run-view";
import { fixture, fixtureForm } from "../testing/fixtures";
import {
	SESSION_SCOPE,
	entryOf,
	memoryScopeOf,
	mergeHistory,
	nextNumberAfter,
	recordOf,
	storedStatusOf,
} from "./history";

const form = fixtureForm("medium");

/** Run 14 of the Done artboard: a run of this session with sent files. */
const run14 = () => fixture("done").runs[0];

function withStatus(run: RunEntry, status: RunEntry["status"]): RunEntry {
	return { ...run, status };
}

describe("recordOf", () => {
	test("files are saved as names and sizes, never as slots", () => {
		const record = recordOf(run14(), form, "profile:local");
		expect(record.version).toBe(2);
		expect(record.scope).toBe("profile:local");
		expect(record.appId).toBe(form.appId);
		expect(record.eventId).toBe(form.eventId);
		expect(record.inputs.invoice_file).toEqual({
			$file: { name: "invoice-RE-2026-0917.pdf", size: 1284096 },
		});
		expect(record.inputs.supporting_documents).toEqual([
			{ $file: { name: "Lieferschein LS-77120.pdf", size: 318464 } },
			{ $file: { name: "PO-48213.pdf", size: 96256 } },
		]);
		expect(record.inputs.vendor_name).toBe("Nordwind Logistik GmbH");
		expect(record.status).toBe("done");
	});

	test("every status between dispatch and the end is saved as running", () => {
		const run = run14();
		const statuses = ["starting", "asking", "running", "streaming"] as const;
		for (const status of statuses)
			expect(recordOf(withStatus(run, status), form, "s").status).toBe(
				"running",
			);
		expect(storedStatusOf("sending")).toBe("sending");
		expect(storedStatusOf("queued")).toBe("queued");
		expect(storedStatusOf("notStarted")).toBe("notStarted");
	});

	test("'Don't save' and secret-looking values are kept out", () => {
		const run = run14();
		const token = `ghp_${"a1".repeat(20)}`;
		const secret: RunEntry = {
			...run,
			copy: {
				...run.copy,
				values: { ...run.copy.values, vendor_name: token },
			},
		};
		expect(recordOf(secret, form, "s").inputs.vendor_name).toEqual({
			$hidden: true,
		});
		const noSave = recordOf(run, form, "s", ["vendor_name"]);
		expect(noSave.inputs.vendor_name).toEqual({ $hidden: true });
		expect(noSave.inputs.max_pages).toBe("20");
	});

	test("a refused start is never saved as its outcome", () => {
		const run: RunEntry = { ...run14(), outcome: { kind: "noPlace" } };
		expect(recordOf(run, form, "s").outcome).toBeNull();
	});
});

describe("entryOf", () => {
	const record = (status: StoredRunRecord["status"]) => ({
		...recordOf(run14(), form, "profile:local"),
		status,
	});

	test("queued and sending read back as not started: the form was closed before its turn", () => {
		for (const status of ["queued", "sending"] as const) {
			const entry = entryOf(record(status), form);
			expect(entry.status).toBe("notStarted");
			expect(entry.outcome).toEqual({
				kind: "notStarted",
				reason: "formClosed",
			});
		}
	});

	test("a run still running when the page closed reads back as unknown", () => {
		const entry = entryOf(record("running"), form);
		expect(entry.status).toBe("unknown");
		expect(entry.outcome).toEqual({ kind: "unknown" });
	});

	test("files come back as Pick again reminders with ids unique per run", () => {
		const entry = entryOf(record("done"), form);
		expect(entry.origin).toBe("history");
		expect(entry.output).toBeNull();
		const slot = entry.copy.values.invoice_file as FileSlot;
		expect(slot.state).toBe("reminder");
		expect(slot.name).toBe("invoice-RE-2026-0917.pdf");
		expect(slot.id.startsWith(`${entry.id}:`)).toBe(true);
		const other = entryOf({ ...record("done"), id: "run-other" }, form);
		expect((other.copy.values.invoice_file as FileSlot).id).not.toBe(slot.id);
	});

	test("hidden inputs stay hidden; inputs that no longer fit are left out", () => {
		const base = record("done");
		const entry = entryOf(
			{
				...base,
				inputs: { ...base.inputs, vendor_name: { $hidden: true }, gone: "x" },
			},
			form,
		);
		expect(entry.copy.values.vendor_name).toEqual({ $hidden: true });
		expect("gone" in entry.copy.values).toBe(false);
	});

	test("an object property kept out stays hidden, so Run again offers Use these inputs instead", () => {
		const base = record("done");
		const terms = form.fields.find((field) => field.kind === "group");
		if (!terms) throw new Error("the medium form has no object field");
		const [kept, hidden] = terms.props;
		const entry = entryOf(
			{
				...base,
				inputs: {
					...base.inputs,
					vendor_name: "Nordwind Logistik GmbH",
					[terms.name]: {
						[kept.name]: "EUR",
						[hidden.name]: { $hidden: true },
					},
				},
			},
			form,
		);
		expect(entry.copy.values[terms.name]).toMatchObject({
			[kept.name]: "EUR",
			[hidden.name]: { $hidden: true },
		});
		expect(repeatBlocker(entry, Date.now())).toBe("files");
		const fileNames = new Set(
			form.fields
				.filter((field) => field.kind === "file" || field.kind === "files")
				.map((field) => field.name),
		);
		const withoutFiles: RunEntry = {
			...entry,
			copy: {
				...entry.copy,
				values: Object.fromEntries(
					Object.entries(entry.copy.values).filter(
						([name]) => !fileNames.has(name),
					),
				),
			},
		};
		expect(repeatBlocker(withoutFiles, Date.now())).toBe("hidden");
	});

	test("recordOf → entryOf keeps the values a run sent (files as reminders)", () => {
		const entry = entryOf(record("done"), form);
		const fields = form.fields.filter(
			(field: WorkbenchField) =>
				field.kind !== "file" && field.kind !== "files",
		);
		for (const field of fields)
			expect(entry.copy.values[field.name]).toEqual(
				run14().copy.values[field.name],
			);
	});
});

describe("mergeHistory", () => {
	test("this session's runs first; saved runs not of this session after them, newest first", () => {
		const done = fixture("done");
		const session = done.runs.filter((run) => run.origin === "session");
		const records = [
			recordOf(done.runs[0], form, "s"),
			...done.runs.slice(1).map((run) => recordOf(run, form, "s")),
		].reverse();
		const merged = mergeHistory(session, records, form);
		expect(merged[0]).toBe(session[0]);
		expect(merged.map((run) => run.n)).toEqual(
			[...done.runs].map((run) => run.n),
		);
		expect(merged.filter((run) => run.n === 14)).toHaveLength(1);
	});

	test("the next number follows every run", () => {
		expect(nextNumberAfter(fixture("done").runs)).toBe(15);
		expect(nextNumberAfter([])).toBe(1);
	});

	test("session persistence uses one fixed scope", () => {
		expect(memoryScopeOf({ memoryScope: "user:abc" })).toBe("user:abc");
		expect(memoryScopeOf({ memoryScope: null })).toBe(SESSION_SCOPE);
	});
});
