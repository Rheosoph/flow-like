import { expect, test } from "bun:test";
import {
	type ExportCommands,
	parseExportedLatestEvents,
	prepareDesktopProject,
} from "./project-export";

function fixture() {
	const manifest = new TextEncoder().encode("snapshot manifest");
	const data = new Uint8Array(3 * 1024 * 1024 + 15).fill(42);
	const files = new Map([
		["apps/project/manifest.app", manifest],
		["apps/project/upload/data", data],
	]);
	const reads: { path: string; offset: number; length: number }[] = [];
	let releases = 0;
	const commands: ExportCommands = {
		prepare: async (project) => ({
			export_id: "a0000000-0000-4000-8000-000000000000",
			project_id: project,
			files: [...files].map(([path, bytes]) => ({ path, size: bytes.length })),
			assets: { bit_pins: [], package_pins: [] },
		}),
		read: async (_, path, offset, length) => {
			reads.push({ path, offset, length });
			const value = files.get(path);
			if (!value) throw new Error("outside snapshot");
			return value.slice(offset, offset + length).buffer;
		},
		release: async () => {
			releases++;
		},
	};
	return { commands, reads, releases: () => releases };
}

test("automatic export hashes a large native snapshot through bounded chunks", async () => {
	const f = fixture();
	const prepared = await prepareDesktopProject("project", f.commands);
	expect(prepared.artifact.descriptor.file_count).toBe(2);
	expect(f.reads.length).toBe(5);
	expect(f.reads.every((read) => read.length <= 1024 * 1024)).toBe(true);
	const file = prepared.artifact.files.find((file) =>
		file.path.endsWith("/data"),
	)?.file;
	expect(file).toBeDefined();
	if (!file) throw new Error("Missing snapshot file");
	expect(new Uint8Array(await file.slice(1024, 1030).arrayBuffer())).toEqual(
		new Uint8Array(6).fill(42),
	);
	await prepared.release();
	await prepared.release();
	expect(f.releases()).toBe(1);
	await expect(file.slice(0, 1).arrayBuffer()).rejects.toThrow("released");
});

test("incomplete chunk or a mismatched project releases the native snapshot", async () => {
	const f = fixture();
	f.commands.read = async () => new ArrayBuffer(0);
	await expect(prepareDesktopProject("project", f.commands)).rejects.toThrow(
		"incomplete chunk",
	);
	expect(f.releases()).toBe(1);
	const g = fixture();
	const prepare = g.commands.prepare;
	g.commands.prepare = async (id) => ({
		...(await prepare(id)),
		project_id: "other",
	});
	await expect(prepareDesktopProject("project", g.commands)).rejects.toThrow(
		"another project",
	);
	expect(g.reads).toHaveLength(0);
	expect(g.releases()).toBe(1);
});

test("cancelling while the native snapshot is preparing releases it before reading", async () => {
	const f = fixture();
	const cancel = new AbortController();
	const prepare = f.commands.prepare;
	f.commands.prepare = async (id) => {
		cancel.abort();
		return prepare(id);
	};
	await expect(
		prepareDesktopProject("project", f.commands, cancel.signal),
	).rejects.toThrow();
	expect(f.reads).toHaveLength(0);
	expect(f.releases()).toBe(1);
});

test("the exporter cannot smuggle files from another project", async () => {
	const f = fixture();
	const prepare = f.commands.prepare;
	f.commands.prepare = async (id) => ({
		...(await prepare(id)),
		files: [{ path: "apps/other/private.txt", size: 1 }],
	});
	await expect(prepareDesktopProject("project", f.commands)).rejects.toThrow();
	expect(f.reads).toHaveLength(0);
	expect(f.releases()).toBe(1);
});

test("the export says what it did with each event that follows Latest", async () => {
	const f = fixture();
	// An export from before Latest events could be deployed says nothing.
	expect(
		(await prepareDesktopProject("project", f.commands)).latestEvents,
	).toBeNull();
	const prepare = f.commands.prepare;
	// The literal of run-more-design §1.8.
	const latest_events = [
		{
			event_id: "evt_a",
			board_id: "board_a",
			board_version: [0, 0, 8],
			problem: null,
		},
		{
			event_id: "evt_b",
			board_id: "board_b",
			board_version: null,
			problem: "edited",
		},
		{
			event_id: "evt_c",
			board_id: "board_c",
			board_version: null,
			problem: "target_missing",
		},
	];
	f.commands.prepare = async (id) => ({
		...(await prepare(id)),
		latest_events,
	});
	expect(
		(await prepareDesktopProject("project", f.commands)).latestEvents,
	).toEqual(latest_events as never);
	expect(parseExportedLatestEvents([])).toEqual([]);
	// A cause this client does not know loses only the cause; an entry it can't read is dropped, not the list.
	expect(
		parseExportedLatestEvents([
			{
				event_id: "evt_d",
				board_id: "board_d",
				board_version: null,
				problem: "locked",
			},
			{ event_id: "evt_e" },
			"evt_f",
		]),
	).toEqual([
		{
			event_id: "evt_d",
			board_id: "board_d",
			board_version: null,
			problem: null,
		},
	]);
});
