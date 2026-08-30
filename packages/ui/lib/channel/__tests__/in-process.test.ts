import { beforeEach, describe, expect, it, mock } from "bun:test";
import type { IChannelHandle } from "../../schema/channel";

const invoke = mock(async (_command: string, _args?: unknown) => "delivered");
mock.module("@tauri-apps/api/core", () => ({ invoke }));

const { replyToChannel } = await import("../index");

const handle: IChannelHandle = {
	channel_id: "run-desktop",
	request_id: "req-7",
	expires_at: 4_102_444_800,
	transport: { type: "in_process" },
};

beforeEach(() => {
	invoke.mockClear();
	invoke.mockImplementation(async () => "delivered");
});

describe("in_process channel push", () => {
	it("invokes channel_push with the envelope and accepts 'delivered'", async () => {
		await replyToChannel(handle, { approved: true });

		expect(invoke).toHaveBeenCalledTimes(1);
		expect(invoke.mock.calls[0]).toEqual([
			"channel_push",
			{
				push: {
					channel_id: "run-desktop",
					request_id: "req-7",
					kind: "reply",
					value: { approved: true },
				},
			},
		]);
	});

	it.each([
		"unknown_channel",
		"unknown_request",
		"expired",
		"duplicate",
		"full",
	])("throws when the host answers '%s'", async (result) => {
		invoke.mockImplementation(async () => result);
		await expect(replyToChannel(handle, 1)).rejects.toThrow(
			new RegExp(`request 'req-7' on channel 'run-desktop'.*${result}`),
		);
	});
});
