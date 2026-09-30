---
title: Offline access to online Apps
description: Keep table data on a device, queue changes, and resolve synchronization problems.
---

**Offline access** lets a supported Desktop client use selected tables from an
online App while its hub is unreachable. The App stays online and shared;
this device keeps a local copy and queues changes for the hub.

This differs from a [local-only App](/apps/offline-online/), which lives on one
device and has no server synchronization target.

## Before you disconnect

Use a signed-in Desktop client with **Data → Offline access** available in the
App. The entry appears only on hosts that implement offline writes, for online
Apps and accounts with permission to execute Events. The hub must also support
offline changes. If the page reports that it does not, update the hub before
enabling tables. This guide describes clients that expose these controls;
older releases can have a different feature set.

Table write permissions are checked when changes reach the hub. Downloading a
copy does not grant new access rights. Your Flow must also support local
execution, and any external APIs or models it calls need their own connectivity.

## Make a table available

1. Connect to the hub and open the App's **Data → Offline access** page.
2. In **Tables available offline**, choose the table to enable.
3. Select a text or whole-number column whose values uniquely identify rows.
   Offline writes match rows using this key.
4. Turn on **Download everything** when the entire table must remain available
   without a connection. Otherwise data downloads as Flows use it.
5. Review the setup details and confirm. Wait for **Ready** and, for a complete
   copy, **Fully available offline** before disconnecting.

Table names with dots or special characters are not supported by this feature.
The setup dialog explains why a table or key is unavailable. A key without a
suitable index can require a full-table download for changes; choose Download
everything before planning offline writes in that case.

**Ready** describes setup, not how much data has downloaded. A partial copy can
only answer from data already on the device. An unpinned full copy can later
be removed to free space; Download everything keeps it selected for offline use.

## What changes after enabling a table

Flows on this device read the local copy and queue table changes even while
connected. The client sends changes in order, in batches. Changes made by
others arrive through refreshes, so this view can lag the hub. The page reports
the copy's update time and any refresh error.

Structure changes, branches, and graph operations stay unavailable for an
offline-managed table on this device. Use the online management path for those
operations. Large imports through the queue can take longer than direct writes.

New files are kept locally and uploaded later. Review **Waiting to sync** after
a Flow finishes; successful local execution does not prove that every queued
write has reached the hub.

## Reconnect and check synchronization

1. Restore the connection and sign in as the account that created the changes.
2. Open **Offline access** and select **Sync now**.
3. Check the sync status and **Waiting to sync**. Inspect any blocked head change.
4. Confirm that the queue is empty and the status is **Synced** before treating
   the device's work as delivered.

Changes belong to their originating account. Work from another account waits
until that account signs in; switching accounts does not send those changes as
the new user.

| State or problem | What to do |
| --- | --- |
| Waiting for connection | Reconnect to the configured hub, then retry synchronization. |
| Waiting for sign-in | Sign in as the named account. |
| Permission denied | Restore the required App access, then retry, or deliberately skip the change. |
| Hub rejects the size | Ask the operator to adjust the applicable limit or skip the change. |
| Outcome unknown | The hub may already have applied the change. Retry to check its outcome. |
| Table conflict | The hub copy changed after the local change was made. Inspect both sides; skipping discards this queued change so later work can continue. |
| File conflict | Use **Keep both** to upload the local version under another name, or skip it. |

**Skip** removes a change from this device's queue and requires a reason. It
cannot undo a write the hub already applied. Later changes to the same resource
can continue, so inspect dependent work before skipping an earlier operation.
Use **Find a change by ID** when investigating a specific queued operation.

## Storage and limits

**Storage on this device** reports queue usage, offline-copy usage, and download
allowances. Its settings include the queued-changes limit, offline-copies limit,
and oldest allowed change. Keep enough capacity for the tables and uploads you
expect before starting work without a connection.

A stale or failed download needs attention even when a previous local copy is
usable. Check the displayed update time and test the actual Flow against its
required data before relying on a disconnected run.

See [Offline vs. Online](/apps/offline-online/) for execution modes and
[Storage](/apps/storage/) for shared and per-user files.
