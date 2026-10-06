import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import type { DevicesT } from "../primitives/area-context";
import {
	type ActionCopy,
	MODELS_WRITE_KINDS,
	type ModelsWriteKind,
	cancelJobCopy,
	configureCopy,
	ensureCopy,
	installCopy,
	installRuntimeCopy,
	isModelsWriteKind,
	loadCopy,
	removeCopy,
	removeRuntimeCopy,
	residencyCopy,
	trayTitle,
	unloadCopy,
	updateRuntimeCopy,
} from "./action-copy";

const t = getI18n().getFixedT("en", "devices") as DevicesT;

const MODEL = "Qwen3-8B Q4_K_M";
const DEVICE = "edge-berlin-01";
const FILE = "gemma-3-4b-it-Q4_K_M.gguf";
const RUNTIME = "llama.cpp for CPU";
const names = { model: MODEL, device: DEVICE };

/** A wire value, a key or an unfilled placeholder (R3); the names are the user's own text. */
const leaks = (sentence: string) =>
	/_|\{\{|undefined|NaN|devices:/.test(
		sentence.replaceAll(MODEL, "NAME").replaceAll(FILE, "NAME"),
	);

const ALWAYS_ON = { mode: "always_on" } as const;
const ON_DEMAND = {
	mode: "on_demand",
	idle_unload_after_seconds: 900,
} as const;
const KEPT_OFF = { mode: "pinned_off" } as const;
const configured = (
	loaded: boolean,
	settingsChanged: boolean,
	residency: Parameters<typeof configureCopy>[2] = ALWAYS_ON,
) => configureCopy(t, { ...names, loaded, settingsChanged }, residency);

const COPIES = {
	install: installCopy(t, { ...names, size: 5_027_784_064, files: 1 }),
	configure: configured(true, true),
	load: loadCopy(t, names),
	unload: unloadCopy(t, { ...names, loadsOnRequest: false }),
	remove: removeCopy(t, names),
	ensure: ensureCopy(t, { device: DEVICE, pins: 2 }),
	install_runtime: installRuntimeCopy(t, {
		runtime: RUNTIME,
		device: DEVICE,
		size: 16_734_586,
	}),
	remove_runtime: removeRuntimeCopy(t, { runtime: RUNTIME, device: DEVICE }),
	cancel_job: cancelJobCopy(t, { file: FILE, device: DEVICE }),
} satisfies Record<ModelsWriteKind, ActionCopy>;

const sentences = (copy: ActionCopy) => [
	copy.label,
	copy.rows.what,
	copy.rows.who,
	copy.rows.when,
	copy.rows.undo.text,
	...(copy.rows.stays ? [copy.rows.stays] : []),
	...(copy.checkLabel ? [copy.checkLabel] : []),
];

describe("action copy (R8)", () => {
	test("every write names its verb and object, fills its rows and leaks nothing", () => {
		for (const kind of MODELS_WRITE_KINDS) {
			const copy = COPIES[kind];
			for (const sentence of sentences(copy)) {
				if (typeof sentence !== "string")
					throw new Error(
						`${kind} must produce text for every consequence row`,
					);
				expect([kind, sentence.length > 3]).toEqual([kind, true]);
				expect([kind, leaks(sentence)]).toEqual([kind, false]);
			}
		}
		expect(
			Object.fromEntries(
				MODELS_WRITE_KINDS.map((kind) => [kind, COPIES[kind].label]),
			),
		).toEqual({
			install: `Add ${MODEL}`,
			configure: `Change the settings of ${MODEL}`,
			load: `Load ${MODEL}`,
			unload: `Unload ${MODEL}`,
			remove: `Remove ${MODEL}`,
			ensure: "Prepare the app's models",
			install_runtime: `Install ${RUNTIME}`,
			remove_runtime: `Remove ${RUNTIME}`,
			cancel_job: `Cancel the download of ${FILE}`,
		});
	});

	test("deleting files asks for an acknowledgement; changes that cut users off are danger", () => {
		const checked = MODELS_WRITE_KINDS.filter(
			(kind) => COPIES[kind].strength === "check",
		);
		expect(checked).toEqual(["remove", "remove_runtime"]);
		for (const kind of MODELS_WRITE_KINDS)
			expect([kind, COPIES[kind].checkLabel !== undefined]).toEqual([
				kind,
				COPIES[kind].strength === "check",
			]);
		expect(COPIES.remove.checkLabel).toBe(`Let ${DEVICE} delete its files`);
		expect(
			MODELS_WRITE_KINDS.filter((kind) => COPIES[kind].tone === "danger"),
		).toEqual(["unload", "remove", "remove_runtime", "cancel_job"]);
		expect(COPIES.remove.rows.undo.reversible).toBe(false);
		expect(COPIES.ensure.rows.undo.reversible).toBeNull();
		expect(COPIES.load.rows.undo.reversible).toBe(true);
	});

	test("the rows follow the model: its files, whether it is loaded, how it loads", () => {
		expect(COPIES.install.rows.what).toBe(
			`${DEVICE} downloads 4.7 GiB in 1 file from its sources and checks it against its fingerprint.`,
		);
		expect(
			installCopy(t, { ...names, size: 1_000, files: 3 }).rows.what,
		).toContain("in 3 files from their sources and checks each");
		expect(configured(false, true).rows.what).toBe(
			`${MODEL} uses the new settings the next time it loads.`,
		);
		expect(COPIES.unload.rows.who).toBe(
			"Apps and people that use it get an error until it is loaded again.",
		);
		expect(unloadCopy(t, { ...names, loadsOnRequest: true }).rows.who).toBe(
			"The next request loads it again and waits until it has started.",
		);
		expect(COPIES.install_runtime.rows.what).toContain(
			`the signed ${RUNTIME} runtime (16.0 MiB)`,
		);
		expect(COPIES.ensure.rows.what).toBe(
			`${DEVICE} downloads the files of the 2 models the app uses, unless it has them already.`,
		);
		expect(ensureCopy(t, { device: DEVICE, pins: 1 }).rows.what).toContain(
			"the files of 1 model the app uses",
		);
	});

	test("rows say what the device does: requests in flight finish, settings restart only a running engine, files go after a day", () => {
		const rows = (copy: ActionCopy) =>
			[copy.rows.what, copy.rows.who, copy.rows.undo.text].join(" ");
		for (const kind of MODELS_WRITE_KINDS)
			expect([kind, rows(COPIES[kind]).includes("cut off")]).toEqual([
				kind,
				false,
			]);
		expect(COPIES.unload.rows.what).toBe(
			`${MODEL} lets the requests in flight finish, then stops and frees its memory.`,
		);
		expect(COPIES.configure.rows.what).toBe(
			`${MODEL} lets the requests in flight finish, then restarts with the new settings.`,
		);
		expect(COPIES.configure.rows.who).toBe(
			"New requests wait until it has restarted.",
		);
		const onDemand = configured(true, true, ON_DEMAND);
		expect(onDemand.rows.what).toBe(
			`${MODEL} lets the requests in flight finish, then stops. Its next request loads it with the new settings.`,
		);
		expect(onDemand.rows.who).toBe(
			"The next request loads it again and waits until it has started.",
		);
		expect(configured(true, true, KEPT_OFF).rows.what).toBe(
			`${MODEL} lets the requests in flight finish, then stops and stays off until you change this in its settings.`,
		);
		expect(configured(false, true, KEPT_OFF).rows.what).toBe(
			`${MODEL} keeps the new settings and stays off until you change this in its settings.`,
		);
		expect(configured(true, false).rows.what).toBe(
			`Nothing restarts: ${MODEL} keeps its settings.`,
		);
		expect(COPIES.remove.rows.what).toBe(
			`${MODEL} is removed from ${DEVICE} once the requests in flight have finished. Files no other model uses are deleted after a day, or sooner when the disk runs short.`,
		);
		expect(COPIES.remove.rows.undo.text).toBe(
			"Add it again: files deleted by then download again.",
		);
		for (const copy of [onDemand, configured(true, true, KEPT_OFF)])
			for (const sentence of sentences(copy))
				expect([sentence, leaks(String(sentence))]).toEqual([sentence, false]);
	});

	test("a change of residency alone says what changes and that nothing restarts", () => {
		const onDemand = residencyCopy(t, names, {
			mode: "on_demand",
			idle_unload_after_seconds: 900,
		});
		expect(onDemand.label).toBe(`Load ${MODEL} on demand`);
		expect(onDemand.rows.what).toBe(
			`${MODEL} unloads after 15 min without requests and frees its memory. Nothing restarts now.`,
		);
		expect(onDemand.rows.who).toBe(
			"The next request after that loads it again and waits until it has started.",
		);
		const kept = residencyCopy(t, names, { mode: "always_on" });
		expect(kept.label).toBe(`Keep ${MODEL} loaded`);
		const off = residencyCopy(t, names, { mode: "pinned_off" });
		expect(off.label).toBe(`Keep ${MODEL} off`);
		expect(off.rows.who).toBe(
			"Apps and people that use it get an error until then.",
		);
		for (const copy of [onDemand, kept, off]) {
			expect(copy.strength).toBe("none");
			for (const sentence of sentences(copy))
				expect([sentence, leaks(String(sentence))]).toEqual([sentence, false]);
		}
	});

	test("updating an installed runtime names no size: the overview knows only the installed build's", () => {
		const update = updateRuntimeCopy(t, { runtime: RUNTIME, device: DEVICE });
		expect(update.label).toBe(`Update ${RUNTIME}`);
		expect(update.rows.what).toBe(
			`${DEVICE} downloads the newest signed ${RUNTIME} runtime and checks its signature before it replaces the installed build.`,
		);
		expect(update.rows.what).not.toMatch(/MiB|GiB|KiB/);
		expect(update.rows.undo.reversible).toBe(false);
		for (const sentence of sentences(update))
			expect([sentence, leaks(String(sentence))]).toEqual([sentence, false]);
	});
});

describe("tray", () => {
	test("each write has its own tray title; reads are not writes", () => {
		const titles = MODELS_WRITE_KINDS.map((kind) => trayTitle(t, kind));
		expect(new Set(titles).size).toBe(titles.length);
		for (const title of titles) expect(leaks(title)).toBe(false);
		expect(trayTitle(t, "cancel_job")).toBe("Cancel download");
		for (const kind of MODELS_WRITE_KINDS)
			expect(isModelsWriteKind(kind)).toBe(true);
		for (const value of ["overview", "stats", "jobs", "probe", undefined, 3])
			expect([value, isModelsWriteKind(value)]).toEqual([value, false]);
	});
});
