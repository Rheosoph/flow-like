import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "./api-error";
import {
	type IForkJobView,
	awaitForkJob,
	isForkJobView,
	resolveOnlineFork,
} from "./fork-job";
import type { IForkReport, IOnlineForkResponse } from "./schema/app/fork";

const report: IForkReport = {
	id_map: { boards: { src: "dst" } },
	skipped: [],
	warnings: [],
	bytes_copied: 1,
	objects_copied: 1,
};

function job(overrides: Partial<IForkJobView> = {}): IForkJobView {
	return {
		job_id: "job_1",
		source_app_id: "src_app",
		new_app_id: "dst_app",
		status: "QUEUED",
		step: "allocate",
		bytes_copied: 0,
		objects_copied: 0,
		...overrides,
	};
}

const noSleep = async () => {};

describe("isForkJobView", () => {
	test("tells the 202 job row apart from a finished fork", () => {
		const done: IOnlineForkResponse = { new_app_id: "dst_app", report };
		expect(isForkJobView(job())).toBe(true);
		expect(isForkJobView(done)).toBe(false);
	});
});

describe("resolveOnlineFork", () => {
	test("returns a 200 response without polling", async () => {
		const done: IOnlineForkResponse = { new_app_id: "dst_app", report };
		let polls = 0;
		const result = await resolveOnlineFork(done, async () => {
			polls += 1;
			return job();
		});
		expect(result).toBe(done);
		expect(polls).toBe(0);
	});

	test("polls a 202 job until it carries a report", async () => {
		const states = [
			job({ status: "RUNNING", step: "copy_storage" }),
			job({ status: "RUNNING", step: "write_rows" }),
			job({ status: "DONE", step: "done", report }),
		];
		let polls = 0;
		const result = await resolveOnlineFork(
			job(),
			async (jobId) => {
				expect(jobId).toBe("job_1");
				polls += 1;
				return states.shift() ?? job({ status: "FAILED" });
			},
			{ sleep: noSleep },
		);
		expect(polls).toBe(3);
		expect(result).toEqual({ new_app_id: "dst_app", report });
	});
});

describe("awaitForkJob", () => {
	test("surfaces the server's failure reason", async () => {
		await expect(
			awaitForkJob(
				job(),
				async () => job({ status: "FAILED", last_error: "storage gone" }),
				{ sleep: noSleep },
			),
		).rejects.toThrow("storage gone");
	});

	test("reports an aborted job instead of polling a deleted row forever", async () => {
		await expect(
			awaitForkJob(
				job(),
				async () => {
					throw new ApiResponseError({ status: 404, message: "not found" });
				},
				{ sleep: noSleep },
			),
		).rejects.toThrow("aborted before it finished");
	});

	test("does not loop forever when a job finishes without a report", async () => {
		await expect(
			awaitForkJob(job(), async () => job({ status: "DONE" }), {
				sleep: noSleep,
			}),
		).rejects.toThrow("without a report");
	});

	test("gives up on a job that never leaves RUNNING", async () => {
		let clock = 0;
		await expect(
			awaitForkJob(job(), async () => job({ status: "RUNNING" }), {
				pollIntervalMs: 1_000,
				deadlineMs: 5_000,
				sleep: async (ms) => {
					clock += ms;
				},
				now: () => clock,
			}),
		).rejects.toThrow("still RUNNING after 5s");
	});

	test("rides out a transient poll failure instead of failing the fork", async () => {
		let polls = 0;
		const result = await awaitForkJob(
			job(),
			async () => {
				polls += 1;
				if (polls <= 2) throw new Error("network down");
				return job({ status: "DONE", report });
			},
			{ sleep: noSleep },
		);
		expect(polls).toBe(3);
		expect(result).toEqual({ new_app_id: "dst_app", report });
	});

	test("stops after the consecutive-failure budget and rethrows", async () => {
		let polls = 0;
		await expect(
			awaitForkJob(
				job(),
				async () => {
					polls += 1;
					throw new ApiResponseError({ status: 502, message: "bad gateway" });
				},
				{ sleep: noSleep, maxConsecutiveFailures: 3 },
			),
		).rejects.toThrow("bad gateway");
		expect(polls).toBe(3);
	});

	test("backs off between polls up to the cap", async () => {
		const waits: number[] = [];
		const states = [
			job({ status: "RUNNING" }),
			job({ status: "RUNNING" }),
			job({ status: "DONE", report }),
		];
		await awaitForkJob(job(), async () => states.shift() ?? job(), {
			pollIntervalMs: 100,
			maxPollIntervalMs: 200,
			sleep: async (ms) => {
				waits.push(ms);
			},
		});
		expect(waits).toEqual([100, 150, 200]);
	});
});
