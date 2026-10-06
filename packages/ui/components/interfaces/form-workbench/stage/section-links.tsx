"use client";

import { useTranslation } from "@flow-like/locales";
import { cx } from "../../../settings/devices/primitives/tone";
import { sectionLabel } from "./copy";
import type { PaneEnv } from "./pane-env";
import type { SectionKey } from "./pane-model";

/**
 * "Steps · Answer · Result · Files 12 · Inputs": links that scroll the body to a section; the current one
 * follows the 40 % line. Only a run with two or more sections gets them, else a plain hairline (canvas `nav`).
 */
export function SectionLinks({
	runN,
	sections,
	active,
	fileCount,
	env,
	onGo,
}: Readonly<{
	runN: number;
	sections: readonly SectionKey[];
	active: SectionKey | null;
	fileCount: number;
	env: PaneEnv;
	onGo: (key: SectionKey) => void;
}>) {
	const { t } = useTranslation("interfaces");
	if (sections.length < 2)
		return <div className="h-3.5 shrink-0 border-hairline border-b" />;
	return (
		<nav
			aria-label={t("workbench.stage.links.label", "Sections of run {{n}}", {
				n: runN,
			})}
			className={cx(
				"no-scrollbar mt-1.5 flex shrink-0 gap-5 overflow-x-auto border-hairline border-b",
				env.pad,
			)}
		>
			{sections.map((key) => {
				const current = key === active;
				return (
					<button
						key={key}
						type="button"
						aria-current={current ? "true" : undefined}
						onClick={() => onGo(key)}
						className={cx(
							"-mb-px inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap border-b-2 font-medium text-[13px] outline-ring hover:text-foreground focus-visible:outline-2 focus-visible:-outline-offset-2",
							env.touch ? "h-11" : "h-9",
							current
								? "border-foreground text-foreground"
								: "border-transparent text-muted-foreground",
						)}
					>
						<span>{sectionLabel(t, key)}</span>
						{key === "files" && fileCount > 0 ? (
							<span className="font-mono text-muted-foreground text-xs tabular-nums">
								{fileCount}
							</span>
						) : null}
					</button>
				);
			})}
		</nav>
	);
}
