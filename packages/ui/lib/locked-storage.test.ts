import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type {
	ILockedItemDetail,
	ILockedUploadTicket,
} from "../state/backend-state/locked-storage-state";
import {
	type ILockedStorageRequest,
	type LockedStorageAction,
	LockedStorageClient,
	sha256Hex,
} from "./locked-storage";

const originalXhr = globalThis.XMLHttpRequest;
const FILE_NAME = "Übersicht (2)#1.pdf";
const DATA_SHA256 =
	"3a6eb0790f39ac87c94f3856b2dd2c5d110e6811602261a9a923d3bb23adc8b7";

class PutRequest {
	static last: PutRequest;
	static status = 200;
	upload = new EventTarget();
	events = new EventTarget();
	status = PutRequest.status;
	statusText = "";
	method = "";
	url = "";
	headers = new Map<string, string>();
	body?: XMLHttpRequestBodyInit;
	constructor() {
		PutRequest.last = this;
	}
	addEventListener(type: string, callback: EventListener) {
		this.events.addEventListener(type, callback);
	}
	open(method: string, url: string) {
		this.method = method;
		this.url = url;
	}
	setRequestHeader(key: string, value: string) {
		this.headers.set(key, value);
	}
	send(body: XMLHttpRequestBodyInit) {
		this.body = body;
		queueMicrotask(() => this.events.dispatchEvent(new Event("load")));
	}
	abort() {
		this.events.dispatchEvent(new Event("abort"));
	}
}

const ticket: ILockedUploadTicket = {
	item: { id: "item-1" } as ILockedUploadTicket["item"],
	version: { id: "version-1" } as ILockedUploadTicket["version"],
	upload: {
		url: "https://locked.s3.example/apps/app/project/item-1/version-1?X-Amz-Signature=sig",
		method: "PUT",
		headers: {
			"content-type": "application/pdf",
			"x-amz-checksum-sha256": "Om6weQ85rIfJTzhWst0sXREOaBFgImGpqSPTuyOtyLc=",
			"x-amz-object-lock-mode": "COMPLIANCE",
			"x-amz-object-lock-retain-until-date": "2036-10-21T00:00:00Z",
		},
		expires_at: "2026-10-21T12:10:00Z",
	},
};
const detail = { item: ticket.item, versions: [] } as ILockedItemDetail;

function recordingTransport() {
	const calls: Array<{
		appId: string;
		action: LockedStorageAction;
		request: ILockedStorageRequest;
	}> = [];
	const transport = mock(
		async (
			appId: string,
			action: LockedStorageAction,
			request: ILockedStorageRequest,
		) => {
			calls.push({ appId, action, request });
			if (action === "upload") return ticket;
			if (action === "commit") return detail;
			return {};
		},
	);
	return {
		calls,
		client: new LockedStorageClient(transport as never),
	};
}

beforeEach(() => {
	PutRequest.status = 200;
	globalThis.XMLHttpRequest = PutRequest as unknown as typeof XMLHttpRequest;
});

afterEach(() => {
	globalThis.XMLHttpRequest = originalXhr;
});

describe("locked uploads", () => {
	test("hash, reserve, PUT with the signed headers, then commit", async () => {
		const { calls, client } = recordingTransport();
		const file = new File(["data"], FILE_NAME, { type: "application/pdf" });
		const phases: string[] = [];

		const result = await client.uploadLockedFile("app", file, {
			folder: "Übersicht",
			scope: "user",
			retentionDays: 30,
			deleteAfterRetention: true,
			comment: "  signed contract  ",
			ifExists: "new_version",
			onProgress: ({ phase }) => {
				if (phases.at(-1) !== phase) phases.push(phase);
			},
		});

		expect(result).toBe(detail);
		expect(calls.map((call) => call.action)).toEqual(["upload", "commit"]);
		expect(calls[0]?.request.body).toEqual({
			file_name: FILE_NAME,
			content_type: "application/pdf",
			size: 4,
			sha256: DATA_SHA256,
			retention_days: 30,
			delete_after_retention: true,
			comment: "signed contract",
			metadata: undefined,
			scope: "user",
			folder: "Übersicht",
			name: undefined,
			if_exists: "new_version",
		});
		expect(PutRequest.last.method).toBe("PUT");
		expect(PutRequest.last.url).toBe(ticket.upload.url);
		expect(Object.fromEntries(PutRequest.last.headers)).toEqual(
			ticket.upload.headers,
		);
		expect(PutRequest.last.body).toBe(file);
		expect(calls[1]?.request.body).toEqual({
			item_id: "item-1",
			version_id: "version-1",
		});
		expect(phases).toEqual(["hashing", "uploading", "committing"]);
	});

	test("a new version names the item and drops path fields", async () => {
		const { calls, client } = recordingTransport();
		await client.uploadLockedFile("app", new File(["data"], FILE_NAME), {
			itemId: "item-1",
			folder: "ignored",
			ifExists: "error",
		});
		const body = calls[0]?.request.body as Record<string, unknown>;
		expect(body.item_id).toBe("item-1");
		expect(body).not.toHaveProperty("folder");
		expect(body).not.toHaveProperty("if_exists");
	});

	test("a failed PUT aborts the pending version and never commits", async () => {
		PutRequest.status = 403;
		const { calls, client } = recordingTransport();
		await expect(
			client.uploadLockedFile("app", new File(["data"], FILE_NAME)),
		).rejects.toThrow("Upload failed with status 403");
		expect(calls.map((call) => call.action)).toEqual(["upload", "abort"]);
		expect(calls[1]?.request.body).toEqual({
			item_id: "item-1",
			version_id: "version-1",
		});
	});

	test("an aborted upload stops before reserving", async () => {
		const { calls, client } = recordingTransport();
		const controller = new AbortController();
		controller.abort();
		await expect(
			client.uploadLockedFile("app", new File(["data"], FILE_NAME), {
				signal: controller.signal,
			}),
		).rejects.toThrow("Upload cancelled");
		expect(calls).toEqual([]);
	});
});

describe("locked storage calls", () => {
	test("routes each action with its contract body", async () => {
		const { calls, client } = recordingTransport();
		await client.getLockedStorageStatus("app");
		await client.downloadLockedItem("app", "item-1");
		await client.downloadLockedItem("app", "item-1", 2);
		await client.setLockedRetention(
			"app",
			"item-1",
			"version-1",
			"2040-01-01T00:00:00.000Z",
		);
		await client.setLockedLegalHold("app", "item-1", "version-1", true);
		await client.setLockedAutoDelete("app", "item-1", "version-1", false);
		await client.purgeLockedItem("app", "item-1");
		await client.purgeLockedItem("app", "item-1", "version-1");
		expect(
			calls.map(({ action, request }) => [
				action,
				request.method,
				request.body,
			]),
		).toEqual([
			["status", "GET", undefined],
			["download", "POST", { item_id: "item-1", version: undefined }],
			["download", "POST", { item_id: "item-1", version: 2 }],
			[
				"retention",
				"POST",
				{
					item_id: "item-1",
					version_id: "version-1",
					retain_until: "2040-01-01T00:00:00.000Z",
				},
			],
			[
				"legal-hold",
				"POST",
				{ item_id: "item-1", version_id: "version-1", enabled: true },
			],
			[
				"auto-delete",
				"POST",
				{ item_id: "item-1", version_id: "version-1", enabled: false },
			],
			["purge", "POST", { item_id: "item-1", version_id: undefined }],
			["purge", "POST", { item_id: "item-1", version_id: "version-1" }],
		]);
	});
});

describe("sha256Hex", () => {
	test("hashes the exact bytes as lowercase hex", async () => {
		expect(await sha256Hex(new Blob(["data"]))).toBe(DATA_SHA256);
	});

	test("streams files too large to buffer and reports progress", async () => {
		const bytes = new Uint8Array(64 * 1024 * 1024 + 3).fill(7);
		const expected = Array.from(
			new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
			(byte) => byte.toString(16).padStart(2, "0"),
		).join("");
		const progress: number[] = [];
		const blob = new Blob([bytes]);
		expect(
			await sha256Hex(blob, { onBytes: (loaded) => progress.push(loaded) }),
		).toBe(expected);
		expect(progress.length).toBeGreaterThan(1);
		expect(progress.at(-1)).toBe(blob.size);
	});
});
