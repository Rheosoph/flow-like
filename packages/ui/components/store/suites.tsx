"use client";

import { ArrowLeft, Globe, Layers } from "lucide-react";
import { useRouter } from "next/navigation";
import type { IGroup, IGroupMember } from "../..";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	useBackend,
	useInvoke,
} from "../..";

function initials(value?: string | null): string {
	const cleaned = (value ?? "").replace(/[^A-Za-z0-9 ]/g, "").trim();
	if (!cleaned) return "?";
	const parts = cleaned.split(/\s+/);
	return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase() || "?";
}

function seedGradient(seed: string): string {
	let hash = 0;
	for (let i = 0; i < seed.length; i++)
		hash = (hash * 31 + seed.charCodeAt(i)) | 0;
	const hue = ((hash % 360) + 360) % 360;
	return `linear-gradient(135deg, hsl(${hue} 62% 52%), hsl(${(hue + 42) % 360} 58% 44%))`;
}

const DOT_TEXTURE = {
	backgroundImage:
		"radial-gradient(circle at 1px 1px, rgba(255,255,255,.32) 1px, transparent 0)",
	backgroundSize: "14px 14px",
} as const;

export function SuiteCard({
	group,
	onOpen,
}: Readonly<{ group: IGroup; onOpen: (group: IGroup) => void }>) {
	const label = group.use_case || group.name || "Suite";
	return (
		<button
			type="button"
			onClick={() => onOpen(group)}
			className="group relative w-[320px] shrink-0 text-left rounded-2xl border bg-card overflow-hidden shadow-sm transition-all hover:shadow-lg hover:-translate-y-0.5 hover:border-primary/40"
		>
			<div
				className="h-24 relative"
				style={{
					backgroundImage: group.banner ? undefined : seedGradient(group.id),
				}}
			>
				{group.banner && (
					// eslint-disable-next-line @next/next/no-img-element
					<img
						src={group.banner}
						alt=""
						className="absolute inset-0 h-full w-full object-cover"
					/>
				)}
				<div className="absolute inset-0 opacity-40" style={DOT_TEXTURE} />
			</div>
			<div className="px-4 pb-4 -mt-8">
				<Avatar className="h-14 w-14 rounded-2xl ring-4 ring-card">
					{group.icon ? <AvatarImage src={group.icon} alt="" /> : null}
					<AvatarFallback
						className="rounded-2xl text-white text-lg font-bold"
						style={{ backgroundImage: seedGradient(group.id) }}
					>
						{initials(group.name)}
					</AvatarFallback>
				</Avatar>
				<p className="mt-3 text-[11px] font-mono uppercase tracking-wider text-primary flex items-center gap-1.5">
					<Layers className="w-3 h-3" />
					Platform · {group.member_count} app
					{group.member_count === 1 ? "" : "s"}
				</p>
				<p className="text-lg font-semibold leading-tight mt-0.5">{label}</p>
				{group.use_case && group.name && (
					<p className="text-xs text-muted-foreground">{group.name}</p>
				)}
				{group.description && (
					<p className="text-xs text-muted-foreground mt-1.5 line-clamp-2">
						{group.description}
					</p>
				)}
				<div className="flex items-center justify-between mt-3 pt-3 border-t">
					<div className="flex items-center -space-x-2">
						{group.members.slice(0, 5).map((member) => (
							<Avatar
								key={member.id}
								className="h-6 w-6 rounded-md ring-2 ring-card"
							>
								{member.app_icon ? (
									<AvatarImage src={member.app_icon} alt="" />
								) : null}
								<AvatarFallback
									className="rounded-md text-white text-[9px] font-bold"
									style={{ backgroundImage: seedGradient(member.app_id) }}
								>
									{initials(member.app_name)}
								</AvatarFallback>
							</Avatar>
						))}
					</div>
					<Badge variant="secondary" className="gap-1 text-[11px]">
						<Globe className="w-3 h-3" />
						Suite
					</Badge>
				</div>
			</div>
		</button>
	);
}

export function SuitesRail() {
	const backend = useBackend();
	const router = useRouter();
	const suites = useInvoke(
		backend.appState.getStoreGroups,
		backend.appState,
		[0, 12],
	);
	const groups = suites.data ?? [];

	if (suites.data && groups.length === 0) return null;
	if (!suites.data) return null;

	return (
		<section className="space-y-3 mb-8">
			<div className="flex items-baseline gap-3">
				<h2 className="text-lg font-semibold tracking-tight">
					Suites &amp; Platforms
				</h2>
				<span className="text-xs text-muted-foreground">
					Related apps, grouped as one
				</span>
			</div>
			<div className="flex gap-4 overflow-x-auto pb-3 -mx-1 px-1">
				{groups.map((group) => (
					<SuiteCard
						key={group.id}
						group={group}
						onOpen={(g) => router.push(`/store/suite?id=${g.id}`)}
					/>
				))}
			</div>
		</section>
	);
}

function MemberTile({ member }: Readonly<{ member: IGroupMember }>) {
	const isPrimary = member.kind === "PRIMARY";
	return (
		<div className="flex items-center gap-3 rounded-xl border bg-card p-3">
			<Avatar className="h-10 w-10 rounded-lg">
				{member.app_icon ? <AvatarImage src={member.app_icon} alt="" /> : null}
				<AvatarFallback
					className="rounded-lg text-white text-xs font-bold"
					style={{ backgroundImage: seedGradient(member.app_id) }}
				>
					{initials(member.app_name)}
				</AvatarFallback>
			</Avatar>
			<div className="min-w-0 flex-1">
				<p className="text-sm font-medium truncate flex items-center gap-2">
					{member.app_name ?? member.app_id}
					{isPrimary && (
						<Badge variant="outline" className="text-[10px]">
							Anchor
						</Badge>
					)}
				</p>
				{member.app_description && (
					<p className="text-xs text-muted-foreground truncate">
						{member.app_description}
					</p>
				)}
			</div>
		</div>
	);
}

export function SuiteDetail({ groupId }: Readonly<{ groupId: string }>) {
	const backend = useBackend();
	const router = useRouter();
	const suite = useInvoke(backend.appState.getStoreGroup, backend.appState, [
		groupId,
	]);
	const group = suite.data;

	if (!group) {
		return (
			<div className="p-10 text-center text-muted-foreground">
				Loading suite…
			</div>
		);
	}

	const label = group.use_case || group.name || "Suite";

	return (
		<div className="max-w-5xl mx-auto p-6 space-y-6">
			<Button variant="ghost" size="sm" onClick={() => router.back()}>
				<ArrowLeft className="w-4 h-4 mr-1.5" />
				Back to store
			</Button>

			<div className="relative rounded-2xl overflow-hidden border shadow-sm">
				<div
					className="h-44 relative"
					style={{
						backgroundImage: group.banner ? undefined : seedGradient(group.id),
					}}
				>
					{group.banner && (
						// eslint-disable-next-line @next/next/no-img-element
						<img
							src={group.banner}
							alt=""
							className="absolute inset-0 h-full w-full object-cover"
						/>
					)}
					<div className="absolute inset-0 opacity-40" style={DOT_TEXTURE} />
				</div>
				<div className="px-6 pb-6 -mt-14 flex items-end gap-4">
					<Avatar className="h-24 w-24 rounded-3xl ring-4 ring-card shadow-lg">
						{group.icon ? <AvatarImage src={group.icon} alt="" /> : null}
						<AvatarFallback
							className="rounded-3xl text-white text-3xl font-bold"
							style={{ backgroundImage: seedGradient(group.id) }}
						>
							{initials(group.name)}
						</AvatarFallback>
					</Avatar>
					<div className="pb-1">
						<p className="text-[11px] font-mono uppercase tracking-wider text-primary flex items-center gap-1.5">
							<Layers className="w-3 h-3" />
							Platform · {group.member_count} app
							{group.member_count === 1 ? "" : "s"}
						</p>
						<h1 className="text-2xl font-bold tracking-tight">{label}</h1>
						{group.use_case && group.name && (
							<p className="text-sm text-muted-foreground">{group.name}</p>
						)}
					</div>
				</div>
				{group.description && (
					<p className="px-6 pb-6 text-sm text-muted-foreground max-w-prose">
						{group.description}
					</p>
				)}
			</div>

			<div className="space-y-3">
				<h2 className="text-sm font-medium text-muted-foreground uppercase tracking-wide">
					Apps in this suite
				</h2>
				<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
					{group.members.map((member) => (
						<MemberTile key={member.id} member={member} />
					))}
				</div>
			</div>
		</div>
	);
}
