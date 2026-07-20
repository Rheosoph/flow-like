import type { ReactNode } from "react";

export function ProductDemoFrame({
	children,
	source,
	className,
}: Readonly<{
	children: ReactNode;
	source: string;
	className?: string;
}>) {
	return (
		<section
			className={["not-prose my-10 min-w-0 w-full max-w-full", className]
				.filter(Boolean)
				.join(" ")}
			data-product-ui-source={source}
		>
			{children}
			<p className="mt-2.5 flex items-center gap-1.5 px-1 text-[11px] text-muted-foreground/80">
				<span
					className="size-1.5 rounded-full bg-emerald-500"
					aria-hidden="true"
				/>
				Interactive Flow-Like product UI · sample data
			</p>
		</section>
	);
}

export function cn(
	...classes: Array<string | false | null | undefined>
): string {
	return classes.filter(Boolean).join(" ");
}
