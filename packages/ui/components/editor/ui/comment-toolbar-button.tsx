"use client";

import { MessageSquareTextIcon } from "lucide-react";
import { useEditorRef, usePluginOption } from "platejs/react";

import { discussionPlugin } from "../plugins/discussion-kit";

import { commentPlugin } from "../plugins/comment-kit";

import { ToolbarButton } from "./toolbar";

export function CommentToolbarButton() {
	const editor = useEditorRef();
	const enabled = usePluginOption(discussionPlugin, "enabled");
	const currentUserId = usePluginOption(discussionPlugin, "currentUserId");

	return (
		<ToolbarButton
			disabled={!enabled || !currentUserId}
			onClick={() => {
				editor.getTransforms(commentPlugin).comment.setDraft();
			}}
			data-plate-prevent-overlay
			tooltip="Comment"
		>
			<MessageSquareTextIcon />
		</ToolbarButton>
	);
}
