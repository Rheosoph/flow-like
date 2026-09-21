"use client";

import { useTranslation } from "@flow-like/locales";
import { Link2, ScrollText } from "lucide-react";
import { useMemo, useState } from "react";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import {
	ACTIVITY_SUFFIX,
	AuditHeadCard,
	AuditRecordList,
	AuditVerifyPanel,
	PLATFORM_CHAIN,
} from "../../../audit";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
} from "../../../ui";
import { useChainStatus } from "./use-chain-status";

interface AuditChainExplorerProps {
	profile: IProfile | undefined;
	chainId: string;
	onChainChange: (chainId: string) => void;
}

function ChainPicker({
	profile,
	value,
	onOpen,
}: Readonly<{
	profile: IProfile | undefined;
	value: string;
	onOpen: (chainId: string) => void;
}>) {
	const { t } = useTranslation("audit");
	const [draft, setDraft] = useState(value);
	const status = useChainStatus(profile);

	const suggestions = useMemo(() => {
		const pending = new Map<string, number>([[PLATFORM_CHAIN, 0]]);
		for (const chain of status.data?.recent_chains ?? []) {
			pending.set(chain.chain_id, chain.pending);
		}
		return [...pending.entries()];
	}, [status.data?.recent_chains]);

	return (
		<Card>
			<CardContent className="space-y-3 pt-6">
				<form
					className="flex flex-wrap gap-2"
					onSubmit={(event) => {
						event.preventDefault();
						const next = draft.trim();
						if (next) onOpen(next);
					}}
				>
					<div className="relative min-w-60 flex-1">
						<Link2 className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
						<Input
							value={draft}
							onChange={(event) => setDraft(event.target.value)}
							placeholder={t(
								"chainIdPlaceholder",
								"platform, an app id, or <app id>#activity",
							)}
							aria-label={t("chainId", "Chain id")}
							className="pl-8 font-mono text-sm"
						/>
					</div>
					<Button type="submit" disabled={!draft.trim()}>
						{t("openChain", "Open chain")}
					</Button>
				</form>
				<div className="flex flex-wrap items-center gap-1.5">
					<span className="text-xs text-muted-foreground">
						{t("recentChains", "Recently sealed")}
					</span>
					{suggestions.map(([chainId, pending]) => (
						<Button
							key={chainId}
							type="button"
							size="sm"
							variant={chainId === value ? "secondary" : "outline"}
							className="h-7 gap-1.5 font-mono text-xs"
							onClick={() => onOpen(chainId)}
						>
							<span className="max-w-56 truncate">{chainId}</span>
							{pending > 0 && (
								<Badge
									variant="outline"
									className="px-1 text-[10px]"
									title={t("pendingInChain", "Pending records")}
								>
									{pending}
								</Badge>
							)}
						</Button>
					))}
				</div>
			</CardContent>
		</Card>
	);
}

export function AuditChainExplorer({
	profile,
	chainId,
	onChainChange,
}: Readonly<AuditChainExplorerProps>) {
	const { t } = useTranslation("audit");
	const isActivity = chainId.endsWith(ACTIVITY_SUFFIX);

	return (
		<div className="space-y-4">
			<ChainPicker
				key={chainId}
				profile={profile}
				value={chainId}
				onOpen={onChainChange}
			/>
			<div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_380px]">
				<Card className="min-w-0">
					<CardHeader className="pb-3">
						<CardTitle className="flex items-center gap-2 text-base">
							<ScrollText className="h-4 w-4 text-primary" />
							{t("records", "Records")}
							<Badge variant="outline" className="text-[10px]">
								{isActivity
									? t("classActivity", "Activity")
									: t("classEvidence", "Evidence")}
							</Badge>
						</CardTitle>
						<CardDescription className="break-all font-mono text-xs">
							{chainId}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<AuditRecordList
							key={chainId}
							profile={profile}
							chainId={chainId}
						/>
					</CardContent>
				</Card>
				<div className="min-w-0 space-y-4">
					<AuditVerifyPanel
						key={chainId}
						profile={profile}
						chainId={chainId}
						allowFull
					/>
					<AuditHeadCard profile={profile} chainId={chainId} />
				</div>
			</div>
		</div>
	);
}
