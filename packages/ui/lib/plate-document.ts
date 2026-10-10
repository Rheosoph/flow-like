import type { Value } from "platejs";

export const PLATE_JSON_PREFIX = "plate_json::";

export interface EditorUser {
	id: string;
	name: string;
	avatarUrl?: string;
}

export interface ReviewComment {
	id: string;
	contentRich: Value;
	createdAt: string | Date;
	updatedAt?: string | Date;
	discussionId: string;
	isEdited: boolean;
	userId: string;
}

export interface ReviewDiscussion {
	id: string;
	comments: ReviewComment[];
	createdAt: string | Date;
	isResolved: boolean;
	userId: string;
	documentContent?: string;
}

/** Content and editorial review travel together through every document save. */
export interface PlateDocument {
	version: 1;
	children: Value;
	documentId?: string;
	discussions: ReviewDiscussion[];
	users: Record<string, EditorUser>;
	/** Read compatibility for documents saved before review metadata existed. */
	legacy?: boolean;
}

const isObject = (value: unknown): value is Record<string, unknown> =>
	Boolean(value) && typeof value === "object" && !Array.isArray(value);

const isDateString = (value: unknown): value is string =>
	typeof value === "string" && Number.isFinite(Date.parse(value));

function validCommentBody(nodes: unknown, depth = 0): nodes is Value {
	return (
		depth <= 64 &&
		Array.isArray(nodes) &&
		nodes.every(
			(node) =>
				isObject(node) &&
				(typeof node.text === "string" ||
					validCommentBody(node.children, depth + 1)),
		)
	);
}

function validDiscussions(value: unknown): value is ReviewDiscussion[] {
	if (!Array.isArray(value)) return false;
	const threadIds = new Set<string>();
	const commentIds = new Set<string>();
	return value.every((thread) => {
		if (
			!isObject(thread) ||
			typeof thread.id !== "string" ||
			!thread.id ||
			threadIds.has(thread.id) ||
			typeof thread.userId !== "string" ||
			!thread.userId ||
			!isDateString(thread.createdAt) ||
			typeof thread.isResolved !== "boolean" ||
			(thread.documentContent !== undefined &&
				typeof thread.documentContent !== "string") ||
			!Array.isArray(thread.comments)
		)
			return false;
		threadIds.add(thread.id);
		return thread.comments.every((comment) => {
			if (
				!isObject(comment) ||
				typeof comment.id !== "string" ||
				!comment.id ||
				commentIds.has(comment.id) ||
				typeof comment.userId !== "string" ||
				!comment.userId ||
				comment.discussionId !== thread.id ||
				!isDateString(comment.createdAt) ||
				(comment.updatedAt !== undefined && !isDateString(comment.updatedAt)) ||
				typeof comment.isEdited !== "boolean" ||
				!validCommentBody(comment.contentRich)
			)
				return false;
			commentIds.add(comment.id);
			return true;
		});
	});
}

function validUsers(value: unknown): value is Record<string, EditorUser> {
	return (
		isObject(value) &&
		Object.entries(value).every(
			([id, user]) =>
				isObject(user) &&
				user.id === id &&
				typeof user.name === "string" &&
				(user.avatarUrl === undefined || typeof user.avatarUrl === "string"),
		)
	);
}

export function parsePlateDocument(content: string): PlateDocument | undefined {
	if (!content.startsWith(PLATE_JSON_PREFIX)) return undefined;
	try {
		const value: unknown = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
		if (Array.isArray(value)) {
			return {
				version: 1,
				children: value as Value,
				discussions: [],
				users: {},
				legacy: true,
			};
		}
		if (!value || typeof value !== "object") return undefined;
		const doc = value as Partial<PlateDocument>;
		if (doc.version !== 1 || !Array.isArray(doc.children)) return undefined;
		if (doc.discussions !== undefined && !validDiscussions(doc.discussions))
			return undefined;
		if (doc.users !== undefined && !validUsers(doc.users)) return undefined;
		return {
			version: 1,
			children: doc.children,
			...(typeof doc.documentId === "string"
				? { documentId: doc.documentId }
				: {}),
			discussions: doc.discussions ?? [],
			users: doc.users ?? {},
		};
	} catch {
		return undefined;
	}
}

export function serializePlateDocument(document: PlateDocument): string {
	const { legacy, ...envelope } = document;
	const value =
		legacy &&
		!document.documentId &&
		document.discussions.length === 0 &&
		Object.keys(document.users).length === 0
			? document.children
			: envelope;
	return `${PLATE_JSON_PREFIX}${JSON.stringify(value)}`;
}

/** Transforms visible content without discarding its review history. */
export function replacePlateDocumentChildren(
	content: string,
	children: Value,
): string {
	const document = parsePlateDocument(content);
	return serializePlateDocument({
		version: 1,
		discussions: [],
		users: {},
		legacy: true,
		...document,
		children,
	});
}
