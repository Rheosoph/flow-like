import { agentSupports } from "../../../../lib/device-management/agent-reads";
import type { PlanTarget } from "../../../../lib/device-management/model/deploy-plan";
import type { AgentFeatures } from "../../../../lib/device-management/model/types";
import {
	type ExportCommands,
	desktopExportCommands,
} from "../../../../lib/device-management/project-export";
import type { IApp } from "../../../../lib/schema/app/app";

/**
 * Bit metadata v2 goes to devices only when every target's agent acquires
 * model files itself (`model_store`); one older agent keeps the whole deploy
 * on v1, which it reads unchanged.
 */
export function targetsHaveModelStore(
	targets: readonly Pick<PlanTarget, "deviceId">[],
	features: (deviceId: string) => AgentFeatures | undefined,
): boolean {
	return (
		targets.length > 0 &&
		targets.every((target) =>
			agentSupports(features(target.deviceId), "model_store"),
		)
	);
}

/** The desktop export; `modelStore` asks it for Bit metadata v2. */
export async function desktopExport(
	onlineApp?: IApp,
	userSub?: string,
	modelStore = false,
): Promise<ExportCommands> {
	const commands = await desktopExportCommands(onlineApp, userSub);
	if (!modelStore) return commands;
	const { invoke } = await import("@tauri-apps/api/core");
	return {
		...commands,
		prepare: (appId) =>
			invoke("prepare_device_project_export", {
				appId,
				onlineApp,
				userSub,
				modelStore,
			}),
	};
}
