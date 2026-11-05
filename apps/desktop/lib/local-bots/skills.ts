import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import type {
	IBotSkill,
	ICreateBotSkillRequest,
	IUpdateBotSkillRequest,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { invoke } from "@tauri-apps/api/core";

async function run<T>(
	operation: string,
	command: string,
	args: Record<string, unknown>,
): Promise<T> {
	try {
		return await invoke<T>(command, args);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		throw new Error(`Could not ${operation}: ${message}`);
	}
}

function expectSkill(value: unknown, operation: string): IBotSkill {
	if (!isRecord(value)) {
		throw new Error(`Could not ${operation}: this computer returned nothing`);
	}
	return value as unknown as IBotSkill;
}

/** The skills of the bots that live on this computer. */
export const localSkills = {
	async list(botId: string): Promise<IBotSkill[]> {
		return asArray(
			await run<IBotSkill[]>(
				`list the skills of local bot ${botId}`,
				"local_bot_skills",
				{ botId },
			),
		);
	},

	async create(
		botId: string,
		request: ICreateBotSkillRequest,
	): Promise<IBotSkill> {
		const operation = `add a skill to local bot ${botId}`;
		return expectSkill(
			await run(operation, "local_bot_create_skill", { botId, request }),
			operation,
		);
	},

	async update(
		botId: string,
		skillId: string,
		request: IUpdateBotSkillRequest,
	): Promise<IBotSkill> {
		const operation = `change skill ${skillId} of local bot ${botId}`;
		return expectSkill(
			await run(operation, "local_bot_update_skill", {
				botId,
				skillId,
				request,
			}),
			operation,
		);
	},

	async setStatus(
		botId: string,
		skillId: string,
		status: "active" | "disabled",
	): Promise<IBotSkill> {
		const operation = `switch skill ${skillId} of local bot ${botId} ${status}`;
		return expectSkill(
			await run(operation, "local_bot_set_skill_status", {
				botId,
				skillId,
				status,
			}),
			operation,
		);
	},

	async delete(botId: string, skillId: string): Promise<void> {
		await run(
			`delete skill ${skillId} of local bot ${botId}`,
			"local_bot_delete_skill",
			{ botId, skillId },
		);
	},

	async installPackage(
		botId: string,
		packageId: string,
		version?: string,
	): Promise<IBotSkill[]> {
		return asArray(
			await run<IBotSkill[]>(
				`install the skills of ${packageId} into local bot ${botId}`,
				"local_bot_install_skill_package",
				{ botId, packageId, version: version ?? null },
			),
		);
	},

	async import(botId: string, zip: Uint8Array): Promise<IBotSkill> {
		const operation = `import a skill into local bot ${botId}`;
		return expectSkill(
			await run(operation, "local_bot_import_skill", {
				botId,
				zip: Array.from(zip),
			}),
			operation,
		);
	},
};
