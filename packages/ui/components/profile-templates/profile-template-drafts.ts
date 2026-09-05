import type { IProfile } from "../../lib/schema/profile/profile";

export interface ProfileTemplateDraft {
	draft: IProfile;
	baseline: IProfile;
}

const drafts = new Map<string, ProfileTemplateDraft>();
const MAX_DRAFTS = 20;

export function readProfileTemplateDraft(
	key: string,
): ProfileTemplateDraft | undefined {
	const stored = drafts.get(key);
	return stored ? structuredClone(stored) : undefined;
}

export function writeProfileTemplateDraft(
	key: string,
	value: ProfileTemplateDraft,
): void {
	drafts.delete(key);
	drafts.set(key, structuredClone(value));
	while (drafts.size > MAX_DRAFTS) {
		const oldest = drafts.keys().next().value;
		if (oldest === undefined) break;
		drafts.delete(oldest);
	}
}

export function clearProfileTemplateDraft(key: string): void {
	drafts.delete(key);
}
