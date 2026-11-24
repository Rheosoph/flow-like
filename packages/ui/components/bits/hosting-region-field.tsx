"use client";
import {
	HOSTING_REGIONS,
	HOSTING_REGION_KEY,
	type HostingRegion,
	parseHostingRegion,
	withHostingRegion,
} from "../../lib/bit/hosting-region";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";

const UNSET = "unset";
const REGION_LABELS: Record<HostingRegion, string> = {
	eu: "European Union",
};

export function HostingRegionField({
	parameters,
	onChange,
}: {
	parameters: Record<string, unknown>;
	onChange: (value: unknown) => void;
}) {
	const stored = parameters[HOSTING_REGION_KEY];
	const region = parseHostingRegion(stored);
	return (
		<div className="space-y-3 rounded-xl border bg-background p-4">
			<div>
				<label htmlFor="bit-hosting-region" className="text-sm font-medium">
					Hosting region
				</label>
				<p className="mt-1 text-xs text-muted-foreground">
					Where the upstream provider serves this model. Users see a region
					badge and can filter the model catalog by it, so only set it when
					every request on this route stays in that region.
				</p>
			</div>
			<Select
				value={region ?? UNSET}
				onValueChange={(value) =>
					onChange(withHostingRegion(parameters, parseHostingRegion(value)))
				}
			>
				<SelectTrigger id="bit-hosting-region" className="w-full sm:w-64">
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					<SelectItem value={UNSET}>Not specified</SelectItem>
					{HOSTING_REGIONS.map((option) => (
						<SelectItem key={option} value={option}>
							{REGION_LABELS[option]}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			{stored != null && !region && (
				<p role="alert" className="text-xs text-destructive">
					Unknown hosting region {JSON.stringify(stored)}. Pick a region or
					choose Not specified before saving.
				</p>
			)}
		</div>
	);
}
