import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { UnlockStep } from "../../../../lib/device-management/workspace/types";
import { byRole, installDom } from "../testing/dom-harness";

const dom = installDom();
const { ConnectionProgress, connectionSteps, progressReport } = await import(
	"./connection-progress"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

const done = (id: UnlockStep["id"]): UnlockStep => ({ id, state: "done" });
const ids = (steps: readonly UnlockStep[]) => steps.map((step) => step.id);
const rows = () =>
	Array.from(
		byRole("list", "Unlock progress").querySelectorAll("li"),
		(row) => ({
			state: row.getAttribute("data-state"),
			text: row.textContent ?? "",
		}),
	);

const LIVE: UnlockStep[] = [
	done("getting_pass"),
	done("reaching_device"),
	{
		id: "trying_direct",
		state: "skipped",
		detail: { code: "relay_ice_timeout" },
	},
	{ id: "securing", state: "active" },
	{ id: "reading_services", state: "pending" },
];

describe("connection progress", () => {
	test("without Connect live: the two key steps and the encrypted status read", () => {
		expect(ids(connectionSteps([], [], false))).toEqual([
			"unlocking_keys",
			"checking_identity",
			"reading_encrypted_status",
		]);
		const steps = connectionSteps(
			[
				done("unlocking_keys"),
				{ id: "rotating_endpoint", state: "skipped" },
				done("checking_identity"),
				{
					id: "saving_backup",
					state: "skipped",
					detail: { code: "not_requested" },
				},
				{ id: "reading_encrypted_status", state: "active" },
			],
			LIVE,
			false,
		);
		expect(steps).toEqual([
			done("unlocking_keys"),
			done("checking_identity"),
			{ id: "reading_encrypted_status", state: "active" },
		]);
	});

	test("with Connect live: seven steps, and the live ones count only once the keys are open", () => {
		const waiting = connectionSteps(
			[{ id: "unlocking_keys", state: "active" }],
			LIVE,
			true,
		);
		expect(ids(waiting)).toEqual([
			"unlocking_keys",
			"checking_identity",
			"getting_pass",
			"reaching_device",
			"trying_direct",
			"securing",
			"reading_services",
		]);
		expect(waiting.slice(1).every((step) => step.state === "pending")).toBe(
			true,
		);

		const open = connectionSteps(
			[done("unlocking_keys"), done("checking_identity")],
			LIVE,
			true,
		);
		expect(open.slice(2)).toEqual(LIVE);
	});

	test("a renewed connection identity and a saved backup get a line only when they happened", () => {
		const steps = connectionSteps(
			[
				done("unlocking_keys"),
				{
					id: "rotating_endpoint",
					state: "done",
					detail: { code: "fresh_endpoint" },
				},
				done("checking_identity"),
				{
					id: "saving_backup",
					state: "failed",
					detail: { code: "backup_limit" },
				},
				done("reading_encrypted_status"),
			],
			[],
			false,
		);
		expect(ids(steps)).toEqual([
			"unlocking_keys",
			"checking_identity",
			"rotating_endpoint",
			"saving_backup",
			"reading_encrypted_status",
		]);
	});

	test("every step reads as a sentence: done, running, relayed, failed", async () => {
		await dom.render(
			<ConnectionProgress
				steps={[
					done("unlocking_keys"),
					{
						id: "rotating_endpoint",
						state: "done",
						detail: { code: "fresh_endpoint" },
					},
					done("checking_identity"),
					{
						id: "saving_backup",
						state: "failed",
						detail: { code: "backup_local_only" },
					},
					...LIVE,
				]}
			/>,
		);
		expect(rows()).toEqual([
			{ state: "pass", text: "Keys unlocked" },
			{
				state: "pass",
				text: "This computer got a new connection identity. Encrypted metric groups will need the owner's re-approval.",
			},
			{ state: "pass", text: "Device identity matches" },
			{
				state: "warn",
				text: "The backup couldn't be uploaded. It stays on this computer and uploads when the hub is reachable.",
			},
			{ state: "pass", text: "Connection pass received" },
			{ state: "pass", text: "Device answered" },
			{
				state: "warn",
				text: "Connected through the hub: a direct connection timed out.",
			},
			{ state: "active", text: "Securing the connection" },
			{ state: "pending", text: "Reading services" },
		]);
	});

	test("a failed step says why in plain words, and a failed status read is only a warning", async () => {
		await dom.render(
			<ConnectionProgress
				steps={[
					{
						id: "unlocking_keys",
						state: "failed",
						detail: { code: "wrong_password" },
					},
					{
						id: "getting_pass",
						state: "failed",
						detail: { code: "invalid_admission" },
					},
					{ id: "securing", state: "failed" },
					{ id: "reading_encrypted_status", state: "failed" },
					{
						id: "reading_services",
						state: "skipped",
						detail: { code: "slots_in_use" },
					},
				]}
			/>,
		);
		expect(rows()).toEqual([
			{
				state: "fail",
				text: "That password doesn't open the keys for this device on this computer.",
			},
			{
				state: "fail",
				text: "The hub gave an invalid connection pass. Try again; if it repeats, contact the hub operator.",
			},
			{ state: "fail", text: "Securing the connection didn't finish." },
			{
				state: "warn",
				text: "The encrypted status couldn't be read yet. It's retried in the background.",
			},
			{
				state: "warn",
				text: "All connection slots for your access are in use, so the oldest one was closed.",
			},
		]);
		expect(byRole("list", "Unlock progress").textContent).not.toMatch(
			/wrong_password|invalid_admission|slots_in_use/,
		);
	});

	test("the copied report keeps the step and detail codes", () => {
		expect(
			progressReport([
				done("unlocking_keys"),
				{
					id: "trying_direct",
					state: "skipped",
					detail: { code: "relay_ice_timeout" },
				},
			]),
		).toEqual([
			"unlocking_keys done",
			"trying_direct skipped relay_ice_timeout",
		]);
	});
});
