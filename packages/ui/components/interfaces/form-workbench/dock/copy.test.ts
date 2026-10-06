import { beforeAll, describe, expect, test } from "bun:test";
import i18next from "i18next";
import {
	type DockMessage,
	FORM_LIMITS,
	type ShortWords,
	type StartMessage,
} from "../contracts";
import {
	type CopyContext,
	type InterfacesT,
	afterRunSummary,
	backText,
	capButton,
	capSentence,
	failureText,
	holdText,
	joinLabels,
	keysOf,
	limitText,
	listJoinerOf,
	messageText,
	offerText,
	runAgainLabel,
	runLabel,
	runTitle,
	seriesEndText,
	shortcutsLabel,
	stopAria,
	stopTitle,
} from "./copy";

let t: InterfacesT;

beforeAll(async () => {
	const instance = i18next.createInstance();
	await instance.init({
		lng: "en",
		resources: {},
		interpolation: { escapeValue: false },
	});
	t = instance.getFixedT("en", "interfaces") as unknown as InterfacesT;
});

const copy = (mac = true): CopyContext => ({
	mac,
	decimalSign: ".",
	warnBytes: FORM_LIMITS.warnFileBytes,
	list: listJoinerOf("en"),
});

const text = (message: DockMessage, mac = true) =>
	messageText(t, message, copy(mac));

const start = (patch: Partial<StartMessage>): DockMessage => ({
	kind: "start",
	start: {
		n: 15,
		state: "started",
		files: 0,
		pairs: [],
		lastFile: false,
		leftAsIs: false,
		...patch,
	},
});

describe("start messages (spec M1)", () => {
	test("what the run took in its per-run fields", () => {
		expect(
			text(start({ pairs: ["invoice-RE-2026-0918.pdf", "18 Sep 2026"] })),
		).toBe("Run 15 started: invoice-RE-2026-0918.pdf, 18 Sep 2026.");
	});

	test("queued and sending runs say so", () => {
		expect(
			text(
				start({
					n: 18,
					state: "queued",
					pairs: ["invoice-RE-2026-0921.pdf", "22 Sep 2026"],
				}),
			),
		).toBe("Run 18 is queued: invoice-RE-2026-0921.pdf, 22 Sep 2026.");
		expect(
			text(
				start({
					n: 16,
					state: "sending",
					files: 1,
					pairs: ["invoice-RE-2026-0919.pdf", "21 Sep 2026"],
				}),
			),
		).toBe(
			"Run 16 starts when its file is sent: invoice-RE-2026-0919.pdf, 21 Sep 2026.",
		);
		expect(text(start({ n: 2, state: "sending", files: 3 }))).toBe(
			"Run 2 starts when its files are sent.",
		);
	});

	test("the last file of a series", () => {
		expect(
			text(
				start({
					n: 24,
					pairs: ["invoice-RE-2026-0927.pdf", "30 Sep 2026"],
					lastFile: true,
				}),
			),
		).toBe(
			"Run 24 started: invoice-RE-2026-0927.pdf, 30 Sep 2026. That was the last file.",
		);
	});

	test("⇧⌘↵ leaves the inputs as they are", () => {
		expect(text(start({ n: 16, leftAsIs: true }))).toBe(
			"Run 16 started. Inputs left as they are this time.",
		);
	});

	test("a queued run with nothing per run", () => {
		expect(text(start({ n: 4, state: "queued" }))).toBe("Run 4 is queued.");
	});
});

describe("the other dock messages", () => {
	test("same inputs: the key that runs it again follows the platform", () => {
		const same: DockMessage = { kind: "sameInputs", n: 14 };
		expect(text(same)).toBe("Same inputs as run 14. ⌘↵ runs it again.");
		expect(text(same, false)).toBe(
			"Same inputs as run 14. Ctrl+Enter runs it again.",
		);
	});

	test("resets, removals and presets", () => {
		expect(text({ kind: "fieldReset", label: "Max pages" })).toBe(
			"Max pages reset.",
		);
		expect(
			text({ kind: "fileRemoved", name: "invoice-RE-2026-0918.pdf" }),
		).toBe("invoice-RE-2026-0918.pdf removed.");
		expect(text({ kind: "presetApplied", name: "Nordwind", misfit: 0 })).toBe(
			"Nordwind applied.",
		);
		expect(text({ kind: "presetApplied", name: "Nordwind", misfit: 1 })).toBe(
			"Nordwind applied. 1 saved input no longer fits this form.",
		);
		expect(text({ kind: "presetApplied", name: "Nordwind", misfit: 2 })).toBe(
			"Nordwind applied. 2 saved inputs no longer fit this form.",
		);
		expect(text({ kind: "presetUpdated", name: "Nordwind" })).toBe(
			"Nordwind updated.",
		);
		expect(text({ kind: "presetSaved", name: "Nordwind" })).toBe(
			"Saved as Nordwind.",
		);
		expect(text({ kind: "presetDeleted", name: "Nordwind" })).toBe(
			"Nordwind deleted.",
		);
		expect(text({ kind: "resetTo", presetName: null })).toBe(
			"Inputs reset to their defaults.",
		);
		expect(text({ kind: "resetTo", presetName: "Nordwind" })).toBe(
			"Inputs reset to Nordwind.",
		);
	});

	test("the offer's answer names the fields that are now per run", () => {
		expect(
			text({ kind: "perRunNow", labels: ["Order number", "Receipt"] }),
		).toBe("Order number and Receipt are now per run.");
		expect(text({ kind: "perRunNow", labels: ["Order number"] })).toBe(
			"Order number is now per run.",
		);
	});

	test("Use these inputs: what still needs doing", () => {
		const base = { kind: "inputsIn" as const, n: 9 };
		expect(text({ ...base, pickAgain: 3, enterAgain: [], misfit: 0 })).toBe(
			"Run 9's inputs are in. Pick 3 files again.",
		);
		expect(text({ ...base, pickAgain: 1, enterAgain: [], misfit: 0 })).toBe(
			"Run 9's inputs are in. Pick 1 file again.",
		);
		expect(text({ ...base, pickAgain: 0, enterAgain: [], misfit: 0 })).toBe(
			"Run 9's inputs are in.",
		);
		expect(
			text({ ...base, pickAgain: 0, enterAgain: ["Password"], misfit: 0 }),
		).toBe("Run 9's inputs are in. Enter Password again.");
		expect(text({ ...base, pickAgain: 0, enterAgain: [], misfit: 1 })).toBe(
			"Run 9's inputs are in. 1 input no longer fits this form.",
		);
		expect(
			text({ ...base, pickAgain: 2, enterAgain: ["A", "B"], misfit: 3 }),
		).toBe(
			"Run 9's inputs are in. Pick 2 files again. Enter A and B again. 3 inputs no longer fit this form.",
		);
	});

	test("Don't save, next files and the queue", () => {
		expect(text({ kind: "noSave", label: "Vendor" })).toBe(
			"Vendor is no longer saved on this device. Earlier runs no longer show it.",
		);
		expect(text({ kind: "nextFilesRemoved", count: 9 })).toBe(
			"9 next files removed.",
		);
		expect(text({ kind: "nextFilesRemoved", count: 1 })).toBe(
			"1 next file removed.",
		);
		expect(text({ kind: "nextFilesCapped" })).toBe(
			"Only the first 50 files were added. A field takes up to 50 at a time.",
		);
		expect(text({ kind: "queueCleared", runs: 1, files: 5 })).toBe(
			"Queue cleared. 1 run did not start and 5 next files were removed.",
		);
		expect(text({ kind: "queueCleared", runs: 2, files: 0 })).toBe(
			"Queue cleared. 2 runs did not start.",
		);
		expect(text({ kind: "queueCleared", runs: 0, files: 1 })).toBe(
			"Queue cleared. 1 next file was removed.",
		);
		expect(text({ kind: "queueCleared", runs: 0, files: 0 })).toBe(
			"Queue cleared.",
		);
	});

	test("files left out of a pick", () => {
		const slot = (name: string) => ({
			id: name,
			name,
			size: 1,
			type: "application/pdf",
			state: "sent" as const,
			progress: null,
			ref: null,
			error: null,
			sentAt: null,
			expiresAt: null,
		});
		expect(
			text({
				kind: "leftOut",
				files: [{ slot: slot("invoice-RE-2026-0917.pdf"), n: 14 }],
			}),
		).toBe(
			"invoice-RE-2026-0917.pdf was already sent in run 14 and was left out.",
		);
		expect(
			text({
				kind: "leftOut",
				files: [
					{ slot: slot("a.pdf"), n: 1 },
					{ slot: slot("b.pdf"), n: 2 },
					{ slot: slot("c.pdf"), n: 3 },
				],
			}),
		).toBe("3 files were already sent in earlier runs and were left out.");
	});

	test("size refusals and warnings, in the repo's MB", () => {
		expect(
			text({
				kind: "fileRefused",
				name: "invoice-RE-2026-0923.pdf",
				limitBytes: 3_735_552,
				host: "service",
			}),
		).toBe(
			"invoice-RE-2026-0923.pdf is larger than 3.5 MB, the most this device page accepts. It was not added.",
		);
		expect(
			text({
				kind: "fileRefused",
				name: "big.pdf",
				limitBytes: 1_523_712,
				host: "hosted",
			}),
		).toBe(
			"big.pdf is larger than 1.4 MB, the most this link can send. It was not added.",
		);
		expect(text({ kind: "fileWarning", name: "huge.zip" })).toBe(
			"huge.zip is larger than 35 MB. Sending it may fail.",
		);
		expect(text({ kind: "oneFileOnly", name: "a.pdf" })).toBe(
			"Only a.pdf was added. This page sends one file per run.",
		);
		expect(
			text({
				kind: "requestTooLarge",
				totalBytes: 10_170_000,
				limitBytes: 7_471_104,
			}),
		).toBe(
			"These files are too large to send together from this page (9.7 MB of 7.1 MB).",
		);
	});
});

describe("failure, hold and the cap", () => {
	test("a failure names its step when it has one", () => {
		const step = { number: 2, title: "Run OCR" };
		expect(failureText(t, { runId: "r", n: 18, step })).toBe(
			"Run 18 failed at step 2: Run OCR.",
		);
		expect(
			failureText(t, { runId: "r", n: 18, step: { number: 2, title: "" } }),
		).toBe("Run 18 failed at step 2.");
		expect(failureText(t, { runId: "r", n: 18, step: null })).toBe(
			"Run 18 failed.",
		);
	});

	test("the hold names both runs and the step", () => {
		expect(
			holdText(t, { runs: [18, 19], step: { number: 2, title: "Run OCR" } }),
		).toBe("Runs 18 and 19 failed at step 2: Run OCR. The queue is on hold.");
		expect(
			holdText(t, { runs: [18, 19], step: { number: 2, title: "" } }),
		).toBe("Runs 18 and 19 failed at step 2. The queue is on hold.");
	});

	test("a form without fields at its cap", () => {
		expect(capButton(t)).toBe("Wait for a run to end");
		expect(capSentence(t, 3)).toBe(
			"3 runs are going. You can run again when one ends.",
		);
		expect(capSentence(t, 1)).toBe(
			"1 run is going. You can run again when it ends.",
		);
	});
});

describe("the Run and Stop words", () => {
	test("labels and the title with the key that runs", () => {
		expect(runLabel(t)).toBe("Run");
		expect(runAgainLabel(t)).toBe("Run again");
		expect(runTitle(t, "Run", true, false)).toBe("Run · ⌘↵");
		expect(runTitle(t, "Run", true, true)).toBe(
			"Run · ⌘↵. Run and leave the inputs as they are · ⇧⌘↵",
		);
		expect(runTitle(t, "Run", false, true)).toBe(
			"Run · Ctrl+Enter. Run and leave the inputs as they are · Ctrl+Shift+Enter",
		);
		expect(runTitle(t, "Submit", false, false)).toBe("Submit · Ctrl+Enter");
	});

	test("Stop names the run and its key", () => {
		expect(stopAria(t, 17)).toBe("Stop run 17");
		expect(stopTitle(t, 17, true)).toBe("Stop run 17 · ⌘.");
		expect(stopTitle(t, 17, false)).toBe("Stop run 17 · Ctrl+.");
	});

	test("key names", () => {
		expect(keysOf(true).shortcuts).toBe("⌘/");
		expect(shortcutsLabel(t, false)).toBe("Keyboard shortcuts (Ctrl+/)");
	});
});

describe("the after-run line and the questions", () => {
	test("how many inputs are per run", () => {
		expect(afterRunSummary(t, 0)).toBe("No input is per run");
		expect(afterRunSummary(t, 1)).toBe("1 input is per run");
		expect(afterRunSummary(t, 3)).toBe("3 inputs are per run");
	});

	test("labels read as a list in the app's language", () => {
		const { list } = copy();
		expect(joinLabels(t, list, ["A"])).toBe("A");
		expect(joinLabels(t, list, ["A", "B"])).toBe("A and B");
		expect(joinLabels(t, list, ["A", "B", "C"])).toBe("A, B and C");
		expect(joinLabels(t, list, ["A", "B", "C", "D"])).toBe("A, B and 2 more");
		expect(joinLabels(t, list, ["A", "B", "C", "D", "E"])).toBe(
			"A, B and 3 more",
		);
		expect(joinLabels(t, listJoinerOf("de"), ["A", "B"])).toBe("A und B");
	});

	test("the offer and the question when a series ends", () => {
		const { list } = copy();
		expect(offerText(t, joinLabels(t, list, ["Order number", "Receipt"]))).toBe(
			"Make Order number and Receipt per run?",
		);
		expect(offerText(t, joinLabels(t, list, ["Customer number"]))).toBe(
			"Make Customer number per run?",
		);
		expect(
			seriesEndText(
				t,
				joinLabels(t, list, [
					"Invoice",
					"Supporting documents",
					"Invoice date",
				]),
			),
		).toBe(
			"Should Invoice, Supporting documents and Invoice date stay per run?",
		);
	});
});

describe("what a Per run row goes back to", () => {
	const words: ShortWords = {
		none: "none",
		empty: "empty",
		on: "On",
		off: "Off",
		files: (count) => `${count} files`,
		entries: (count) => `${count} entries`,
		date: (iso) => iso,
	};

	test("spec M1 c", () => {
		expect(backText(t, { kind: "nextFile" }, words)).toBe("Next file");
		expect(backText(t, { kind: "empty" }, words)).toBe("Empty");
		expect(backText(t, { kind: "on" }, words)).toBe("Back to On");
		expect(backText(t, { kind: "off" }, words)).toBe("Back to Off");
		expect(backText(t, { kind: "value", text: "4400, 4410" }, words)).toBe(
			"Back to 4400, 4410",
		);
		expect(backText(t, { kind: "objectDefault" }, words)).toBe(
			"Back to its starting value",
		);
	});
});

describe("size limits as the repo words them", () => {
	test("MB floored to one decimal", () => {
		expect(limitText(3_735_552, ".")).toBe("3.5 MB");
		expect(limitText(1_523_712, ".")).toBe("1.4 MB");
		expect(limitText(7_471_104, ".")).toBe("7.1 MB");
		expect(limitText(35 * 1_048_576, ".")).toBe("35 MB");
		expect(limitText(3_735_552, ",")).toBe("3,5 MB");
	});
});
