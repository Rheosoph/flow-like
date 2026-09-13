// @vitest-environment happy-dom
import { afterEach, describe, expect, test, vi } from "vitest";
const native = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));
import { executeDeviceCommand } from "@flow-like/flow-like-ui/lib/device-bridge";
import { installNativeDeviceAdapter } from "../device-adapter";

let dispose: (() => void) | undefined;
afterEach(() => {
	dispose?.();
	dispose = undefined;
	vi.unstubAllGlobals();
	vi.clearAllMocks();
});
const fix = () => ({
	geometry: { type: "Point", coordinates: [13.4, 52.5] },
	latitude: 52.5,
	longitude: 13.4,
	accuracy: 8,
	timestamp: Date.now(),
	altitude: null,
	altitudeAccuracy: null,
	speed: null,
	heading: null,
});
const context = {
	appId: "app",
	executionTarget: "local" as const,
	userInitiated: true,
};
function platform(userAgent: string, geolocation?: unknown) {
	vi.stubGlobal("navigator", { userAgent, geolocation });
	vi.stubGlobal("isSecureContext", true);
	Object.defineProperty(document, "visibilityState", {
		configurable: true,
		value: "visible",
	});
	dispose = installNativeDeviceAdapter();
}

describe("native location adapter", () => {
	test("geofence settings forward explicit modes and preserve native permission failures", async () => {
		platform("iPhone");
		const status = {
			authorization: "not_determined",
			backgroundSupported: true,
			monitoredRegionCount: 0,
			maxMonitoredRegions: 20,
		};
		native.invoke.mockResolvedValueOnce(status);
		expect(
			await executeDeviceCommand(
				"location.geofencePermission",
				{ mode: "status" },
				{ appId: "app", executionTarget: "local" },
			),
		).toBe(status);
		expect(native.invoke).toHaveBeenLastCalledWith(
			"native_geofence_permission",
			{ mode: "status" },
		);
		native.invoke.mockRejectedValueOnce(
			"permission_denied: Change Location permission in system settings",
		);
		await expect(
			executeDeviceCommand(
				"location.geofencePermission",
				{ mode: "background" },
				context,
			),
		).rejects.toMatchObject({
			code: "permission_denied",
			message: "Change Location permission in system settings",
		});
	});
	test("unavailable native monitoring returns status data without calling an OS permission API", async () => {
		platform("Windows");
		expect(
			await executeDeviceCommand(
				"location.geofencePermission",
				{ mode: "status" },
				context,
			),
		).toMatchObject({
			backgroundSupported: false,
			error: { code: "unsupported" },
		});
		expect(native.invoke).not.toHaveBeenCalled();
	});
	test("Apple requests carry an ID and deadline and return the unwrapped geometry fix", async () => {
		platform("iPhone");
		native.invoke.mockImplementation(async (command) =>
			command === "native_get_location" ? fix() : undefined,
		);
		const result = await executeDeviceCommand(
			"location.current",
			{ highAccuracy: true, timeoutMs: 2000 },
			context,
		);
		expect(result).toMatchObject({
			geometry: { type: "Point", coordinates: [13.4, 52.5] },
		});
		expect(native.invoke).toHaveBeenCalledWith("native_get_location", {
			requestId: expect.any(String),
			options: expect.objectContaining({
				highAccuracy: true,
				timeoutMs: 2000,
				maximumAgeMs: 0,
				requestDeadline: expect.any(Number),
			}),
		});
	});
	test("aborting the shared context cancels the exact Apple native request", async () => {
		platform("Macintosh");
		let finish!: () => void;
		native.invoke.mockImplementation((command) =>
			command === "native_get_location"
				? new Promise((resolve) => {
						finish = () => resolve(fix());
					})
				: Promise.resolve(),
		);
		const controller = new AbortController();
		const pending = executeDeviceCommand(
			"location.current",
			{},
			{ ...context, signal: controller.signal },
		);
		await vi.waitFor(() =>
			expect(native.invoke).toHaveBeenCalledWith(
				"native_get_location",
				expect.anything(),
			),
		);
		const requestId = native.invoke.mock.calls.find(
			([command]) => command === "native_get_location",
		)![1].requestId;
		controller.abort();
		await expect(pending).rejects.toMatchObject({ code: "cancelled" });
		expect(native.invoke).toHaveBeenCalledWith("native_cancel_location", {
			requestId,
		});
		finish();
	});
	test("preserves native permission errors without falling back to another sensor provider", async () => {
		platform("iPhone");
		native.invoke.mockRejectedValue(
			"permission_denied: Location access was denied",
		);
		await expect(
			executeDeviceCommand("location.current", {}, context),
		).rejects.toMatchObject({
			code: "permission_denied",
			message: "Location access was denied",
		});
		expect(native.invoke).toHaveBeenCalledOnce();
	});
	test("non-Apple desktops use the same cancellable browser watch", async () => {
		let success!: PositionCallback;
		const clearWatch = vi.fn();
		platform("Windows", {
			watchPosition: vi.fn((callback) => {
				success = callback;
				return 4;
			}),
			clearWatch,
		});
		const pending = executeDeviceCommand("location.current", {}, context);
		await vi.waitFor(() => expect(success).toBeTypeOf("function"));
		const value = fix();
		success({
			coords: value,
			timestamp: value.timestamp,
		} as unknown as GeolocationPosition);
		expect(await pending).toMatchObject({ geometry: value.geometry });
		expect(clearWatch).toHaveBeenCalledWith(4);
		expect(native.invoke).not.toHaveBeenCalled();
	});
});
