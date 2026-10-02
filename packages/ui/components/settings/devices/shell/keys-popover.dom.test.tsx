import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	KeySessionSnapshot,
	LiveState,
} from "../../../../lib/device-management/workspace/types";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { KeySessionRow } from "./keys-popover";

const dom = installDom();
const { KeysPopoverView, keyChipOf, keySessionRows } = await import(
	"./keys-popover"
);

const SESSIONS: KeySessionRow[] = [
	{
		deviceId: "d1",
		name: "edge-berlin-01",
		state: "live",
		transport: "direct",
	},
	{ deviceId: "d2", name: "studio-mac-mini", state: "unlocked" },
	{ deviceId: "d3", name: "lab-gpu-02", state: "locked" },
	{ deviceId: "d4", name: "cold-storage-nas", state: "held_elsewhere" },
	{ deviceId: "d5", name: "old-kiosk", state: "stale" },
];

function snapshot(
	deviceId: string,
	state: KeySessionSnapshot["state"],
): KeySessionSnapshot {
	return {
		deviceId,
		state,
		role: "owner",
		grantId: "",
		canSign: false,
		keepUnlocked: false,
		restoredNeedsFreshEndpoint: false,
	};
}

const LIVE: LiveState = {
	kind: "live",
	transport: "websocket",
	expiresAt: 0,
	bootId: "b",
	connectedAt: 0,
};

afterEach(dom.cleanup);
afterAll(dom.restore);

describe("keys popover", () => {
	test("lists every device with keys here: Lock for open sessions, Unlock for closed ones", async () => {
		const calls: string[] = [];
		const { container } = await dom.render(
			<KeysPopoverView
				sessions={SESSIONS}
				onLock={(id) => calls.push(`lock:${id}`)}
				onUnlock={(id) => calls.push(`unlock:${id}`)}
			/>,
		);
		expect(byRole("heading", "Key sessions on this computer")).toBeTruthy();
		const rows = Array.from(container.querySelectorAll("[data-key-session]"));
		expect(rows.map((row) => row.getAttribute("data-key-session"))).toEqual([
			"d1",
			"d2",
			"d3",
			"d4",
			"d5",
		]);
		expect(rows[0]?.textContent).toContain("Live · direct");
		expect(rows[2]?.textContent).toContain("Locked");
		await click(byRole("button", "Lock edge-berlin-01"));
		await click(byRole("button", "Lock studio-mac-mini"));
		await click(byRole("button", "Unlock lab-gpu-02…"));
		expect(byRole("button", "Unlock cold-storage-nas…").textContent).toBe(
			"Use here…",
		);
		expect(calls).toEqual(["lock:d1", "lock:d2", "unlock:d3"]);
		expect(rows[4]?.querySelector("button")).toBeNull();
	});

	test("offers Unlock several…, Lock all and Keys & recovery, and states the idle lock", async () => {
		const calls: string[] = [];
		const { container } = await dom.render(
			<KeysPopoverView
				sessions={SESSIONS}
				onUnlockSeveral={() => calls.push("several")}
				onLockAll={() => calls.push("lock-all")}
				onOpenKeys={() => calls.push("keys")}
			/>,
		);
		await click(byRole("button", "Unlock several…"));
		await click(byRole("button", "Lock all"));
		await click(byRole("button", "Keys & recovery"));
		expect(calls).toEqual(["several", "lock-all", "keys"]);
		expect(container.textContent).toContain(
			"Unlocked devices lock after 30 min unused. Locking clears logs and decrypted history from this window.",
		);
		expect(container.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	});

	test("nothing unlocked: Lock all stays visible, disabled, and sends nothing", async () => {
		let locked = 0;
		await dom.render(
			<KeysPopoverView
				sessions={SESSIONS.slice(2)}
				onLockAll={() => {
					locked += 1;
				}}
			/>,
		);
		const lockAll = byRole("button", "Lock all");
		expect(lockAll.getAttribute("aria-disabled")).toBe("true");
		await click(lockAll);
		expect(locked).toBe(0);
		expect(queryByRole("button", "Lock lab-gpu-02")).toBeNull();
		expect(allByRole("button", /^Unlock (lab|cold)/)).toHaveLength(2);
	});

	test("no keys on this computer: says so", async () => {
		const { container } = await dom.render(<KeysPopoverView sessions={[]} />);
		expect(container.textContent).toContain("No device keys on this computer.");
		expect(queryByRole("list")).toBeNull();
	});
});

describe("key session rows", () => {
	test("the live connection sits on top of open keys", () => {
		expect(keyChipOf(snapshot("d", "unlocked"), LIVE)).toEqual({
			state: "live",
			transport: "relayed",
		});
		expect(
			keyChipOf(snapshot("d", "unlocked"), { ...LIVE, transport: "webrtc" }),
		).toEqual({ state: "live", transport: "direct" });
		expect(keyChipOf(snapshot("d", "unlocked"), { kind: "idle" })).toEqual({
			state: "unlocked",
		});
		expect(
			keyChipOf(snapshot("d", "unlocked"), {
				kind: "reconnecting",
				attempt: 1,
				retryAt: 0,
				cause: { step: "session", code: "closed" },
			}),
		).toEqual({ state: "reconnecting" });
		expect(keyChipOf(snapshot("d", "locked"), LIVE)).toEqual({
			state: "locked",
		});
	});

	test("open sessions come first, then names; unknown devices show a short id", () => {
		const rows = keySessionRows(
			[
				snapshot("aaaaaaaa-1111", "locked"),
				snapshot("d2", "unlocked"),
				snapshot("d1", "locked"),
				snapshot("d9", "none"),
			],
			{
				devices: [
					{ device_id: "d1", name: "alpha" },
					{ device_id: "d2", name: "zulu", display_name: "Zulu box" },
				] as never,
				live: {},
			},
		);
		expect(rows.map((row) => row.name)).toEqual([
			"Zulu box",
			"aaaaaaaa",
			"alpha",
		]);
		expect(rows[0]?.state).toBe("unlocked");
	});
});
