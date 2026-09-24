import { afterEach, describe, expect, mock, test } from "bun:test";
import {
	normalizeLocationFix,
	normalizeLocationOptions,
	readBrowserLocation,
	requestCurrentLocation,
} from "./location";

const saved = new Map<string, PropertyDescriptor | undefined>();
function replaceGlobal(name: string, value: unknown) {
	if (!saved.has(name))
		saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
	Object.defineProperty(globalThis, name, {
		configurable: true,
		writable: true,
		value,
	});
}
afterEach(() => {
	for (const [name, descriptor] of saved)
		descriptor
			? Object.defineProperty(globalThis, name, descriptor)
			: Reflect.deleteProperty(globalThis, name);
	saved.clear();
});
const fix = (timestamp = Date.now()) => ({
	latitude: 52.52,
	longitude: 13.405,
	accuracy: 8,
	timestamp,
	altitude: null,
	altitudeAccuracy: null,
	speed: null,
	heading: null,
});
function browser() {
	const doc = Object.assign(new EventTarget(), { visibilityState: "visible" });
	const win = new EventTarget();
	let success!: PositionCallback;
	let failure!: PositionErrorCallback;
	const clearWatch = mock((_id: number) => {});
	const watchPosition = mock(
		(
			ok: PositionCallback,
			error: PositionErrorCallback,
			_options: PositionOptions,
		) => {
			success = ok;
			failure = error;
			return 7;
		},
	);
	replaceGlobal("document", doc);
	replaceGlobal("window", win);
	replaceGlobal("navigator", { geolocation: { watchPosition, clearWatch } });
	replaceGlobal("isSecureContext", true);
	return {
		doc,
		win,
		watchPosition,
		clearWatch,
		position: (value = fix()) =>
			success({
				coords: value,
				timestamp: value.timestamp,
			} as unknown as GeolocationPosition),
		error: (code: number) => failure({ code } as GeolocationPositionError),
	};
}

describe("location values", () => {
	test("normalizes WGS84 longitude first into Geometry(Point) without inventing optional measurements", () => {
		const raw = fix(10_000);
		expect(
			normalizeLocationFix(raw, {
				maximumAgeMs: 0,
				startedAt: 10_000,
				now: 10_001,
			}),
		).toEqual({
			...raw,
			geometry: { type: "Point", coordinates: [13.405, 52.52] },
		});
	});
	test("rejects stale, future, inconsistent, out-of-range and malformed fixes", () => {
		const options = { maximumAgeMs: 500, startedAt: 10_000, now: 10_000 };
		expect(() => normalizeLocationFix(fix(9_499), options)).toThrow(
			"older than requested",
		);
		expect(() =>
			normalizeLocationFix(fix(9_999), { ...options, maximumAgeMs: 0 }),
		).toThrow("older than requested");
		for (const invalid of [
			{ latitude: 91 },
			{ longitude: -181 },
			{ accuracy: -1 },
			{ speed: -1 },
			{ heading: 360 },
			{ altitude: Infinity },
			{ altitudeAccuracy: undefined },
			{ timestamp: 16_000 },
			{ geometry: { type: "Point", coordinates: [52.52, 13.405] } },
		])
			expect(() =>
				normalizeLocationFix({ ...fix(10_000), ...invalid }, options),
			).toThrow();
	});
	test("validates location options before a permission or sensor request", () => {
		expect(normalizeLocationOptions({})).toEqual({
			highAccuracy: false,
			maximumAgeMs: 0,
			timeoutMs: 10_000,
		});
		for (const invalid of [
			{ highAccuracy: null },
			{ maximumAgeMs: Infinity },
			{ maximumAgeMs: -1 },
			{ maximumAgeMs: 300_001 },
			{ timeoutMs: 99 },
			{ timeoutMs: "10000" },
			{ requestDeadline: NaN },
		])
			expect(() => normalizeLocationOptions(invalid)).toThrow();
	});
});

describe("foreground browser location", () => {
	test("uses a one-shot watch, returns normalized data, and clears the watch on success", async () => {
		const mockBrowser = browser();
		const options = normalizeLocationOptions({ highAccuracy: true });
		const pending = readBrowserLocation(options, { appId: "app" });
		expect(mockBrowser.watchPosition).toHaveBeenCalledWith(
			expect.any(Function),
			expect.any(Function),
			expect.objectContaining({ enableHighAccuracy: true, maximumAge: 0 }),
		);
		mockBrowser.position();
		expect((await pending).geometry).toEqual({
			type: "Point",
			coordinates: [13.405, 52.52],
		});
		expect(mockBrowser.clearWatch).toHaveBeenCalledWith(7);
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
	});
	test("waits for a fresh fix after cached readings and transient acquisition failures", async () => {
		const mockBrowser = browser();
		const pending = readBrowserLocation(normalizeLocationOptions({}), {
			appId: "app",
		});
		mockBrowser.position(fix(Date.now() - 10_000));
		mockBrowser.error(2);
		expect(mockBrowser.clearWatch).not.toHaveBeenCalled();
		mockBrowser.position();
		expect(await pending).toMatchObject({
			geometry: { type: "Point", coordinates: [13.405, 52.52] },
		});
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
	});
	test("ends unsuccessful acquisition at its deadline and permits a new request", async () => {
		const mockBrowser = browser();
		const pending = readBrowserLocation(
			normalizeLocationOptions({ timeoutMs: 100 }),
			{ appId: "app" },
		);
		mockBrowser.position(fix(Date.now() - 10_000));
		mockBrowser.error(2);
		await expect(pending).rejects.toMatchObject({ code: "timeout" });
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
		mockBrowser.position();
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
		const retry = readBrowserLocation(normalizeLocationOptions({}), {
			appId: "app",
		});
		mockBrowser.position();
		expect(await retry).toMatchObject({ latitude: 52.52 });
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(2);
	});
	test("invalid fresh coordinates still fail immediately", async () => {
		const mockBrowser = browser();
		const pending = readBrowserLocation(normalizeLocationOptions({}), {
			appId: "app",
		});
		mockBrowser.position({ ...fix(), latitude: 100 });
		await expect(pending).rejects.toMatchObject({ code: "invalid_location" });
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
	});
	test("rejects hidden or insecure contexts without starting a watch", async () => {
		const mockBrowser = browser();
		mockBrowser.doc.visibilityState = "hidden";
		await expect(
			readBrowserLocation(normalizeLocationOptions({}), { appId: "app" }),
		).rejects.toMatchObject({ code: "foreground_required" });
		mockBrowser.doc.visibilityState = "visible";
		replaceGlobal("isSecureContext", false);
		await expect(
			readBrowserLocation(normalizeLocationOptions({}), { appId: "app" }),
		).rejects.toMatchObject({ code: "unsupported" });
		expect(mockBrowser.watchPosition).not.toHaveBeenCalled();
	});
	test("stops on cancellation and ignores a late sensor result", async () => {
		const mockBrowser = browser();
		const controller = new AbortController();
		const pending = readBrowserLocation(normalizeLocationOptions({}), {
			appId: "app",
			signal: controller.signal,
		});
		controller.abort();
		await expect(pending).rejects.toMatchObject({ code: "cancelled" });
		mockBrowser.position();
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
	});
	test("permission-style focus loss keeps a visible location request alive", async () => {
		const mockBrowser = browser();
		const pending = readBrowserLocation(normalizeLocationOptions({}), {
			appId: "app",
		});
		mockBrowser.win.dispatchEvent(new Event("blur"));
		mockBrowser.win.dispatchEvent(new Event("flow-like:device-inactive"));
		expect(mockBrowser.clearWatch).not.toHaveBeenCalled();
		mockBrowser.position();
		expect(await pending).toMatchObject({ latitude: 52.52 });
	});
	test("hiding the screen and native backgrounding each clean up a live watch", async () => {
		for (const kind of [
			"visibilitychange",
			"flow-like:location-background",
			"pagehide",
		]) {
			const mockBrowser = browser();
			const pending = readBrowserLocation(normalizeLocationOptions({}), {
				appId: "app",
			});
			if (kind === "visibilitychange") {
				mockBrowser.doc.visibilityState = "hidden";
				mockBrowser.doc.dispatchEvent(new Event(kind));
			} else mockBrowser.win.dispatchEvent(new Event(kind));
			await expect(pending).rejects.toMatchObject({
				code: "foreground_required",
			});
			expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(1);
		}
	});
	test("permission errors and timeouts clear the watch", async () => {
		const mockBrowser = browser();
		const denied = readBrowserLocation(normalizeLocationOptions({}), {
			appId: "app",
		});
		mockBrowser.error(1);
		await expect(denied).rejects.toMatchObject({ code: "permission_denied" });
		const timedOut = readBrowserLocation(
			normalizeLocationOptions({ timeoutMs: 100 }),
			{ appId: "app" },
		);
		await expect(timedOut).rejects.toMatchObject({ code: "timeout" });
		expect(mockBrowser.clearWatch).toHaveBeenCalledTimes(2);
	});
});

describe("workflow location sharing", () => {
	test("waits for per-app consent before invoking the adapter even when OS permission exists", async () => {
		browser();
		let approve!: () => void;
		const consent = mock(
			async () =>
				new Promise<void>((resolve) => {
					approve = resolve;
				}),
		);
		const adapter = mock(async () => fix());
		const pending = requestCurrentLocation(
			{},
			{ appId: "remote-app", executionTarget: "remote", userInitiated: true },
			adapter,
			consent,
		);
		expect(consent).toHaveBeenCalledWith(
			expect.objectContaining({
				appId: "remote-app",
				executionTarget: "remote",
			}),
		);
		expect(adapter).not.toHaveBeenCalled();
		approve();
		await pending;
		expect(adapter).toHaveBeenCalledTimes(1);
	});
	test("an explicit local Locate click avoids an extra sharing dialog", async () => {
		browser();
		const consent = mock(async () => {});
		await requestCurrentLocation(
			{},
			{ appId: "app", executionTarget: "local", userInitiated: true },
			async () => fix(),
			consent,
		);
		expect(consent).not.toHaveBeenCalled();
	});
	test("denial or cancellation before approval cannot start sensor capture", async () => {
		browser();
		const adapter = mock(async () => fix());
		await expect(
			requestCurrentLocation(
				{},
				{ appId: "app", executionTarget: "remote" },
				adapter,
				async () => {
					throw Error("Denied");
				},
			),
		).rejects.toThrow("Denied");
		const controller = new AbortController();
		let approve!: () => void;
		const pending = requestCurrentLocation(
			{},
			{ appId: "app", executionTarget: "remote", signal: controller.signal },
			adapter,
			async () =>
				new Promise<void>((resolve) => {
					approve = resolve;
				}),
		);
		controller.abort();
		await expect(pending).rejects.toMatchObject({ code: "cancelled" });
		approve();
		await Promise.resolve();
		expect(adapter).not.toHaveBeenCalled();
	});
});
