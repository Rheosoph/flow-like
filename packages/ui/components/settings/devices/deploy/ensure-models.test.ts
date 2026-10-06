import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { PreparedModelAsset } from "../../../../lib/device-management/artifacts";
import type { ModelJob } from "../../../../lib/device-management/models";
import type { ManagementResponse } from "../../../../lib/device-management/types";
import {
	type EnsureModelsInput,
	type ModelFileRow,
	ensureModels,
	ensureModelsSeams,
} from "./ensure-models";
import { DeployRunFailure } from "./update-path";

const seams = { ...ensureModelsSeams };
beforeEach(() => {
	ensureModelsSeams.pollMs = 1;
	ensureModelsSeams.confirmMs = 0;
});
afterEach(() => Object.assign(ensureModelsSeams, seams));

const JOB = "12345678-1234-4234-8234-123456789abc";
const hex = (fill: string) => fill.repeat(64);

function asset(fill: string, fileName = `${fill}.gguf`): PreparedModelAsset {
	return {
		pin: `pin-${fill}`,
		bitId: `bit-${fill}`,
		bitHash: hex(fill),
		descriptor: {
			digest: { algorithm: "blake3", hex: hex(fill) },
			size: 1000,
			file_name: fileName,
			sources: ["https://cdn.flow-like.com/bits/model"],
		},
	};
}

function job(fill: string, state: Record<string, unknown>): ModelJob {
	return {
		job_id: JOB,
		digest: { algorithm: "blake3", hex: hex(fill) },
		size: 1000,
		file_name: `${fill}.gguf`,
		updated_at: 1,
		...state,
	} as ModelJob;
}

/** A device whose `ensure` summaries and job lists follow the given scripts; the last entry repeats. */
function device(script: {
	ensure: { total: number; present: number; pending?: unknown[] }[];
	jobs: ModelJob[][];
}) {
	const sent: Record<string, unknown>[] = [];
	const call = async (
		command: Record<string, unknown>,
	): Promise<ManagementResponse> => {
		sent.push(command);
		const request = command.request as { kind: string };
		const take = <T>(list: T[]) => (list.length > 1 ? list.shift() : list[0]);
		const result =
			request.kind === "ensure"
				? { pending: [], ...take(script.ensure) }
				: { jobs: take(script.jobs) ?? [], next: null };
		return {
			operation_id: "op",
			state: "completed",
			result,
		} as ManagementResponse;
	};
	return {
		call,
		sent,
		kinds: () => sent.map((c) => (c.request as { kind: string }).kind),
	};
}

function input(
	call: EnsureModelsInput["call"],
	assets: PreparedModelAsset[],
	extra: Partial<EnsureModelsInput> = {},
) {
	const reports: (readonly ModelFileRow[])[] = [];
	const value: EnsureModelsInput = {
		call,
		features: () => ({ model_store: 1 }),
		projectId: "project",
		models: {
			pins: assets.map((entry) => ({
				bit_id: entry.pin,
				metadata_sha256: hex("e"),
			})),
			assets,
		},
		signal: new AbortController().signal,
		guard: () => {},
		report: (rows) => reports.push(rows),
		...extra,
	};
	return { value, reports };
}

describe("ensure models", () => {
	test("files the device holds already need one question", async () => {
		const fake = device({ ensure: [{ total: 1, present: 1 }], jobs: [[]] });
		const { value, reports } = input(fake.call, [asset("a")]);
		await ensureModels(value);
		expect(fake.kinds()).toEqual(["ensure"]);
		expect(fake.sent[0]).toEqual({
			type: "models",
			request: {
				kind: "ensure",
				project_id: "project",
				pins: [{ bit_id: "pin-a", metadata_sha256: hex("e") }],
			},
		});
		expect(reports.at(-1)?.map((row) => row.state)).toEqual(["present"]);
	});

	test("follows the device's download and confirms its end", async () => {
		const fake = device({
			ensure: [
				{
					total: 1,
					present: 0,
					pending: [
						{
							digest: { algorithm: "blake3", hex: hex("a") },
							job_id: JOB,
							state: "fetching",
							source_index: 0,
							bytes: 0,
						},
					],
				},
				{ total: 1, present: 1 },
			],
			jobs: [
				[
					job("a", {
						state: "fetching",
						source_index: 0,
						bytes: 500,
						source_host: "cdn.flow-like.com",
						bytes_per_second: 1000,
					}),
				],
				[],
			],
		});
		const { value, reports } = input(fake.call, [asset("a")]);
		await ensureModels(value);
		expect(fake.kinds()).toEqual(["ensure", "jobs", "jobs", "ensure"]);
		expect(reports.flat()).toContainEqual(
			expect.objectContaining({
				state: "downloading",
				bytes: 500,
				sourceHost: "cdn.flow-like.com",
				bytesPerSecond: 1000,
			}),
		);
		expect(reports.at(-1)?.[0]?.state).toBe("present");
	});

	test("sends a file the device can't download, then confirms", async () => {
		const fake = device({
			ensure: [
				{ total: 1, present: 0 },
				{ total: 1, present: 1 },
			],
			jobs: [[job("a", { state: "failed", reason: "egress_blocked" })], []],
		});
		const sent: string[] = [];
		const { value, reports } = input(fake.call, [asset("a")], {
			send: async (entry, jobId, progress) => {
				sent.push(`${entry.descriptor.file_name}:${jobId}`);
				progress(400, "download");
			},
		});
		await ensureModels(value);
		expect(sent).toEqual([`a.gguf:${JOB}`]);
		expect(reports.flat()).toContainEqual(
			expect.objectContaining({ state: "failed", reason: "egress_blocked" }),
		);
		expect(reports.flat()).toContainEqual(
			expect.objectContaining({
				state: "sending",
				bytes: 400,
				from: "download",
			}),
		);
		expect(fake.kinds()).toEqual(["ensure", "jobs", "jobs", "ensure"]);
	});

	test("a download into this computer shows as its own step; after the last byte the device verifies", async () => {
		const fake = device({
			ensure: [
				{ total: 1, present: 0 },
				{ total: 1, present: 1 },
			],
			jobs: [[job("a", { state: "failed", reason: "egress_blocked" })], []],
		});
		const { value, reports } = input(fake.call, [asset("a")], {
			send: async (_entry, _jobId, progress) => {
				progress(300, "local_download");
				progress(1000, "local_download");
				progress(0, "bit_store");
				progress(1000, "bit_store");
			},
		});
		await ensureModels(value);
		const pushed = reports
			.flat()
			.filter((row) => row.from && row.state !== "present");
		expect(pushed.map((row) => [row.state, row.from, row.bytes])).toEqual([
			["sending", "local_download", 300],
			["sending", "local_download", 1000],
			["sending", "bit_store", 0],
			["verifying", "bit_store", 1000],
		]);
		expect(pushed[2]?.bytesPerSecond).toBeUndefined();
		expect(pushed[3]?.bytesPerSecond).toBeUndefined();
	});

	test("a failed file among more downloads than the first pages hold is found and sent", async () => {
		const others = Array.from({ length: 100 }, (_, index) => ({
			...job("0", { state: "fetching", source_index: 0, bytes: 1 }),
			job_id: `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`,
			digest: {
				algorithm: "sha256" as const,
				hex: index.toString(16).padStart(64, "0"),
			},
		}));
		const mine = {
			...job("a", { state: "failed", reason: "egress_blocked" }),
			job_id: "ffffffff-0000-4000-8000-000000000000",
		};
		let present = false;
		const pages: number[] = [];
		const call = async (
			command: Record<string, unknown>,
		): Promise<ManagementResponse> => {
			const request = command.request as {
				kind: string;
				after?: string | null;
				limit?: number;
			};
			const answer = (result: unknown) =>
				({
					operation_id: "op",
					state: "completed",
					result,
				}) as ManagementResponse;
			if (request.kind === "ensure")
				return answer({ total: 1, present: present ? 1 : 0, pending: [] });
			const all = present ? others : [...others, mine];
			const start = request.after
				? all.findIndex((entry) => entry.job_id === request.after) + 1
				: 0;
			const page = all.slice(start, start + (request.limit ?? 16));
			pages.push(page.length);
			const more = start + page.length < all.length;
			return answer({ jobs: page, next: more ? page.at(-1)?.job_id : null });
		};
		const sent: string[] = [];
		const { value } = input(call, [asset("a")], {
			send: async (_entry, jobId) => {
				sent.push(jobId);
				present = true;
			},
		});
		await ensureModels(value);
		expect(sent).toEqual([mine.job_id]);
		expect(pages.slice(0, 4)).toEqual([32, 32, 32, 5]);
	});

	test("a full model disk stops the target", async () => {
		const fake = device({
			ensure: [{ total: 1, present: 0 }],
			jobs: [[job("a", { state: "failed", reason: "disk_budget" })]],
		});
		const { value } = input(fake.call, [asset("a")], { send: async () => {} });
		const error = await ensureModels(value).catch((value) => value);
		expect(error).toBeInstanceOf(DeployRunFailure);
		expect(error).toMatchObject({ code: "model_disk", message: "a.gguf" });
	});

	test("without a way to send it the target stops on the file", async () => {
		const fake = device({
			ensure: [{ total: 1, present: 0 }],
			jobs: [[job("a", { state: "failed", reason: "egress_blocked" })]],
		});
		const { value } = input(fake.call, [asset("a")]);
		await expect(ensureModels(value)).rejects.toMatchObject({
			code: "model_unreachable",
		});
	});

	test("a failed send keeps its reason on the file and stops the target", async () => {
		const fake = device({
			ensure: [{ total: 1, present: 0 }],
			jobs: [[job("a", { state: "awaiting_push", bytes: 10 })]],
		});
		const { value, reports } = input(fake.call, [asset("a")], {
			send: async () => {
				throw new Error("None of the sources of a.gguf delivered it.");
			},
		});
		await expect(ensureModels(value)).rejects.toMatchObject({
			code: "model_push",
			message: "a.gguf",
		});
		expect(reports.at(-1)?.[0]).toMatchObject({
			state: "failed",
			error: "None of the sources of a.gguf delivered it.",
		});
	});

	test("more pins than one question holds are asked in parts", async () => {
		const assets = Array.from({ length: 33 }, (_, index) =>
			asset(index.toString(16).padStart(2, "0").slice(-1), `f${index}.gguf`),
		);
		const fake = device({ ensure: [{ total: 1, present: 1 }], jobs: [[]] });
		const { value } = input(fake.call, assets);
		await ensureModels(value);
		const pins = fake.sent.map(
			(command) => (command.request as { pins: unknown[] }).pins.length,
		);
		expect(pins).toEqual([32, 1]);
	});

	test("while a lock left the device's flags unknown, its answer to Ensure stands for the model store", async () => {
		const fake = device({
			ensure: [
				{ total: 1, present: 0 },
				{ total: 1, present: 1 },
			],
			jobs: [
				[job("a", { state: "fetching", source_index: 0, bytes: 500 })],
				[],
			],
		});
		const { value, reports } = input(fake.call, [asset("a")], {
			features: () => undefined,
		});
		await ensureModels(value);
		expect(fake.kinds()).toEqual(["ensure", "jobs", "jobs", "ensure"]);
		expect(reports.at(-1)?.[0]?.state).toBe("present");
	});

	test("an agent without a model store stops with the missing flag", async () => {
		const fake = device({ ensure: [{ total: 1, present: 0 }], jobs: [[]] });
		const { value } = input(fake.call, [asset("a")], {
			features: () => ({}),
		});
		await expect(ensureModels(value)).rejects.toMatchObject({
			code: "agent_feature",
			message: "model_store",
		});
	});
});
