import { describe, expect, test } from "bun:test";
import {
	EVENT_DEFINITIONS,
	sinkSupportsEventExecution,
} from "./event-definitions";
import {
	geofenceCenter,
	geofenceCenterFields,
	validateGeolocationEvent,
} from "./geolocation-event";

describe("geolocation Event configuration", () => {
	test("uses a Point center in longitude-latitude order while preserving the sink wire format", () => {
		const point = { type: "Point", coordinates: [13.405, 52.52] };
		expect(geofenceCenterFields(point)).toEqual({
			latitude: 52.52,
			longitude: 13.405,
		});
		expect(geofenceCenter({ latitude: 52.52, longitude: 13.405 })).toEqual(
			point,
		);
		expect(() =>
			geofenceCenterFields({
				type: "LineString",
				coordinates: [
					[13, 52],
					[14, 53],
				],
			}),
		).toThrow();
		expect(() =>
			geofenceCenterFields({ type: "Point", coordinates: [13, 52, 100] }),
		).toThrow();
	});
	test("requires a valid region and explicit boolean background opt-in", () => {
		const config = EVENT_DEFINITIONS.events_location.configs.geolocation;
		expect(config.background).toBe(false);
		expect(validateGeolocationEvent(config)).toBeUndefined();
		for (const invalid of [
			{ latitude: null },
			{ longitude: 181 },
			{ radius: 0 },
			{ radius: 100001 },
			{ trigger_on: "Nearby" },
			{ background: "true" },
		])
			expect(validateGeolocationEvent({ ...config, ...invalid })).toBeTypeOf(
				"string",
			);
	});
	test("allows a native local sensor to dispatch a Remote Event without advertising a server-side sensor", () => {
		const sink =
			EVENT_DEFINITIONS.events_location.sinkAvailability!.geolocation;
		expect(sink.availability).toBe("local");
		expect(sinkSupportsEventExecution(sink, "Remote", true)).toBe(true);
		expect(sinkSupportsEventExecution(sink, "Local", true)).toBe(true);
		expect(sinkSupportsEventExecution(sink, "Remote", false)).toBe(false);
		expect(
			sinkSupportsEventExecution({ availability: "local" }, "Remote", true),
		).toBe(false);
	});
});
