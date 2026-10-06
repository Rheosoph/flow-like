import { afterEach, expect, test } from "bun:test";
import {
	type DevicePortBridge,
	type DevicePortForward,
	type DeviceServiceTarget,
	type ListenerLease,
	startDevicePortForward,
} from "./tunnel-forward";

function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((yes) => {
		resolve = yes;
	});
	return { promise, resolve };
}

const TARGET: DeviceServiceTarget = {
	deviceId: "device-1",
	placementId: "field-notes",
	serviceId: "hosting",
};

class Bridge implements DevicePortBridge {
	lease: ListenerLease = { id: "forward", token: "owner", port: 45678 };
	listens: { port: number; target: DeviceServiceTarget }[] = [];
	touches = 0;
	closes: ListenerLease[] = [];
	listenGate?: Promise<void>;
	failTouch = false;
	async listen(port: number, target: DeviceServiceTarget) {
		this.listens.push({ port, target });
		await this.listenGate;
		return this.lease;
	}
	async touch() {
		this.touches++;
		if (this.failTouch) throw new Error("IPC disconnected.");
	}
	async close(lease: ListenerLease) {
		this.closes.push(lease);
	}
}

const forwards: DevicePortForward[] = [];
afterEach(async () => {
	for (const forward of forwards.splice(0)) {
		forward.close();
		await forward.closed;
	}
});
const flush = async () => {
	for (let index = 0; index < 20; index++) await Promise.resolve();
};

/** Runs `body` with the heartbeat captured instead of scheduled. */
async function withHeartbeat(body: (beat: () => void) => Promise<void>) {
	const original = globalThis.setInterval;
	let heartbeat: (() => void) | undefined;
	globalThis.setInterval = ((run: () => void) => {
		heartbeat = run;
		return 123;
	}) as typeof setInterval;
	try {
		await body(() => heartbeat?.());
	} finally {
		globalThis.setInterval = original;
	}
}

test("the native listener opens for the service and closes once", async () => {
	const bridge = new Bridge();
	const forward = await startDevicePortForward(
		{ target: TARGET, port: 8080 },
		bridge,
	);
	expect(forward.host).toBe("127.0.0.1");
	expect(forward.port).toBe(45678);
	expect(bridge.listens).toEqual([{ port: 8080, target: TARGET }]);

	forward.close();
	forward.close();
	await forward.closed;
	expect(bridge.closes).toEqual([bridge.lease]);
});

test("the heartbeat renews the lease while the port is open", async () => {
	await withHeartbeat(async (beat) => {
		const bridge = new Bridge();
		const forward = await startDevicePortForward({ target: TARGET }, bridge);
		forwards.push(forward);
		expect(bridge.listens[0]?.port).toBe(0);
		beat();
		await flush();
		beat();
		await flush();
		expect(bridge.touches).toBe(2);
		expect(bridge.closes).toHaveLength(0);
	});
});

test("a failed lease renewal closes the port", async () => {
	await withHeartbeat(async (beat) => {
		const bridge = new Bridge();
		bridge.failTouch = true;
		const forward = await startDevicePortForward({ target: TARGET }, bridge);
		beat();
		await forward.closed;
		expect(bridge.closes).toHaveLength(1);
	});
});

test("aborting the source signal closes an open port", async () => {
	const bridge = new Bridge();
	const signal = new AbortController();
	const forward = await startDevicePortForward(
		{ target: TARGET, signal: signal.signal },
		bridge,
	);
	signal.abort(new Error("Device keys are locked."));
	await forward.closed;
	expect(bridge.closes).toHaveLength(1);
});

test("source cancellation during binding closes the late native listener", async () => {
	const bridge = new Bridge();
	const binding = deferred<void>();
	bridge.listenGate = binding.promise;
	const signal = new AbortController();
	const opening = startDevicePortForward(
		{ target: TARGET, signal: signal.signal },
		bridge,
	);
	await flush();
	signal.abort();
	binding.resolve();
	await expect(opening).rejects.toBeInstanceOf(Error);
	await flush();
	expect(bridge.closes).toHaveLength(1);
});

test("an invalid lease from the desktop is closed and refused", async () => {
	const bridge = new Bridge();
	bridge.lease = { id: "", token: "owner", port: 0 };
	await expect(
		startDevicePortForward({ target: TARGET }, bridge),
	).rejects.toThrow("invalid local listener");
	expect(bridge.closes).toHaveLength(1);
});

test("invalid port selection never calls the native bridge", async () => {
	const bridge = new Bridge();
	for (const port of [-1, 1.5, 65536])
		await expect(
			startDevicePortForward({ port, target: TARGET }, bridge),
		).rejects.toThrow("local port");
	expect(bridge.listens).toHaveLength(0);
});

test("outside the desktop app there is no local port", async () => {
	await expect(startDevicePortForward({ target: TARGET })).rejects.toThrow(
		"Desktop Studio",
	);
});
