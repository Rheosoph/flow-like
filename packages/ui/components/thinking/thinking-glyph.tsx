import { cn } from "../../lib/utils";
import type { ThinkingGlyphLevel } from "./thinking-options";

export interface ThinkingGlyphProps {
	readonly level: ThinkingGlyphLevel;
	readonly size?: "xs" | "sm" | "md";
	/** Provider default: the rings are dashed. */
	readonly auto?: boolean;
	/** Paused until its menu row is hovered or focused. */
	readonly idle?: boolean;
	readonly className?: string;
}

const RINGS = ["fl-think-r1", "fl-think-r2", "fl-think-r3", "fl-think-r4"];

/** The atom that stands for a thinking level; motion lives in `.fl-think` in global.css. */
export function ThinkingGlyph({
	level,
	size = "sm",
	auto = false,
	idle = false,
	className,
}: ThinkingGlyphProps) {
	return (
		<span
			aria-hidden="true"
			data-level={level}
			className={cn(
				"fl-think",
				`fl-think-l${level}`,
				size === "xs" && "fl-think-xs",
				size === "md" && "fl-think-md",
				auto && "fl-think-auto",
				idle && "fl-think-idle",
				className,
			)}
		>
			<span className="fl-think-halo" />
			{RINGS.map((ring) => (
				<span key={ring} className={cn("fl-think-ring", ring)}>
					<span className="fl-think-ellipse" />
					<span className="fl-think-ox">
						<span className="fl-think-oy">
							<span className="fl-think-dot" />
						</span>
					</span>
				</span>
			))}
			<span className="fl-think-core" />
		</span>
	);
}
