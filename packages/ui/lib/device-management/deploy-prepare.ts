import type { IBackendState } from "../../state/backend-state";
import type { IProfile } from "../../types";
import type { PreparedProjectArtifact } from "./artifacts";
import { prepareOnlineDependencies } from "./online-dependencies";
import {
	type ApprovedOnlineMetadata,
	prepareOnlineMetadata,
} from "./online-metadata";
import { desktopExportCommands, prepareDesktopProject } from "./project-export";

/* The bundle a deployment sends (APP §3.2 "Preparing"): one sequence for the wizard step and for a run that starts without one. */

export type DeployPreparePhase =
	/** Online: the approved definitions are being read from the hub. */
	| "read_hub"
	/** Online: the definitions are read; models, packages and files are collected. */
	| "collect"
	/** Local-only: this computer exports its copy of the app. */
	| "read_app";

export interface DeployBundle {
	artifact: PreparedProjectArtifact;
	/** Online apps: the approved definitions the bundle was built from. */
	approved?: ApprovedOnlineMetadata;
	/** A desktop export keeps a private snapshot until this is called. */
	release?(): Promise<void>;
}

/** The lib calls of the sequence, replaceable in tests. */
export interface DeployPrepareSteps {
	metadata: typeof prepareOnlineMetadata;
	dependencies: typeof prepareOnlineDependencies;
	desktop: typeof prepareDesktopProject;
	exportCommands: typeof desktopExportCommands;
}

const STEPS: DeployPrepareSteps = {
	metadata: prepareOnlineMetadata,
	dependencies: prepareOnlineDependencies,
	desktop: prepareDesktopProject,
	exportCommands: desktopExportCommands,
};

export interface DeployPrepareInput {
	appId: string;
	/** A1: local-only apps send this computer's copy, every other app the approved definitions. */
	mode: "online" | "offline";
	desktop: boolean;
	backend: IBackendState;
	profile: IProfile;
	/** Local-only: the signed-in account the native export runs for. */
	account?: string;
	signal?: AbortSignal;
	onPhase?(phase: DeployPreparePhase): void;
	/** Online, before anything is collected: throw to stop (e.g. a chosen event is not approved). */
	onApproved?(approved: ApprovedOnlineMetadata): void;
	steps?: Partial<DeployPrepareSteps>;
}

/** Thrown for a local-only app in the browser: only the desktop app can export the copy. */
export class DeployPrepareDesktopOnlyError extends Error {
	constructor() {
		super(
			"A local-only app can only be prepared in the desktop app, which holds its copy.",
		);
		this.name = "DeployPrepareDesktopOnlyError";
	}
}

/** Bits and packages are exported natively on desktop; the browser downloads them within its size limits. */
function needsNativeExport(approved: ApprovedOnlineMetadata): boolean {
	const { app } = approved;
	return app.bits.length > 0 || Object.keys(app.packages ?? {}).length > 0;
}

async function prepareOnlineBundle(
	input: DeployPrepareInput,
	steps: DeployPrepareSteps,
): Promise<DeployBundle> {
	const { appId, backend, profile, signal } = input;
	input.onPhase?.("read_hub");
	const approved = await steps.metadata(appId, backend, profile, signal);
	input.onApproved?.(approved);
	input.onPhase?.("collect");
	if (!input.desktop || !needsNativeExport(approved)) {
		const { artifact } = await steps.dependencies(
			approved.app,
			backend,
			profile,
			signal,
			approved,
		);
		return { artifact, approved };
	}
	const exported = await steps.desktop(
		appId,
		await steps.exportCommands(approved.app),
		signal,
		approved,
	);
	return { artifact: exported.artifact, approved, release: exported.release };
}

async function prepareOfflineBundle(
	input: DeployPrepareInput,
	steps: DeployPrepareSteps,
): Promise<DeployBundle> {
	if (!input.desktop) throw new DeployPrepareDesktopOnlyError();
	input.onPhase?.("read_app");
	const exported = await steps.desktop(
		input.appId,
		await steps.exportCommands(undefined, input.account),
		input.signal,
	);
	return { artifact: exported.artifact, release: exported.release };
}

export function prepareDeployBundle(
	input: DeployPrepareInput,
): Promise<DeployBundle> {
	const steps = { ...STEPS, ...input.steps };
	return input.mode === "online"
		? prepareOnlineBundle(input, steps)
		: prepareOfflineBundle(input, steps);
}
