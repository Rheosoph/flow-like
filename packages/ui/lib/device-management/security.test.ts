import { describe, expect, test } from "bun:test";
import { base64url, unbase64url, withPassword } from "./crypto";
import { accountStorageKey, controllerBackup, sameCheckpoint } from "./storage";
import {
	FrameQueue,
	matchesOperationResponse,
	signalingUrl,
} from "./transport";

describe("browser management boundaries", () => {
	test("operation queries bind the stored operation ID without relaxing other replies", () => {
		const query = { type: "operation", operation_id: "stored" };
		expect(
			matchesOperationResponse(query, "query", {
				operation_id: "stored",
				state: "completed",
			}),
		).toBe(true);
		expect(
			matchesOperationResponse(query, "query", {
				operation_id: "query",
				state: "rejected",
			}),
		).toBe(true);
		expect(
			matchesOperationResponse(query, "query", {
				operation_id: "query",
				state: "completed",
			}),
		).toBe(false);
		expect(
			matchesOperationResponse(query, "query", {
				operation_id: "other",
				state: "completed",
			}),
		).toBe(false);
		expect(
			matchesOperationResponse({ type: "start" }, "query", {
				operation_id: "stored",
				state: "completed",
			}),
		).toBe(false);
	});
	test("account, issuer, hub and profile form unambiguous storage scopes", () => {
		const base = {
			issuer: "issuer:a",
			account: "user",
			apiOrigin: "https://hub.test",
			profileId: "default",
		};
		const key = accountStorageKey(base);
		for (const update of [
			{ issuer: "issuer" },
			{ account: "a:user" },
			{ apiOrigin: "https://other.test" },
			{ profileId: "other" },
		])
			expect(accountStorageKey({ ...base, ...update })).not.toBe(key);
		expect(() => accountStorageKey({ ...base, account: "" })).toThrow();
		const checkpoint = {
			store_id: Array(32).fill(1),
			revision: 3,
			digest: "digest",
		};
		expect(
			sameCheckpoint(checkpoint, {
				...checkpoint,
				store_id: [...checkpoint.store_id],
			}),
		).toBe(true);
		for (const update of [
			{ revision: 2 },
			{ digest: "fork" },
			{ store_id: Array(32).fill(2) },
		])
			expect(sameCheckpoint(checkpoint, { ...checkpoint, ...update })).toBe(
				false,
			);
		expect(sameCheckpoint(undefined, null)).toBe(true);
		expect(sameCheckpoint(checkpoint, null)).toBe(false);
	});
	test("password buffers are cleared after successful and failed unlock callbacks", async () => {
		let retained: Uint8Array | undefined;
		await withPassword("local secret", (bytes) => {
			retained = bytes;
			expect(bytes.some((byte) => byte !== 0)).toBe(true);
		});
		expect(retained?.every((byte) => byte === 0)).toBe(true);
		await expect(
			withPassword("local secret", (bytes) => {
				retained = bytes;
				throw new Error("unlock rejected");
			}),
		).rejects.toThrow("unlock rejected");
		expect(retained?.every((byte) => byte === 0)).toBe(true);
	});
	test("canonical bounded encodings and signaling URLs reject alternate credential paths", () => {
		const bytes = Uint8Array.from([0, 1, 254, 255]);
		expect(unbase64url(base64url(bytes))).toEqual(bytes);
		for (const value of ["YQ==", "YR", "+/"])
			expect(() => unbase64url(value)).toThrow();
		expect(() => unbase64url(base64url(new Uint8Array(33)), 32)).toThrow();
		expect(signalingUrl("wss://signal.test/ws/devices")).toBe(
			"wss://signal.test/ws/devices",
		);
		for (const value of [
			"ws://signal.test/ws/devices",
			"wss://user:secret@signal.test/ws/devices",
			"wss://signal.test/ws/devices?token=secret",
			"wss://signal.test/ws/devices#other",
			"wss://signal.test/other",
		])
			expect(() => signalingUrl(value)).toThrow();
	});
	test("queues retain exact order and reject overflow, parallel reads and closed sessions", async () => {
		const queue = new FrameQueue<number>();
		queue.push(1);
		queue.push(2);
		expect(await queue.next()).toBe(1);
		expect(await queue.next()).toBe(2);
		const pending = queue.next();
		await expect(queue.next()).rejects.toThrow("ordered reader");
		queue.push(3);
		expect(await pending).toBe(3);
		for (let value = 0; value < 33; value++) queue.push(value);
		await expect(queue.next()).rejects.toThrow("bound");
		const closed = new FrameQueue<number>();
		const waiting = closed.next();
		closed.close();
		await expect(waiting).rejects.toThrow("closed");
	});
	test("backup imports select only protected fields and reject another hub or device", () => {
		const scope = {
			issuer: "issuer",
			account: "user",
			apiOrigin: "https://hub.test",
			profileId: "profile",
		};
		const record = {
			version: 1,
			apiOrigin: scope.apiOrigin,
			deviceId: "device",
			controllerPublic: { device_id: "device" },
			controllerVault: Array(64).fill(1),
			manifestJws: "signed",
			grantId: "owner",
			plaintextSecret: "must be discarded",
		};
		const imported = controllerBackup(JSON.stringify(record), scope, "device");
		expect(imported.controllerVault).toBeInstanceOf(Uint8Array);
		expect(imported).not.toHaveProperty("plaintextSecret");
		expect(() =>
			controllerBackup(JSON.stringify(record), scope, "other"),
		).toThrow();
		expect(() =>
			controllerBackup(
				JSON.stringify({ ...record, apiOrigin: "https://other.test" }),
				scope,
				"device",
			),
		).toThrow();
		expect(() =>
			controllerBackup(
				JSON.stringify({ ...record, controllerVault: [1, 2] }),
				scope,
				"device",
			),
		).toThrow();
	});
});
