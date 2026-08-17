"use client";

import { useTranslation } from "@flow-like/locales";
import type { IApp } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";

export function AboutSection({
	app,
	meta,
}: Readonly<{ app: IApp; meta: IMetadata }>) {
	const { t } = useTranslation("store");
	const hasMedia = (meta.preview_media?.length ?? 0) > 0;
	const hasRelease = !!(meta.release_notes || app.changelog);

	return (
		<div className="space-y-8">
			<p className="text-base leading-relaxed text-muted-foreground max-w-prose">
				{meta.description ?? t('noDescriptionFound', 'No description found.')}
			</p>

			{meta.tags?.length ? (
				<div className="flex flex-wrap gap-1.5">
					{meta.tags.map((t) => (
						<span
							key={t}
							className="rounded-full bg-muted/30 px-2.5 py-0.5 text-[11px] text-muted-foreground capitalize"
						>
							{t}
						</span>
					))}
				</div>
			) : null}

			{hasMedia && (
				<div className="-mx-6 md:-mx-10">
					<div
						className="flex gap-3 overflow-x-auto px-6 md:px-10 snap-x snap-mandatory pb-2"
						style={{ scrollbarWidth: "none" }}
					>
						{meta.preview_media!.map((m, i) => (
							<div key={`${m}-${i}`} className="snap-start shrink-0">
								<img
									src={m}
									alt={t('previewVal', 'Preview {{val}}', { val: i + 1 })}
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
						{t('releaseNotes2', 'Release notes')}
					</summary>
					<div className="mt-3 leading-relaxed text-muted-foreground whitespace-pre-wrap pl-4 border-l-2 border-border/20">
						{meta.release_notes || app.changelog}
					</div>
				</details>
			)}
		</div>
	);
}
