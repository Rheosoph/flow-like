"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { DownloadIcon, Loader2Icon } from "lucide-react";
import { useState } from "react";
import { useBackend } from "../../state/backend-state";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import {
	type SkillPackageCandidate,
	fromInstalled,
	toCandidate,
	uncoveredCapabilities,
} from "./bot-skill-package-model";

const SEARCH_LIMIT = 20;

function useSkillPackages(query: string) {
	const registry = useBackend().registryState;
	return useQuery({
		queryKey: ["bot-skill-packages", query],
		enabled: Boolean(registry),
		queryFn: async (): Promise<SkillPackageCandidate[]> => {
			if (!registry) return [];
			const found = new Map<string, SkillPackageCandidate>();
			const needle = query.trim().toLowerCase();
			for (const pkg of await registry.getInstalledPackages().catch(() => [])) {
				const candidate = fromInstalled(pkg);
				const text = `${pkg.id} ${pkg.manifest.name}`.toLowerCase();
				if (candidate && text.includes(needle))
					found.set(candidate.id, candidate);
			}
			const listed = await registry
				.searchPackages({ query: needle || undefined, limit: SEARCH_LIMIT })
				.catch(() => null);
			await Promise.all(
				(listed?.packages ?? []).map(async (summary) => {
					if (found.has(summary.id)) return;
					const pkg = await registry.getPackage(summary.id).catch(() => null);
					const candidate = toCandidate(pkg?.manifest, summary, false);
					if (candidate) found.set(candidate.id, candidate);
				}),
			);
			return [...found.values()].sort((a, b) => a.name.localeCompare(b.name));
		},
	});
}

/** Packages that ship skills, from this device and the registry, with what the bot lacks to use them. */
export function BotSkillPackagePicker({
	capabilities,
	busy,
	onInstall,
}: Readonly<{
	capabilities: readonly string[];
	busy: boolean;
	onInstall: (packageId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const [query, setQuery] = useState("");
	const packages = useSkillPackages(query);
	return (
		<div data-slot="bot-skill-packages" className="flex flex-col gap-1.5">
			<Input
				aria-label={t(
					"panel.skills.packageSearch",
					"Search packages with skills",
				)}
				placeholder={t(
					"panel.skills.packageSearch",
					"Search packages with skills",
				)}
				value={query}
				className="h-8 text-xs"
				onChange={(event) => setQuery(event.target.value)}
			/>
			{packages.isLoading && (
				<Loader2Icon className="size-4 animate-spin" aria-hidden />
			)}
			{!packages.isLoading && (packages.data ?? []).length === 0 && (
				<p className="text-xs text-muted-foreground">
					{t("panel.skills.packageNone", "No package with skills found.")}
				</p>
			)}
			<ul className="flex flex-col gap-1.5">
				{(packages.data ?? []).map((pkg) => {
					const missing = uncoveredCapabilities(pkg.tags, capabilities);
					return (
						<li
							key={pkg.id}
							data-slot="bot-skill-package"
							data-blocked={missing.length > 0}
							className="flex min-w-0 flex-col gap-1 rounded-lg border p-2.5"
						>
							<div className="flex flex-wrap items-center gap-1.5">
								<span className="text-xs font-medium">{pkg.name}</span>
								<span className="font-mono text-xs text-muted-foreground">
									{pkg.id}@{pkg.version}
								</span>
								{pkg.installed && (
									<Badge variant="secondary">
										{t("panel.skills.packageInstalledBadge", "Installed")}
									</Badge>
								)}
							</div>
							<p className="text-xs text-muted-foreground">
								{pkg.skills.map((skill) => skill.name).join(", ")}
							</p>
							{missing.length > 0 && (
								<p role="alert" className="text-xs text-destructive">
									{t(
										"panel.skills.packageNeeds",
										"Needs what this bot lacks: {{list}}. Switch it on first.",
										{ list: missing.join(", ") },
									)}
								</p>
							)}
							<div className="flex justify-end">
								<Button
									type="button"
									variant="outline"
									size="sm"
									disabled={busy || missing.length > 0}
									onClick={() => onInstall(pkg.id)}
								>
									<DownloadIcon aria-hidden />
									{t("panel.skills.installPackage", "Install package skills")}
								</Button>
							</div>
						</li>
					);
				})}
			</ul>
		</div>
	);
}
