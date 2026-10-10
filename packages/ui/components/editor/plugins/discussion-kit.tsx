"use client";

import { createPlatePlugin, type PlateEditor } from "platejs/react";

import { BlockDiscussion } from "../ui/block-discussion";

import type {
	EditorUser,
	ReviewDiscussion,
	PlateDocument,
} from "../../../lib/plate-document";

export type TDiscussion = ReviewDiscussion;

// TextEditor persists these options alongside the document nodes.
export const discussionPlugin = createPlatePlugin({
	key: "discussion",
	options: {
		documentId: undefined as string | undefined,
		legacy: true,
		inline: true,
		currentUserId: "",
		enabled: false,
		canModerate: false,
		discussions: [] as TDiscussion[],
		users: {} as Record<string, EditorUser>,
	},
})
	.configure({
		render: { aboveNodes: BlockDiscussion },
	})
	.extendSelectors(({ getOption }) => ({
		currentUser: () => getOption("users")[getOption("currentUserId")],
		user: (id: string) => getOption("users")[id],
	}));

export const DiscussionKit = [discussionPlugin];

/** Native export includes review bodies, including resolved discussions. */
export function getPlateEditorDocument(editor: PlateEditor): PlateDocument {
	return {
		version: 1,
		children: editor.children,
		documentId: editor.getOption(discussionPlugin, "documentId"),
		discussions: editor.getOption(discussionPlugin, "discussions"),
		users: editor.getOption(discussionPlugin, "users"),
		legacy: editor.getOption(discussionPlugin, "legacy"),
	};
}

/** Restore review state without replacing the authenticated author's identity. */
export function applyPlateEditorDocument(
	editor: PlateEditor,
	document: PlateDocument,
) {
	const currentUserId = editor.getOption(discussionPlugin, "currentUserId");
	const currentUser = editor.getOption(discussionPlugin, "users")[
		currentUserId
	];
	editor.tf.setValue(document.children);
	editor.setOption(discussionPlugin, "documentId", document.documentId);
	editor.setOption(discussionPlugin, "legacy", document.legacy ?? false);
	editor.setOption(discussionPlugin, "users", {
		...document.users,
		...(currentUser ? { [currentUserId]: currentUser } : {}),
	});
	editor.setOption(discussionPlugin, "discussions", document.discussions);
}
