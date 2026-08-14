"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import {
	ACCESS_LADDERS,
	ROLE_TEMPLATES,
	type RoleTemplate,
	TONE_GAUGE_CLASS,
	templateLevel,
} from "./access-ladders";

export function TemplatePicker({
	onPick,
	onCancel,
}: Readonly<{
	onPick: (template: RoleTemplate) => void;
	onCancel: () => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<section
			id="role-templates"
			className="rounded-lg border bg-card p-4 flex flex-col gap-3"
		>
			<div className="flex items-start justify-between gap-3">
				<div>
					<h2 className="text-sm font-semibold">
						{`Start from a shape that already works`}
					</h2>
					<p className="text-xs text-muted-foreground">
						{t('pickTheClosestFitThenAdjustNothingIsLockedIn', 'Pick the closest fit, then adjust. Nothing is locked in.')}
					</p>
				</div>
				<Button variant="ghost" size="sm" onClick={onCancel}>
					{t('cancel', 'Cancel')}
				</Button>
			</div>
			<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-2.5">
				{ROLE_TEMPLATES.map((template) => (
					<button
						key={template.name}
						type="button"
						onClick={() => onPick(template)}
						className="rounded-lg border bg-muted/40 p-3 text-left flex flex-col gap-2 hover:border-primary hover:bg-primary/5 transition-colors"
					>
						<span className="text-[13px] font-semibold">{template.name}</span>
						<span className="text-xs text-muted-foreground leading-snug">
							{template.description}
						</span>
						<span className="flex items-end gap-1 h-4 mt-auto">
							{ACCESS_LADDERS.map((ladder) => {
								const level = templateLevel(template, ladder);
								const tone = ladder.levels[level].tone;
								return (
									<span
										key={ladder.id}
										style={{ height: `${4 + level * 3.5}px` }}
										className={cn("w-1.5 rounded-sm", TONE_GAUGE_CLASS[tone])}
									/>
								);
							})}
						</span>
					</button>
				))}
			</div>
		</section>
	);
}
