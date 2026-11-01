import { describe, expect, it, vi } from "vitest";
import {
	createNativeAutoModeControl,
	type NativeAutoModeRequest,
} from "./native-auto-mode";

const announcement = (revision = 0) =>
	JSON.stringify({
		type: "native_run_control",
		run_id: "run",
		generation: "generation",
		auto_mode_revision: revision,
	});
const settle = async () => {
	for (let i = 0; i < 8; i++) await Promise.resolve();
};

function fixture(
	update: (
		request: NativeAutoModeRequest,
	) => Promise<{ applied: boolean; revision: number }>,
) {
	let enabled = true;
	let notify = () => {};
	const unsubscribe = vi.fn();
	const control = createNativeAutoModeControl({
		runId: "run",
		read: () => enabled,
		subscribe: (changed) => {
			notify = changed;
			return unsubscribe;
		},
		update,
	});
	return {
		control,
		unsubscribe,
		toggle: (value: boolean) => {
			enabled = value;
			notify();
		},
	};
}

describe("native run Auto mode", () => {
	it("syncs the current choice after a toggle during native initialization", async () => {
		const update = vi.fn(async () => ({ applied: true, revision: 1 }));
		const { control, toggle } = fixture(update);
		toggle(false);
		expect(update).not.toHaveBeenCalled();
		control.accept(announcement());
		await settle();
		expect(update).toHaveBeenCalledWith({
			runId: "run",
			generation: "generation",
			expectedRevision: 0,
			autoApprove: false,
		});
		control.dispose();
	});

	it("serializes mode changes and never lets an older acknowledgement restore Auto", async () => {
		let resolve!: (value: { applied: boolean; revision: number }) => void;
		const update = vi
			.fn()
			.mockImplementationOnce(
				() =>
					new Promise((done) => {
						resolve = done;
					}),
			)
			.mockResolvedValue({ applied: true, revision: 2 });
		const { control, toggle } = fixture(update);
		control.accept(announcement());
		toggle(false);
		expect(update).toHaveBeenCalledTimes(1);
		resolve({ applied: true, revision: 1 });
		await settle();
		expect(update).toHaveBeenLastCalledWith({
			runId: "run",
			generation: "generation",
			expectedRevision: 1,
			autoApprove: false,
		});
		expect(update).toHaveBeenCalledTimes(2);
		control.dispose();
	});

	it("retries a revision conflict using the current choice and stops after disposal", async () => {
		const update = vi
			.fn()
			.mockResolvedValueOnce({ applied: false, revision: 7 })
			.mockResolvedValue({ applied: true, revision: 8 });
		const { control, toggle, unsubscribe } = fixture(update);
		toggle(false);
		control.accept(announcement());
		await settle();
		expect(update).toHaveBeenLastCalledWith({
			runId: "run",
			generation: "generation",
			expectedRevision: 7,
			autoApprove: false,
		});
		control.dispose();
		toggle(true);
		expect(unsubscribe).toHaveBeenCalledOnce();
		expect(update).toHaveBeenCalledTimes(2);
	});

	it("ignores another run's control frame", async () => {
		const update = vi.fn(async () => ({ applied: true, revision: 1 }));
		const { control } = fixture(update);
		control.accept(announcement().replace('"run"', '"other"'));
		await settle();
		expect(update).not.toHaveBeenCalled();
		control.dispose();
	});
});
