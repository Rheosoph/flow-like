import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
	TunnelKind,
	decodeTunnelFrame,
	encodeTunnelFrame,
	readTunnelJson,
	tunnelJson,
} from "./tunnel-protocol";

const fixture = JSON.parse(
	readFileSync(
		new URL(
			"../../../device-protocol/fixtures/tunnel-v1.json",
			import.meta.url,
		),
		"utf8",
	),
);
const encode = (value: unknown, stream = 1) =>
	encodeTunnelFrame({
		kind: TunnelKind.OpenData,
		stream,
		sequence: 0n,
		body: tunnelJson(value),
	});

describe("tunnel data stream wire contract", () => {
	test("service opens default to raw TCP and allow explicit HTTP mode only", () => {
		const open = (value: unknown) =>
			encodeTunnelFrame({
				kind: TunnelKind.Open,
				stream: 1,
				sequence: 0n,
				body: tunnelJson(value),
			});
		const target = { placement_id: "placement", service_id: "hosting" };
		for (const value of [
			target,
			{ ...target, mode: "tcp" },
			{ ...target, mode: "http" },
		])
			expect(readTunnelJson(decodeTunnelFrame(open(value)).body)).toEqual(
				value,
			);
		for (const value of [
			{ ...target, mode: null },
			{ ...target, mode: "https" },
			{ ...target, host: "127.0.0.1" },
		])
			expect(() => open(value)).toThrow();
	});
	test("matches native request and raw artifact fixtures", () => {
		for (const name of ["open_data_request", "open_data_artifact"]) {
			const entry = fixture[name];
			const bytes = Uint8Array.from(Buffer.from(entry.frame_hex, "hex"));
			const frame = decodeTunnelFrame(bytes);
			expect(frame.kind).toBe(TunnelKind.OpenData);
			expect(frame.stream).toBe(fixture.stream_id);
			expect(frame.sequence).toBe(BigInt(fixture.sequence));
			expect(readTunnelJson(frame.body)).toEqual(entry.body);
			expect(encodeTunnelFrame(frame)).toEqual(bytes);
		}
	});

	test("rejects malformed request scope, lifetime and extra envelope fields", () => {
		const valid = fixture.open_data_request.body;
		for (const fields of [
			{ device_id: "device/path" },
			{ operation_id: "" },
			{ issued_at: 0 },
			{ expires_at: 100 },
			{ expires_at: 401 },
			{ command: [] },
			{ command: {} },
			{ host: "127.0.0.1" },
		])
			expect(() =>
				encode({ ...valid, request: { ...valid.request, ...fields } }),
			).toThrow();
		expect(() => encode({ ...valid, extra: true })).toThrow();
		expect(() => encode(valid, 0)).toThrow();
		expect(() => encode(valid, 2)).toThrow();
		expect(() => encode(valid)).not.toThrow();
	});

	test("binds artifact project, transfer, file and bounded offset", () => {
		const valid = fixture.open_data_artifact.body;
		for (const fields of [
			{ project_id: ".." },
			{ project_id: "project/path" },
			{ transfer_id: "not-a-transfer" },
			{ transfer_id: valid.transfer_id.toUpperCase() },
			{ file_index: -1 },
			{ file_index: 0x1_0000_0000 },
			{ file_index: 1.5 },
			{ offset: -1 },
			{ offset: 4 * 1024 * 1024 * 1024 + 1 },
			{ host: "127.0.0.1" },
		])
			expect(() => encode({ ...valid, ...fields })).toThrow();
		const { file_index: _, ...manifest } = valid;
		expect(() => encode(manifest)).not.toThrow();
		expect(() => encode({ ...valid, file_index: 0, offset: 0 })).not.toThrow();
		expect(() => encode({ ...valid, kind: "unknown" })).toThrow();
	});

	test("model gateway opens name the target and no placement or service", () => {
		const open = (value: unknown) =>
			encodeTunnelFrame({
				kind: TunnelKind.Open,
				stream: 1,
				sequence: 0n,
				body: tunnelJson(value),
			});
		for (const value of [
			{ mode: "http", target: "model_gateway" },
			{ target: "model_gateway" },
		])
			expect(readTunnelJson(decodeTunnelFrame(open(value)).body)).toEqual(
				value,
			);
		for (const value of [
			{ target: "service" },
			{ target: "model_gateway", mode: "https" },
			{ target: "model_gateway", placement_id: "placement" },
			{ target: "model_gateway", service_id: "hosting" },
			{
				target: "model_gateway",
				placement_id: "placement",
				service_id: "hosting",
			},
			{ target: "model_gateway", host: "127.0.0.1" },
		])
			expect(() => open(value)).toThrow();
	});

	test("model asset pushes name a canonical job and a bounded offset", () => {
		const valid = {
			kind: "model_asset",
			job_id: "12345678-1234-1234-1234-123456789abc",
			offset: 0,
		};
		expect(readTunnelJson(decodeTunnelFrame(encode(valid)).body)).toEqual(
			valid,
		);
		expect(() =>
			encode({ ...valid, offset: 64 * 1024 * 1024 * 1024 }),
		).not.toThrow();
		for (const fields of [
			{ job_id: valid.job_id.toUpperCase() },
			{ job_id: "job" },
			{ offset: -1 },
			{ offset: 1.5 },
			{ offset: 64 * 1024 * 1024 * 1024 + 1 },
			{ project_id: "project" },
		])
			expect(() => encode({ ...valid, ...fields })).toThrow();
		const { offset: _, ...missing } = valid;
		expect(() => encode(missing)).toThrow();
	});
});
