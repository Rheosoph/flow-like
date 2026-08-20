"use client";

import { Button } from "@flow-like/flow-like-ui";
import { Separator } from "@flow-like/flow-like-ui/components/ui/separator";
import { useTranslation } from "@flow-like/locales";

const poweredBy: {
	name: string;
	description: string;
	author: string;
	href: string;
	license?: { name: string; href?: string };
}[] = [
	{
		name: "Tauri",
		description:
			"Build smaller, faster, and more secure desktop applications with a web frontend. ",
		author: "Tauri",
		href: "https://v2.tauri.app/",
		license: {
			name: "MIT",
			href: "https://github.com/tauri-apps/tauri/blob/dev/LICENSE_MIT",
		},
	},
	{
		name: "llamafile",
		author: "Mozilla, Justine Tunney",
		description: "Distribute and run LLMs with a single file. ",
		license: {
			name: "Apache-2.0",
			href: "https://github.com/Mozilla-Ocho/llamafile?tab=License-1-ov-file#readme",
		},
		href: "https://github.com/Mozilla-Ocho/llamafile",
	},
];

export default function PoweredByPage() {
	const { t } = useTranslation("common");
	return (
		<main className="justify-start flex flex-col items-start w-full pr-4 flex-1 min-h-0">
			<h2 className="scroll-m-20 text-3xl font-semibold tracking-tight first:mt-0">
				{t(
					"standingOnTheShouldersOfGiants",
					"Standing on the Shoulders of Giants",
				)}
			</h2>
			<p className="leading-7 [&:not(:first-child)]:mt-4">
				{t(
					"weAreProudToAcknowledgeTheOpensourceProjectsThatHaveMadeFlowlikePossibleTheirContributionsHaveHelpedUsBuildABetterProductAndWeAreGratefulForTheirDedicationToInnovation",
					"We are proud to acknowledge the open-source projects that have made Flow-Like possible. Their contributions have helped us build a better product, and we are grateful for their dedication to innovation.",
				)}
			</p>
			<Separator className="my-4" />
			<div className="grid grid-cols-2 gap-2 w-full">
				{poweredBy.map((element, i) => (
					<PoweredByElement key={`${i}__powered_by`} {...element} />
				))}
			</div>
		</main>
	);
}

function PoweredByElement({
	name,
	description,
	author,
	href,
	license,
}: Readonly<{
	name: string;
	description: string;
	author: string;
	href: string;
	license?: { name: string; href?: string };
}>) {
	const { t } = useTranslation("common");
	return (
		<div className="border p-4 bg-card text-card-foreground w-full col-span-1">
			<h3>{name}</h3>
			<p className="line-clamp-2 h-[3.75rem]">{description}</p>
			<small>
				{t("by", "By")} <b>{author}</b>
			</small>
			<Separator className="my-2" />
			<div className="flex flex-row items-center gap-4">
				{href && (
					<a href={href} target="_blank" rel="noreferrer">
						<Button>{t("learnMore", "Learn More")}</Button>
					</a>
				)}
				{license &&
					(license.href ? (
						<a href={license.href} target="_blank" rel="noreferrer">
							<span className="underline">{license.name}</span>
						</a>
					) : (
						<span>{license.name}</span>
					))}
			</div>
		</div>
	);
}
