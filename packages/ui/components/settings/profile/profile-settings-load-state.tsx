import { Loader2 } from "lucide-react";
import { Button } from "../../ui/button";

export function ProfileSettingsLoadState({
	error,
	onRetry,
}: { error: unknown; onRetry: () => void }) {
	return (
		<main className="flex flex-1 min-h-0 items-center justify-center p-6">
			{error ? (
				<div role="alert" className="max-w-sm space-y-4 text-center">
					<h1 className="text-xl font-semibold">
						Profile settings could not load
					</h1>
					<p className="text-sm text-muted-foreground">
						Check your connection and try again.
					</p>
					<Button onClick={onRetry}>Try again</Button>
				</div>
			) : (
				<output className="flex items-center gap-3 text-sm text-muted-foreground">
					<Loader2 aria-hidden="true" className="h-5 w-5 animate-spin" />{" "}
					Loading profile settings…
				</output>
			)}
		</main>
	);
}
