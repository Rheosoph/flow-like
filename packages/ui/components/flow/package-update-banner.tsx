"use client";

import { ArrowUpCircle, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { PackageUpdate } from "@/lib/schema/wasm";

export type { PackageUpdate };

export interface PackageUpdateBannerProps {
  updates: PackageUpdate[];
  onReview: () => void;
  onDismiss: () => void;
}

export function PackageUpdateBanner({ updates, onReview, onDismiss }: PackageUpdateBannerProps) {
  if (updates.length === 0) return null;

  return (
    <div className="flex items-center gap-3 rounded-md border border-amber-200 bg-amber-50 px-4 py-2 dark:border-amber-800 dark:bg-amber-950/50">
      <ArrowUpCircle className="h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
      <span className="text-sm text-amber-800 dark:text-amber-200">
        {updates.length === 1
          ? `Update available for ${updates[0].packageName}`
          : `${updates.length} package updates available`}
      </span>
      <div className="flex items-center gap-2 ml-auto">
        <Button variant="outline" size="sm" onClick={onReview}>
          Review
        </Button>
        <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onDismiss}>
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}
