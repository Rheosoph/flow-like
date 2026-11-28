"use client";

import type { IAppShowcaseSettings } from "../../../lib/schema/app/showcase";
import { Card, CardContent } from "../../ui/card";
import { Separator } from "../../ui/separator";
import type { ShowcaseGroupSpec } from "./showcase-group-spec";
import type {
	ShowcaseBoolPath,
	ShowcaseListKey,
} from "./showcase-settings-model";
import { ShowcaseSwitchRow } from "./showcase-switch-row";

export function ShowcaseGroupCard({
	spec,
	settings,
	readFlag,
	readList,
	onFlag,
	onItem,
}: Readonly<{
	spec: ShowcaseGroupSpec;
	settings: IAppShowcaseSettings;
	readFlag: (s: IAppShowcaseSettings, path: ShowcaseBoolPath) => boolean;
	readList: (s: IAppShowcaseSettings, key: ShowcaseListKey) => string[];
	onFlag: (path: ShowcaseBoolPath, value: boolean) => void;
	onItem: (key: ShowcaseListKey, item: string, ticked: boolean) => void;
}>) {
	const list = spec.list;
	const on = readFlag(settings, spec.master.path);
	const ticked = spec.list ? new Set(readList(settings, spec.list.key)) : null;
	const hasChildren = !!spec.list || spec.subs.length > 0;

	return (
		<Card>
			<CardContent className="py-2">
				<ShowcaseSwitchRow
					master
					label={spec.master.label}
					hint={spec.master.hint}
					checked={on}
					onChange={(value) => onFlag(spec.master.path, value)}
				/>
				{hasChildren && (
					<div className="ml-1 border-l pl-4">
						{list && ticked && (
							<div className="pb-1">
								<Separator className="mb-2" />
								<p className="text-xs font-medium text-muted-foreground">
									{list.heading}
								</p>
								{list.items.length === 0 && (
									<p className="py-2 text-xs text-muted-foreground">
										{list.empty}
									</p>
								)}
								{list.items.map((item) => (
									<ShowcaseSwitchRow
										key={item.id}
										label={item.label}
										hint={item.hint}
										checked={ticked.has(item.id)}
										disabled={!on}
										onChange={(value) => onItem(list.key, item.id, value)}
									/>
								))}
							</div>
						)}
						{spec.subs.map((sub) => (
							<ShowcaseSwitchRow
								key={sub.path}
								label={sub.label}
								hint={sub.hint}
								checked={readFlag(settings, sub.path)}
								disabled={!on}
								onChange={(value) => onFlag(sub.path, value)}
							/>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}
