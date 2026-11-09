import { isRecord } from "../../lib/response-shape";
import type {
	IBotNote,
	IBotNoteRef,
	IBotNoteVersion,
} from "../../state/backend-state/bot-state";
import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { plainBody } from "./approval-entry";

const MAX_KEY_LENGTH = 128;

export function noteRefOf(entry: IInboxEntry): IBotNoteRef | undefined {
	const value = plainBody(entry)?.note;
	if (!isRecord(value)) return undefined;
	const { key, version, title, status, closed } = value;
	if (
		typeof key !== "string" ||
		!key ||
		key.length > MAX_KEY_LENGTH ||
		typeof version !== "number" ||
		!Number.isInteger(version) ||
		version < 1 ||
		typeof title !== "string"
	)
		return undefined;
	return {
		key,
		version,
		title,
		...(typeof status === "string" && status ? { status } : {}),
		...(closed === true ? { closed: true } : {}),
	};
}

export interface NoteVersionView {
	version: IBotNoteVersion;
	/** The note moved past the version the message refers to. */
	updatedSince: boolean;
}

function currentVersion(note: IBotNote): IBotNoteVersion {
	return {
		version: note.version,
		title: note.title,
		status: note.status,
		body: note.body,
		state: note.state,
		updated_by: note.updated_by,
		updated_at: note.updated_at,
	};
}

/** The referenced version, or the current one flagged as newer when history no longer has it. */
export function noteVersionView(
	note: IBotNote,
	version: number,
): NoteVersionView {
	if (version === note.version)
		return { version: currentVersion(note), updatedSince: false };
	const past = note.history?.find((entry) => entry.version === version);
	if (past) return { version: past, updatedSince: note.version > version };
	return { version: currentVersion(note), updatedSince: true };
}

export function isNoteConflict(error: unknown): boolean {
	if (isRecord(error) && error.status === 409) return true;
	const message = error instanceof Error ? error.message : String(error);
	return /\b409\b|conflict|stale/i.test(message);
}
