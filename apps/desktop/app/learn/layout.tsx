import type { ReactNode } from "react";

export default function LearnLayout({
	children,
}: { readonly children: ReactNode }) {
	return (
		<div className="flex h-full flex-col">
			<div className="flex min-h-0 flex-1 flex-col overflow-hidden">
				{children}
			</div>
		</div>
	);
}
