import { WorkflowIcon } from "lucide-react";
import { type MouseEvent, memo } from "react";
import type { INode } from "../../../lib/schema/flow/node";
import { cn } from "../../../lib/utils";
import { DynamicImage } from "../../ui/dynamic-image";

export const NodeGlyph = memo(function NodeGlyph({
	node,
	className,
}: Readonly<{ node?: INode; className?: string }>) {
	if (node?.icon) {
		return (
			<DynamicImage
				url={node.icon}
				className={cn("size-3 shrink-0 bg-foreground/70", className)}
			/>
		);
	}
	return (
		<WorkflowIcon
			aria-hidden
			className={cn("size-3 shrink-0 text-muted-foreground", className)}
		/>
	);
});

export const LogNodeChip = memo(function LogNodeChip({
	node,
	label,
	title,
	onClick,
	className,
}: Readonly<{
	node?: INode;
	label: string;
	title: string;
	onClick?: (event: MouseEvent<HTMLButtonElement>) => void;
	className?: string;
}>) {
	return (
		<button
			type="button"
			title={title}
			aria-label={title}
			onClick={onClick}
			className={cn(
				"inline-flex h-4.5 min-w-0 max-w-full shrink-0 items-center gap-1.5 rounded border bg-card px-1.5 font-sans text-[11px] font-medium text-foreground/80 transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
				className,
			)}
		>
			<NodeGlyph node={node} />
			<span className="min-w-0 truncate">{label}</span>
		</button>
	);
});
