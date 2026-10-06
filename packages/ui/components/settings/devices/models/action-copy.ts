import type {
	ModelsRequest,
	Residency,
} from "../../../../lib/device-management/models";
import { bytesText } from "../observe/observe-data";
import type { DevicesT } from "../primitives/area-context";
import type { ConfirmStrength } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";

/*
 * R8 for every write of the Models tab: verb + object for the button, the
 * confirm and the tray, the consequence rows and how hard the confirm is.
 */

export interface ActionCopy {
	/** Verb + object ("Load Qwen3-8B Q4_K_M"). */
	label: string;
	rows: ConsequenceRows;
	strength: ConfirmStrength;
	/** The acknowledgement for strength `check`. */
	checkLabel?: string;
	tone: "danger" | "default";
}

const now = (t: DevicesT) =>
	t("devices:models.actions.when.now", "Immediately.");

const background = (t: DevicesT) =>
	t(
		"devices:models.actions.when.background",
		"Starts now and downloads in the background. Progress shows under Downloads.",
	);

export function installCopy(
	t: DevicesT,
	names: { model: string; device: string; size: number; files: number },
): ActionCopy {
	return {
		label: t("devices:models.actions.install.label", "Add {{model}}", names),
		rows: {
			what: t("devices:models.actions.install.what", {
				count: names.files,
				device: names.device,
				size: bytesText(names.size),
				defaultValue_one:
					"{{device}} downloads {{size}} in {{count, number}} file from its sources and checks it against its fingerprint.",
				defaultValue_other:
					"{{device}} downloads {{size}} in {{count, number}} files from their sources and checks each against its fingerprint.",
			}),
			who: t(
				"devices:models.actions.install.who",
				"Nothing runs yet: the model loads when you load it or on its first request.",
			),
			stays: t(
				"devices:models.actions.install.stays",
				"Files another model already uses aren't downloaded again.",
			),
			when: background(t),
			undo: {
				reversible: true,
				text: t(
					"devices:models.actions.install.undo",
					"Remove the model to free its disk space.",
				),
			},
		},
		strength: "none",
		tone: "default",
	};
}

type ConfigureNames = {
	model: string;
	device: string;
	/** The engine runs now. */
	loaded: boolean;
	/** Only a change of settings restarts a running engine; residency alone never does. */
	settingsChanged: boolean;
};

/** Kept off: the device stops the engine (letting requests in flight finish) and loads it no more. */
function configureOffRows(t: DevicesT, names: ConfigureNames) {
	return {
		what: names.loaded
			? t(
					"devices:models.actions.configure.whatStops",
					"{{model}} lets the requests in flight finish, then stops and stays off until you change this in its settings.",
					names,
				)
			: t(
					"devices:models.actions.configure.whatOff",
					"{{model}} keeps the new settings and stays off until you change this in its settings.",
					names,
				),
		who: t(
			"devices:models.actions.residency.pinnedOff.who",
			"Apps and people that use it get an error until then.",
		),
	};
}

/** A running engine with new settings: drained and stopped, then loaded again with them. */
function configureRestartRows(
	t: DevicesT,
	names: ConfigureNames,
	residency: Residency,
) {
	if (residency.mode === "always_on")
		return {
			what: t(
				"devices:models.actions.configure.whatLoaded",
				"{{model}} lets the requests in flight finish, then restarts with the new settings.",
				names,
			),
			who: t(
				"devices:models.actions.configure.whoLoaded",
				"New requests wait until it has restarted.",
			),
		};
	return {
		what: t(
			"devices:models.actions.configure.whatOnDemand",
			"{{model}} lets the requests in flight finish, then stops. Its next request loads it with the new settings.",
			names,
		),
		who: t(
			"devices:models.actions.unload.whoOnDemand",
			"The next request loads it again and waits until it has started.",
		),
	};
}

function configureRows(
	t: DevicesT,
	names: ConfigureNames,
	residency: Residency,
) {
	if (residency.mode === "pinned_off") return configureOffRows(t, names);
	if (!names.settingsChanged)
		return {
			what: t(
				"devices:models.actions.configure.whatSame",
				"Nothing restarts: {{model}} keeps its settings.",
				names,
			),
			who: t("devices:models.actions.configure.whoSame", "Nobody notices."),
		};
	if (names.loaded) return configureRestartRows(t, names, residency);
	return {
		what: t(
			"devices:models.actions.configure.what",
			"{{model}} uses the new settings the next time it loads.",
			names,
		),
		who: t(
			"devices:models.actions.configure.who",
			"Nobody notices until it loads.",
		),
	};
}

/** `residency` is the one the change sets; a change of residency alone is `residencyCopy`. */
export function configureCopy(
	t: DevicesT,
	names: ConfigureNames,
	residency: Residency,
): ActionCopy {
	return {
		label: t(
			"devices:models.actions.configure.label",
			"Change the settings of {{model}}",
			names,
		),
		rows: {
			...configureRows(t, names, residency),
			when: now(t),
			undo: {
				reversible: true,
				text: t(
					"devices:models.actions.configure.undo",
					"Change the settings back.",
				),
			},
		},
		strength: "none",
		tone: "default",
	};
}

export function loadCopy(
	t: DevicesT,
	names: { model: string; device: string },
): ActionCopy {
	return {
		label: t("devices:models.actions.load.label", "Load {{model}}", names),
		rows: {
			what: t(
				"devices:models.actions.load.what",
				"{{model}} starts on {{device}} with its settings and keeps its memory until it unloads.",
				names,
			),
			who: t(
				"devices:models.actions.load.who",
				"Apps and people that use it get answers without waiting for it to start.",
			),
			when: t(
				"devices:models.actions.load.when",
				"Immediately. Loading takes a few seconds to a few minutes.",
			),
			undo: {
				reversible: true,
				text: t("devices:models.actions.load.undo", "Unload it again."),
			},
		},
		strength: "none",
		tone: "default",
	};
}

/** A change of residency alone: nothing restarts, only when the model loads and unloads changes. */
export function residencyCopy(
	t: DevicesT,
	names: { model: string; device: string },
	residency: Residency,
): ActionCopy {
	const rows = {
		always_on: () => ({
			label: t(
				"devices:models.actions.residency.alwaysOn.label",
				"Keep {{model}} loaded",
				names,
			),
			what: t(
				"devices:models.actions.residency.alwaysOn.what",
				"{{model}} loads within seconds if it isn't loaded and stays loaded, even without requests.",
				names,
			),
			who: t(
				"devices:models.actions.residency.alwaysOn.who",
				"Apps and people that use it get answers without waiting for it to start.",
			),
		}),
		on_demand: () => ({
			label: t(
				"devices:models.actions.residency.onDemand.label",
				"Load {{model}} on demand",
				names,
			),
			what: t(
				"devices:models.actions.residency.onDemand.what",
				"{{model}} unloads after {{minutes, number}} min without requests and frees its memory. Nothing restarts now.",
				{
					...names,
					minutes:
						residency.mode === "on_demand"
							? Math.round(residency.idle_unload_after_seconds / 60)
							: 0,
				},
			),
			who: t(
				"devices:models.actions.residency.onDemand.who",
				"The next request after that loads it again and waits until it has started.",
			),
		}),
		pinned_off: () => ({
			label: t(
				"devices:models.actions.residency.pinnedOff.label",
				"Keep {{model}} off",
				names,
			),
			what: t(
				"devices:models.actions.residency.pinnedOff.what",
				"{{model}} stops, frees its memory and stays off until you change this in its settings.",
				names,
			),
			who: t(
				"devices:models.actions.residency.pinnedOff.who",
				"Apps and people that use it get an error until then.",
			),
		}),
	} satisfies Record<Residency["mode"], () => Record<string, string>>;
	const { label, what, who } = rows[residency.mode]();
	return {
		label,
		rows: {
			what,
			who,
			when: now(t),
			undo: {
				reversible: true,
				text: t(
					"devices:models.actions.residency.undo",
					"Change it back in its settings.",
				),
			},
		},
		strength: "none",
		tone: "default",
	};
}

/** `loadsOnRequest`: false only for a model kept off, whose requests fail. */
export function unloadCopy(
	t: DevicesT,
	names: { model: string; device: string; loadsOnRequest: boolean },
): ActionCopy {
	return {
		label: t("devices:models.actions.unload.label", "Unload {{model}}", names),
		rows: {
			what: t(
				"devices:models.actions.unload.what",
				"{{model}} lets the requests in flight finish, then stops and frees its memory.",
				names,
			),
			who: names.loadsOnRequest
				? t(
						"devices:models.actions.unload.whoOnDemand",
						"The next request loads it again and waits until it has started.",
					)
				: t(
						"devices:models.actions.unload.who",
						"Apps and people that use it get an error until it is loaded again.",
					),
			when: now(t),
			undo: {
				reversible: true,
				text: t("devices:models.actions.unload.undo", "Load it again."),
			},
		},
		strength: "none",
		tone: "danger",
	};
}

/** The device drops the model's references at once; its store deletes files nothing uses after a grace period (plan §3.1). */
export function removeCopy(
	t: DevicesT,
	names: { model: string; device: string },
): ActionCopy {
	return {
		label: t("devices:models.actions.remove.label", "Remove {{model}}", names),
		rows: {
			what: t(
				"devices:models.actions.remove.what",
				"{{model}} is removed from {{device}} once the requests in flight have finished. Files no other model uses are deleted after a day, or sooner when the disk runs short.",
				names,
			),
			who: t(
				"devices:models.actions.remove.who",
				"After that, apps and people that use it get an error.",
			),
			stays: t(
				"devices:models.actions.remove.stays",
				"Other models, runtimes and the usage statistics stay.",
			),
			when: now(t),
			undo: {
				reversible: false,
				text: t(
					"devices:models.actions.remove.undo",
					"Add it again: files deleted by then download again.",
				),
			},
		},
		strength: "check",
		checkLabel: t(
			"devices:models.actions.remove.check",
			"Let {{device}} delete its files",
			names,
		),
		tone: "danger",
	};
}

export function installRuntimeCopy(
	t: DevicesT,
	names: { runtime: string; device: string; size: number },
): ActionCopy {
	return {
		label: t(
			"devices:models.actions.installRuntime.label",
			"Install {{runtime}}",
			names,
		),
		rows: {
			what: t(
				"devices:models.actions.installRuntime.what",
				"{{device}} downloads the signed {{runtime}} runtime ({{size}}) and checks its signature before installing it.",
				{ ...names, size: bytesText(names.size) },
			),
			who: t(
				"devices:models.actions.installRuntime.who",
				"Models on this engine use it the next time they load.",
			),
			when: background(t),
			undo: {
				reversible: true,
				text: t(
					"devices:models.actions.installRuntime.undo",
					"Remove the runtime again.",
				),
			},
		},
		strength: "none",
		tone: "default",
	};
}

/** An installed pack in a newer build: the overview knows neither its build nor its size. */
export function updateRuntimeCopy(
	t: DevicesT,
	names: { runtime: string; device: string },
): ActionCopy {
	return {
		label: t(
			"devices:models.actions.updateRuntime.label",
			"Update {{runtime}}",
			names,
		),
		rows: {
			what: t(
				"devices:models.actions.updateRuntime.what",
				"{{device}} downloads the newest signed {{runtime}} runtime and checks its signature before it replaces the installed build.",
				names,
			),
			who: t(
				"devices:models.actions.installRuntime.who",
				"Models on this engine use it the next time they load.",
			),
			when: background(t),
			undo: {
				reversible: false,
				text: t(
					"devices:models.actions.updateRuntime.undo",
					"The build it replaces is deleted.",
				),
			},
		},
		strength: "none",
		tone: "default",
	};
}

export function removeRuntimeCopy(
	t: DevicesT,
	names: { runtime: string; device: string },
): ActionCopy {
	return {
		label: t(
			"devices:models.actions.removeRuntime.label",
			"Remove {{runtime}}",
			names,
		),
		rows: {
			what: t(
				"devices:models.actions.removeRuntime.what",
				"The {{runtime}} runtime is deleted from {{device}}.",
				names,
			),
			who: t(
				"devices:models.actions.removeRuntime.who",
				"Models on this engine fall back to another installed runtime, or can't load until one is installed.",
			),
			when: now(t),
			undo: {
				reversible: true,
				text: t(
					"devices:models.actions.removeRuntime.undo",
					"Install it again; it downloads again.",
				),
			},
		},
		strength: "check",
		checkLabel: t(
			"devices:models.actions.removeRuntime.check",
			"Models on this engine may stop working",
		),
		tone: "danger",
	};
}

export function cancelJobCopy(
	t: DevicesT,
	names: { file: string; device: string },
): ActionCopy {
	return {
		label: t(
			"devices:models.actions.cancelJob.label",
			"Cancel the download of {{file}}",
			names,
		),
		rows: {
			what: t(
				"devices:models.actions.cancelJob.what",
				"{{device}} stops downloading {{file}} and deletes what it downloaded so far.",
				names,
			),
			who: t(
				"devices:models.actions.cancelJob.who",
				"Models that need this file can't load until it is downloaded.",
			),
			when: now(t),
			undo: {
				reversible: true,
				text: t(
					"devices:models.actions.cancelJob.undo",
					"Add the model again to download it again.",
				),
			},
		},
		strength: "none",
		tone: "danger",
	};
}

export function ensureCopy(
	t: DevicesT,
	names: { device: string; pins: number },
): ActionCopy {
	return {
		label: t("devices:models.actions.ensure.label", "Prepare the app's models"),
		rows: {
			what: t("devices:models.actions.ensure.what", {
				count: names.pins,
				device: names.device,
				defaultValue_one:
					"{{device}} downloads the files of {{count, number}} model the app uses, unless it has them already.",
				defaultValue_other:
					"{{device}} downloads the files of the {{count, number}} models the app uses, unless it has them already.",
			}),
			who: t(
				"devices:models.actions.ensure.who",
				"Nothing runs until the app's update is applied.",
			),
			when: background(t),
			undo: {
				reversible: null,
				text: t(
					"devices:models.actions.ensure.undo",
					"Not needed: files nothing uses are cleaned up after a day.",
				),
			},
		},
		strength: "none",
		tone: "default",
	};
}

export const MODELS_WRITE_KINDS = [
	"install",
	"configure",
	"load",
	"unload",
	"remove",
	"ensure",
	"install_runtime",
	"remove_runtime",
	"cancel_job",
] as const satisfies readonly ModelsRequest["kind"][];
export type ModelsWriteKind = (typeof MODELS_WRITE_KINDS)[number];

export const isModelsWriteKind = (value: unknown): value is ModelsWriteKind =>
	(MODELS_WRITE_KINDS as readonly unknown[]).includes(value);

/** The tray's title of a model change, by request kind. */
export function trayTitle(t: DevicesT, request: ModelsWriteKind): string {
	const titles = {
		install: t("devices:models.actions.tray.install", "Add model"),
		configure: t("devices:models.actions.tray.configure", "Model settings"),
		load: t("devices:models.actions.tray.load", "Load model"),
		unload: t("devices:models.actions.tray.unload", "Unload model"),
		remove: t("devices:models.actions.tray.remove", "Remove model"),
		ensure: t("devices:models.actions.tray.ensure", "Prepare models"),
		install_runtime: t(
			"devices:models.actions.tray.installRuntime",
			"Install model runtime",
		),
		remove_runtime: t(
			"devices:models.actions.tray.removeRuntime",
			"Remove model runtime",
		),
		cancel_job: t("devices:models.actions.tray.cancelJob", "Cancel download"),
	} satisfies Record<ModelsWriteKind, string>;
	return titles[request];
}
