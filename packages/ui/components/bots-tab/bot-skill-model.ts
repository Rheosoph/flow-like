import type {
	BotSkillStatus,
	IBotSkill,
} from "../../state/backend-state/bot-state";

export const SKILL_NAME_MAX = 64;
export const SKILL_DESCRIPTION_MAX = 1024;
export const SKILL_BODY_MAX_BYTES = 32 * 1024;
export const SKILL_ZIP_MAX_BYTES = 8 * 1024 * 1024;

const SKILL_NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const STATUS_ORDER: Record<BotSkillStatus, number> = {
	pending: 0,
	active: 1,
	disabled: 2,
};

export type SkillSourceKind = "owner" | "import" | "bot" | "package";

export function sourceKind(source: string): SkillSourceKind {
	if (source.startsWith("package:")) return "package";
	if (source === "bot_proposed") return "bot";
	if (source === "import") return "import";
	return "owner";
}

/** Skills that wait for the owner first, then active ones, then switched-off ones. */
export function sortSkills(skills: readonly IBotSkill[]): IBotSkill[] {
	return [...skills].sort(
		(a, b) =>
			STATUS_ORDER[a.status] - STATUS_ORDER[b.status] ||
			a.name.localeCompare(b.name),
	);
}

export type SkillDraftProblem = "name" | "description" | "body" | "length";

export interface SkillDraft {
	name: string;
	description: string;
	body: string;
}

/** The first rule a draft breaks, or null; the server checks everything again. */
export function draftProblem(
	draft: SkillDraft,
	editing: boolean,
): SkillDraftProblem | null {
	const name = draft.name.trim();
	if (!editing && (name.length > SKILL_NAME_MAX || !SKILL_NAME.test(name))) {
		return "name";
	}
	const description = draft.description.trim();
	if (!description || description.includes("\n")) return "description";
	if ([...description].length > SKILL_DESCRIPTION_MAX) return "length";
	if (
		new TextEncoder().encode(draft.body.trim()).length > SKILL_BODY_MAX_BYTES
	) {
		return "length";
	}
	return null;
}

export function canApprove(skill: IBotSkill): boolean {
	return skill.status !== "active";
}

export function canEdit(skill: IBotSkill): boolean {
	return sourceKind(skill.source) !== "package";
}
