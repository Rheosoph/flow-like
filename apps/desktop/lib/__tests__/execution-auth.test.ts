import { describe, expect, test, vi } from "vitest";
import { ExecutionAuthBridge } from "../execution-auth";

const signedIn = {
	hub: "https://hub.test",
	subject: "user-a",
	token: "first",
};

function deferred() {
	let resolve!: () => void;
	const promise = new Promise<void>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

describe("native execution authentication", () => {
	test("opens one native-owned session for queued token updates", async () => {
		const opening = deferred();
		const openSession = vi.fn(async () => {
			await opening.promise;
			return "native-session";
		});
		const send = vi.fn().mockResolvedValue(undefined);
		const bridge = new ExecutionAuthBridge(openSession, send);
		expect(() => bridge.sessionId).toThrow("not ready");
		const initial = bridge.update(signedIn);
		const rotated = bridge.update({ ...signedIn, token: "second" });
		await vi.waitFor(() => expect(openSession).toHaveBeenCalledOnce());
		expect(send).not.toHaveBeenCalled();
		opening.resolve();
		await Promise.all([initial, rotated, bridge.ready()]);
		expect(openSession).toHaveBeenCalledOnce();
		expect(bridge.sessionId).toBe("native-session");
		expect(send.mock.calls.map(([update]) => update.sessionId)).toEqual([
			"native-session",
			"native-session",
		]);
	});

	test("retries failed native session initialization before sending credentials", async () => {
		const openSession = vi
			.fn()
			.mockRejectedValueOnce(new Error("IPC unavailable"))
			.mockResolvedValue("native-session");
		const send = vi.fn().mockResolvedValue(undefined);
		const bridge = new ExecutionAuthBridge(openSession, send);
		await expect(bridge.update(signedIn)).rejects.toThrow("IPC unavailable");
		expect(send).not.toHaveBeenCalled();
		await bridge.update(signedIn);
		await bridge.ready();
		expect(openSession).toHaveBeenCalledTimes(2);
		expect(send).toHaveBeenCalledWith({
			...signedIn,
			sessionId: "native-session",
			sequence: 2,
		});
	});

	test("sends rotations and sign-out in order before allowing a run", async () => {
		const first = deferred();
		const send = vi
			.fn()
			.mockReturnValueOnce(first.promise)
			.mockResolvedValue(undefined);
		const bridge = new ExecutionAuthBridge(async () => "session", send);
		const initial = bridge.update(signedIn);
		const rotated = bridge.update({ ...signedIn, token: "second" });
		const signedOut = bridge.update({
			...signedIn,
			subject: null,
			token: null,
		});
		const ready = vi.fn();
		const waiting = bridge.ready().then(ready);
		await vi.waitFor(() => expect(send).toHaveBeenCalledTimes(1));
		expect(ready).not.toHaveBeenCalled();
		first.resolve();
		await Promise.all([initial, rotated, signedOut, waiting]);
		expect(
			send.mock.calls.map(([update]) => [update.token, update.sequence]),
		).toEqual([
			["first", 1],
			["second", 2],
			[null, 3],
		]);
		expect(ready).toHaveBeenCalledOnce();
	});

	test("does not resend an unchanged auth context", async () => {
		const send = vi.fn().mockResolvedValue(undefined);
		const bridge = new ExecutionAuthBridge(async () => "session", send);
		await bridge.update(signedIn);
		await bridge.update({ ...signedIn });
		expect(send).toHaveBeenCalledOnce();
	});

	test("a failed native update blocks a run and can be retried", async () => {
		const send = vi
			.fn()
			.mockRejectedValueOnce(new Error("IPC unavailable"))
			.mockResolvedValue(undefined);
		const bridge = new ExecutionAuthBridge(async () => "session", send);
		await expect(bridge.update(signedIn)).rejects.toThrow("IPC unavailable");
		await expect(bridge.ready()).rejects.toThrow("IPC unavailable");
		await bridge.update(signedIn);
		await bridge.ready();
		expect(send.mock.calls[1][0].sequence).toBe(2);
	});

	test("a failed rotation does not block a queued sign-out", async () => {
		const send = vi
			.fn()
			.mockRejectedValueOnce(new Error("IPC unavailable"))
			.mockResolvedValue(undefined);
		const bridge = new ExecutionAuthBridge(async () => "session", send);
		const failed = bridge.update(signedIn).catch(() => undefined);
		await bridge.update({ ...signedIn, subject: null, token: null });
		await failed;
		await bridge.ready();
		expect(send.mock.calls[1][0].token).toBeNull();
	});

	test("ready includes an auth change queued while it was waiting", async () => {
		const first = deferred();
		const second = deferred();
		const send = vi
			.fn()
			.mockReturnValueOnce(first.promise)
			.mockReturnValueOnce(second.promise);
		const bridge = new ExecutionAuthBridge(async () => "session", send);
		const initial = bridge.update(signedIn);
		const ready = vi.fn();
		const waiting = bridge.ready().then(ready);
		const switched = bridge.update({
			...signedIn,
			subject: "user-b",
			token: "other",
		});
		first.resolve();
		await initial;
		await Promise.resolve();
		expect(ready).not.toHaveBeenCalled();
		second.resolve();
		await Promise.all([switched, waiting]);
		expect(ready).toHaveBeenCalledOnce();
	});
});
