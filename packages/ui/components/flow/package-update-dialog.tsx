"use client";

import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Checkbox } from "@/components/ui/checkbox";
import { ArrowRight, Package } from "lucide-react";
import type { PackageUpdate } from "@/lib/schema/wasm";

export interface PackageUpdateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  updates: PackageUpdate[];
  onUpdate: (packageIds: string[]) => void;
  loading?: boolean;
}

export function PackageUpdateDialog({
  open,
  onOpenChange,
  updates,
  onUpdate,
  loading,
}: PackageUpdateDialogProps) {
  const [selected, setSelected] = useState<Set<string>>(
    new Set(updates.map((u) => u.packageId))
  );

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    if (selected.size === updates.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(updates.map((u) => u.packageId)));
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Package Updates</DialogTitle>
          <DialogDescription>
            Review and apply available updates for WASM packages.
          </DialogDescription>
        </DialogHeader>
        <ScrollArea className="max-h-80">
          <div className="space-y-3 pr-3">
            {updates.map((u) => (
              <div
                key={u.packageId}
                className="flex items-start gap-3 rounded-md border p-3"
              >
                <Checkbox
                  checked={selected.has(u.packageId)}
                  onCheckedChange={() => toggle(u.packageId)}
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <Package className="h-4 w-4 text-muted-foreground shrink-0" />
                    <span className="font-medium text-sm truncate">
                      {u.packageName}
                    </span>
                    <div className="flex items-center gap-1">
                      <Badge variant="outline" className="text-xs">
                        {u.currentVersion}
                      </Badge>
                      <ArrowRight className="h-3 w-3 text-muted-foreground" />
                      <Badge className="text-xs">{u.latestVersion}</Badge>
                    </div>
                  </div>
                  {u.releaseNotes && (
                    <p className="mt-1 text-xs text-muted-foreground line-clamp-2">
                      {u.releaseNotes}
                    </p>
                  )}
                </div>
              </div>
            ))}
          </div>
        </ScrollArea>
        <DialogFooter className="flex items-center justify-between sm:justify-between">
          <Button variant="ghost" size="sm" onClick={toggleAll}>
            {selected.size === updates.length ? "Deselect All" : "Select All"}
          </Button>
          <Button
            disabled={selected.size === 0 || loading}
            onClick={() => onUpdate(Array.from(selected))}
          >
            {loading
              ? "Updating..."
              : `Update ${selected.size} package${selected.size !== 1 ? "s" : ""}`}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
