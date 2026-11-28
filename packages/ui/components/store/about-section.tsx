"use client";

import { useTranslation } from "@flow-like/locales";
import { asArray } from "../../lib/response-shape";
import type { IApp } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";

export function AboutSection({
	app,
	meta,
}: Readonly<{ app: IApp; meta: IMetadata }>) {
	const { t } = useTranslation("store");
	const tags = asArray(meta.tags);
	const media = [...new Set(asArray(meta.preview_media))];
	const hasRelease = !!(meta.release_notes || app.changelog);

	return (
		<div className="space-y-8">
			<div className="space-y-4">
				<p className="text-base leading-relaxed text-foreground/80 max-w-prose">
					{meta.description ?? t("noDescriptionFound", "No description found.")}
				</p>

				{tags.length ? (
					<ul
						className="flex flex-wrap gap-1.5"
						aria-label={t("storeTags", "Tags")}
					>
						{tags.map((tag) => (
							<li
								key={tag}
								className="rounded-full border border-border/60 px-2.5 py-0.5 text-xs text-muted-foreground capitalize"
							>
								{tag}
							</li>
						))}
					</ul>
				) : null}
			</div>

			{media.length > 0 && (
				<div className="-mx-6 md:-mx-10">
					<div
						className="flex gap-3 overflow-x-auto px-6 md:px-10 snap-x snap-mandatory pb-2"
						style={{ scrollbarWidth: "none" }}
					>
						{media.map((m, i) => (
							<div key={m} className="snap-start shrink-0">
								<img
									src={m}
									alt={t("previewVal", "Preview {{val}}", { val: i + 1 })}
									className="h-48 md:h-64 rounded-xl object-cover"
									loading="lazy"
									decoding="async"
								/>
							</div>
						))}
					</div>
				</div>
			)}

			{hasRelease && (
				<details className="group text-sm">
					<summary className="text-muted-foreground/60 cursor-pointer hover:text-muted-foreground transition-colors select-none">
						{t("releaseNotes2", "Release notes")}
					</summary>
					<div className="mt-3 leading-relaxed text-muted-foreground whitespace-pre-wrap pl-4 border-l-2 border-border/20">
						{meta.release_notes || app.changelog}
					</div>
				</details>
			)}
		</div>
	);
}
