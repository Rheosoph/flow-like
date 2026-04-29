"use client";
import dynamic from "next/dynamic";
import type { Lesson } from "../../lib/learn/types";
import { TextEditor } from "../ui/text-editor";

const ReactPlayer = dynamic(() => import("react-player"), { ssr: false });

interface LessonContentProps {
	readonly lesson: Lesson;
	/** Optional click handler for inline `focus_node` marks in the lesson body. */
	readonly onFocusNode?: (nodeId: string) => void;
}

/**
 * Renders a lesson body. Content is stored as a string that may be either
 * raw markdown (legacy / seed data) or the platform's `plate_json::…` envelope
 * produced by the Platejs editor — TextEditor handles both via isMarkdown.
 */
export function LessonContent({ lesson, onFocusNode }: LessonContentProps) {
	return (
		<article className="flex flex-col gap-6">
			<header className="space-y-1">
				<h1 className="text-2xl font-semibold">{lesson.title}</h1>
				<p className="text-xs text-muted-foreground">
					{lesson.estimated_minutes} min · {lesson.language.toUpperCase()}
					{lesson.is_optional ? " · Optional" : ""}
				</p>
			</header>
			{lesson.video_url && (
				<div className="overflow-hidden rounded-lg bg-black aspect-video">
					<ReactPlayer
						src={lesson.video_url}
						width="100%"
						height="100%"
						controls
					/>
				</div>
			)}
			<div className="prose prose-sm dark:prose-invert max-w-none">
				<TextEditor
					initialContent={lesson.content ?? ""}
					editable={false}
					isMarkdown
					minimal
					onFocusNode={onFocusNode}
				/>
			</div>
		</article>
	);
}
