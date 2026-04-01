"use client";
import { Package } from "lucide-react";
import { Skeleton } from "../ui/skeleton";

export function StoreEmptyState({
	title,
	description,
}: Readonly<{ title: string; description?: string }>) {
	return (
		<div className="flex flex-col items-center justify-center py-20 space-y-3 text-center">
			<Package className="w-10 h-10 text-muted-foreground/20" />
			<h2 className="text-base font-semibold">{title}</h2>
			{description && (
				<p className="text-sm text-muted-foreground/60 max-w-sm">
					{description}
				</p>
			)}
		</div>
	);
}

export function HeroSkeleton() {
	return (
		<section className="relative">
			<Skeleton className="h-60 md:h-75 w-full" />
			<div className="relative -mt-16 max-w-5xl mx-auto px-6 md:px-10">
				<div className="flex items-end gap-4">
					<div className="shrink-0 rounded-full bg-background/60 backdrop-blur-xl p-1">
						<Skeleton className="h-22 w-22 rounded-full" />
					</div>
					<div className="flex-1 space-y-2 pb-1">
						<Skeleton className="h-7 w-48 rounded-full" />
						<Skeleton className="h-4 w-32 rounded-full" />
					</div>
				</div>
			</div>
		</section>
	);
}
