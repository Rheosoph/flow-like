"use client";

import { Boxes, Layers3, UserRound } from "lucide-react";
import type { IProfile } from "../../lib/schema/profile/profile";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Badge } from "../ui/badge";

export function ProfileTemplatePreview({
	profile,
	compact = false,
}: { profile: IProfile; compact?: boolean }) {
	return (
		<div
			className={`overflow-hidden rounded-2xl border bg-card ${compact ? "flex-1" : ""}`}
			data-testid="profile-template-preview"
		>
			<div className="relative aspect-[2.4/1] overflow-hidden bg-gradient-to-br from-violet-500/20 via-primary/10 to-orange-400/15">
				{profile.thumbnail ? (
					<img
						src={profile.thumbnail}
						alt=""
						className="h-full w-full object-cover"
					/>
				) : (
					<div className="absolute inset-0 flex items-center justify-end px-8">
						<Layers3
							className="h-20 w-20 rotate-[-12deg] text-primary/15"
							strokeWidth={1}
						/>
					</div>
				)}
			</div>
			<div className="relative space-y-3 p-5 pt-0">
				<Avatar className="-mt-7 h-14 w-14 rounded-2xl border-4 border-card bg-muted">
					<AvatarImage
						src={profile.icon ?? undefined}
						className="object-cover"
						alt=""
					/>
					<AvatarFallback className="rounded-xl">
						<UserRound className="h-6 w-6 text-primary" />
					</AvatarFallback>
				</Avatar>
				<div className="space-y-2">
					<h2 className="break-words text-xl font-semibold tracking-tight">
						{profile.name || "Your profile name"}
					</h2>
					<p
						className={`${compact ? "line-clamp-3" : "whitespace-pre-wrap"} break-words text-sm leading-relaxed text-muted-foreground`}
					>
						{profile.description ||
							"Describe who this profile is for and what they can do with it."}
					</p>
				</div>
				{!!profile.tags?.length && (
					<div className="flex flex-wrap gap-1.5">
						{profile.tags.slice(0, compact ? 3 : 8).map((tag) => (
							<Badge
								key={tag}
								variant="secondary"
								className="max-w-full break-all"
							>
								{tag}
							</Badge>
						))}
					</div>
				)}
				<div className="flex flex-wrap gap-4 border-t pt-3 text-xs text-muted-foreground">
					<span className="flex items-center gap-1.5">
						<Boxes className="h-3.5 w-3.5" />
						{profile.bits?.length ?? 0}{" "}
						{profile.bits?.length === 1 ? "bit" : "bits"}
					</span>
					<span className="flex items-center gap-1.5">
						<Layers3 className="h-3.5 w-3.5" />
						{profile.apps?.length ?? 0}{" "}
						{profile.apps?.length === 1 ? "app" : "apps"}
					</span>
				</div>
			</div>
		</div>
	);
}
