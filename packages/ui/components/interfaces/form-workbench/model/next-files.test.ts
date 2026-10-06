import { describe, expect, test } from "bun:test";
import {
	FORM_LIMITS,
	type FileSlot,
	type HostCapabilities,
} from "../contracts";
import {
	inlineCheck,
	naturalCompare,
	nextLineParts,
	queueFiles,
	requestCheck,
	sameFile,
	sentFiles,
} from "./next-files";

const MB = 1_048_576;

function slot(
	name: string,
	size: number | null = 1000,
	state: FileSlot["state"] = "waiting",
): FileSlot {
	return {
		id: `slot-${name}`,
		name,
		size,
		type: null,
		state,
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
}

/** FLP_SAMPLES.medium.invoice_file: 0917 (which run 14 sent), then the ten new invoices. */
const FOLDER = [
	slot("invoice-RE-2026-0917.pdf", 1_284_096),
	...[
		["0918", 1_198_080],
		["0919", 1_361_920],
		["0920", 1_003_520],
		["0921", 1_247_232],
		["0922", 1_144_832],
		["0923", 4_089_446],
		["0924", 1_310_720],
		["0925", 1_093_632],
		["0926", 1_222_656],
		["0927", 1_175_552],
	].map(([day, size]) => slot(`invoice-RE-2026-${day}.pdf`, Number(size))),
];
const SENT = sentFiles(
	[
		{
			n: 14,
			values: {
				invoice_file: {
					$file: { name: "invoice-RE-2026-0917.pdf", size: 1_284_096 },
				},
			},
		},
	],
	"invoice_file",
);

/** HostCapabilities as S-STATE resolves them, from FLP_HOSTS. */
const HOSTS: Record<
	"app" | "hosted" | "service",
	Pick<
		HostCapabilities,
		"uploads" | "inlineFileLimitBytes" | "inlineRoomBytes" | "warnFileBytes"
	>
> = {
	app: {
		uploads: "temporary",
		inlineFileLimitBytes: null,
		inlineRoomBytes: null,
		warnFileBytes: FORM_LIMITS.warnFileBytes,
	},
	hosted: {
		uploads: "inline",
		inlineFileLimitBytes: null,
		inlineRoomBytes: Math.floor(
			((FORM_LIMITS.hostedRequestBytes - FORM_LIMITS.hostedReserveBytes) * 3) /
				4,
		),
		warnFileBytes: null,
	},
	service: {
		uploads: "inline",
		inlineFileLimitBytes: 3_735_552,
		inlineRoomBytes: Math.floor(
			((10 * MB - FORM_LIMITS.serviceReserveBytes) * 3) / 4,
		),
		warnFileBytes: null,
	},
};

describe("natural order", () => {
	test("2 before 10, case ignored", () => {
		expect(
			["file-10.pdf", "File-2.pdf", "file-1.pdf"].sort(naturalCompare),
		).toEqual(["file-1.pdf", "File-2.pdf", "file-10.pdf"]);
	});
});

describe("files runs on this device were given (flpSentFiles)", () => {
	test("from session copies and stored records, newest run first", () => {
		const runs = [
			{ n: 15, values: { invoice_file: slot("b.pdf", 2, "sent") } },
			{
				n: 14,
				values: { invoice_file: { $file: { name: "a.pdf", size: null } } },
			},
			{ n: 13, values: { supporting_documents: [slot("c.pdf", 3, "sent")] } },
		];
		expect(sentFiles(runs, "invoice_file")).toEqual([
			{ name: "b.pdf", size: 2, n: 15 },
			{ name: "a.pdf", size: null, n: 14 },
		]);
		expect(sentFiles(runs, "supporting_documents")).toEqual([
			{ name: "c.pdf", size: 3, n: 13 },
		]);
	});

	test("the same file: same name, and the same size when both are known", () => {
		expect(sameFile({ name: "a", size: 1 }, { name: "a", size: 1 })).toBe(true);
		expect(sameFile({ name: "a", size: 1 }, { name: "a", size: 2 })).toBe(
			false,
		);
		expect(sameFile({ name: "a", size: null }, { name: "a", size: 2 })).toBe(
			true,
		);
		expect(sameFile({ name: "a", size: 1 }, { name: "b", size: 1 })).toBe(
			false,
		);
	});
});

describe("several files for a one-file field (flpQueueFiles)", () => {
	test("the benchmark: 0917 is left out, 0918 fills the field, nine wait", () => {
		const queued = queueFiles(null, [], [...FOLDER].reverse(), SENT);
		expect(queued.current?.name).toBe("invoice-RE-2026-0918.pdf");
		expect(queued.next.map((file) => file.name.slice(16, 20))).toEqual([
			"0919",
			"0920",
			"0921",
			"0922",
			"0923",
			"0924",
			"0925",
			"0926",
			"0927",
		]);
		expect(queued.leftOut.map((file) => [file.slot.name, file.n])).toEqual([
			["invoice-RE-2026-0917.pdf", 14],
		]);
		expect(queued.dropped).toBe(0);
		expect(nextLineParts(queued.next)).toEqual({
			name: "invoice-RE-2026-0919.pdf",
			more: 8,
		});
	});

	test("a current file that was not sent stays first; files already in the field are skipped", () => {
		const current = slot("a.pdf");
		const queued = queueFiles(
			current,
			[slot("b.pdf")],
			[slot("c.pdf"), slot("b.pdf"), slot("a.pdf")],
			[],
		);
		expect([
			queued.current?.name,
			...queued.next.map((file) => file.name),
		]).toEqual(["a.pdf", "b.pdf", "c.pdf"]);
		expect(queued.leftOut).toEqual([]);
	});

	test("a current file a run already had is left out too; a reminder simply goes", () => {
		const current = slot("invoice-RE-2026-0917.pdf", 1_284_096, "sent");
		const queued = queueFiles(current, [], [slot("x.pdf")], SENT);
		expect(queued.current?.name).toBe("x.pdf");
		expect(queued.leftOut.map((file) => file.slot.name)).toEqual([
			"invoice-RE-2026-0917.pdf",
		]);
		const reminder = queueFiles(
			slot("old.pdf", 1, "reminder"),
			[],
			[slot("y.pdf"), slot("z.pdf")],
			[],
		);
		expect([
			reminder.current?.name,
			reminder.next.length,
			reminder.leftOut.length,
		]).toEqual(["y.pdf", 1, 0]);
	});

	test("up to 50 next files; the rest are dropped", () => {
		const many = Array.from({ length: 53 }, (_, index) =>
			slot(`scan-${index + 1}.pdf`),
		);
		const queued = queueFiles(null, [], many, []);
		expect([queued.current?.name, queued.next.length, queued.dropped]).toEqual([
			"scan-1.pdf",
			50,
			2,
		]);
		expect(queued.next[0].name).toBe("scan-2.pdf");
	});

	test("the Next line: one left, none left", () => {
		expect(nextLineParts([slot("invoice-RE-2026-0927.pdf")])).toEqual({
			name: "invoice-RE-2026-0927.pdf",
			more: 0,
		});
		expect(nextLineParts([])).toBeNull();
	});
});

describe("what each host can send (flpInlineCheck, flpRequestCheck)", () => {
	test("the hosted link refuses a file above 1.4 MB", () => {
		const check = inlineCheck(HOSTS.hosted, [
			slot("small.pdf", MB),
			slot("invoice-RE-2026-0923.pdf", 4_089_446),
		]);
		expect(check.added.map((file) => file.name)).toEqual(["small.pdf"]);
		expect(check.refused).toEqual([
			{ name: "invoice-RE-2026-0923.pdf", limitBytes: 1_523_712 },
		]);
		expect(Math.floor((1_523_712 / MB) * 10) / 10).toBe(1.4);
	});

	test("the device page refuses a file above 3.5 MB", () => {
		const check = inlineCheck(HOSTS.service, [
			slot("invoice-RE-2026-0923.pdf", 4_089_446),
			slot("ok.pdf", 3 * MB),
		]);
		expect(check.refused).toEqual([
			{ name: "invoice-RE-2026-0923.pdf", limitBytes: 3_735_552 },
		]);
		expect(check.added.length).toBe(1);
	});

	test("the app takes every file and warns above 35 MB", () => {
		const check = inlineCheck(HOSTS.app, [
			slot("big.zip", 40 * MB),
			slot("unknown.bin", null),
		]);
		expect(check.added.length).toBe(2);
		expect(check.refused).toEqual([]);
		expect(check.warned).toEqual([{ name: "big.zip" }]);
	});

	test("at the press: a run's files together on the device page (7.1 MB); nothing to check on the app", () => {
		expect(
			requestCheck(HOSTS.service, [
				slot("a", 3_670_016),
				slot("b", 3_670_016),
				slot("c", 2_831_155),
			]),
		).toEqual({
			totalBytes: 10_171_187,
			limitBytes: 7_471_104,
		});
		expect(Math.floor((7_471_104 / MB) * 10) / 10).toBe(7.1);
		expect(requestCheck(HOSTS.service, [slot("a", MB)])).toBeNull();
		expect(requestCheck(HOSTS.app, [slot("a", 100 * MB)])).toBeNull();
	});
});
