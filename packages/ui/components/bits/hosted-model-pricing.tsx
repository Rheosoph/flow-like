"use client";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "../ui/button";
import {
	HOSTED_PRICING_FIELDS,
	record,
	updateBitPricingField,
	validateBitPricing,
} from "./bit-editor-model";
import { HostedPricingField } from "./hosted-pricing-field";

export function HostedPricing({
	parameters,
	onChange,
}: {
	parameters: Record<string, unknown>;
	onChange: (value: unknown) => void;
}) {
	const configured = parameters.pricing != null;
	const pricing = record(parameters.pricing);
	const error = validateBitPricing(parameters.pricing);
	return (
		<div className="space-y-4 rounded-xl border bg-background p-4">
			<div className="flex items-start justify-between gap-3">
				<div>
					<h4 className="text-sm font-medium">Hosted model pricing</h4>
					<p className="mt-1 text-xs text-muted-foreground">
						These rates are saved with the bit and used to calculate hosted AI
						usage.
					</p>
				</div>
				{configured && (
					<Button
						variant="ghost"
						size="sm"
						onClick={() => {
							const { pricing: _pricing, ...next } = parameters;
							onChange(next);
						}}
					>
						<Trash2 className="size-3.5" />
						Remove pricing
					</Button>
				)}
			</div>
			{configured ? (
				<>
					<div className="grid gap-4 sm:grid-cols-2">
						{HOSTED_PRICING_FIELDS.map(({ key, label, optional }) => (
							<HostedPricingField
								key={key}
								label={label}
								optional={optional}
								value={pricing[key]}
								onChange={(text) =>
									onChange(updateBitPricingField(parameters, key, text))
								}
							/>
						))}
					</div>
					<p className="text-xs text-muted-foreground">
						Enter amounts in USD with up to 6 decimal places. Zero is allowed.
						Leave the request fee blank for no fee.
					</p>
					{error && (
						<p role="alert" className="text-xs text-destructive">
							{error}
						</p>
					)}
				</>
			) : (
				<>
					<output className="block text-sm text-amber-700 dark:text-amber-400">
						No price is configured. Requests still run, and the server logs a
						warning.
					</output>
					<Button
						variant="outline"
						size="sm"
						onClick={() => onChange({ ...parameters, pricing: {} })}
					>
						<Plus className="size-3.5" />
						Add pricing
					</Button>
				</>
			)}
		</div>
	);
}
