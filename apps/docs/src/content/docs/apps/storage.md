---
title: Storage
description: Upload and manage files for your apps
sidebar:
  order: 30
---

Every Flow-Like app has its own **file storage** for the assets your flows read and write. Two views manage it:

- **Storage**: upload and manage shared files and directories for the app. Select files from your local machine or drag and drop them directly.
- **User Storage**: browse and search your own private, per-user files within the app.

:::tip[Looking for databases?]
Relational tables and the semantic layer built on top of them now live in **[Data Studio](/apps/data-studio/)**.
:::

**User Storage** appears in the App navigation when [Developer
Mode](/start/developer-mode/) is enabled. Access permissions still apply to both
views. For queued uploads and cached tables in an online App, see
[Offline access](/apps/offline-access/).

## Upload Directory
Here you can upload files and entire folders to make them part of your app. For online apps, the files are stored in the cloud and synchronized across all devices where you are logged in with your Flow-Like account:

![A screenshot of Flow-Like Desktop showing the upload directory of an app](../../../assets/AppStorage.webp)

## Rename files and folders

In **Storage** or **User Storage**, open a file or folder's actions menu and
choose **Rename**. Enter a new name and confirm. Renaming requires permission to
write files and keeps the item in its current parent folder. A folder rename
also moves its contents. An existing file or folder with the new name blocks
the rename.

Renaming changes storage paths. Update any flows or widgets that reference the
old path, including paths inside a renamed folder.

Cloud folder renames copy the contents before deleting the original objects.
Avoid editing or uploading into the folder while it is being renamed. If a
rename fails, some copies may remain under the new name. On Amazon S3, the
conditional copy preserves file contents but does not retain the source
object's metadata or tags.

## View and edit files

Select a supported file in **Storage** to open its viewer. The editor loads on
demand. PDFs, common images, audio and video open inline beside the file list.
PDFs use the browser's reader with zoom and page controls. Audio and video keep
their playback controls. Choose **Edit** to open the corresponding editor.

If your role allows writing files, choose **Edit**, make changes, then
**Save**. Saving updates the selected file in its current folder. Closing an
editor with unsaved changes asks whether to discard them. When an editor can
import a format but cannot export it, the button names the output format, such
as **Save as .xlsx**. That action creates a distinctly named file in the same
folder and leaves the source available. Further saves update that converted file
while the editor stays open.

| Editor | Supported file families | Save behavior |
| --- | --- | --- |
| Flows Word | DOCX | Saves DOCX |
| Flows PDF | PDF | Browser reading, PDF editing and saving |
| Flows Slides | PPTX, POTX, PPSX | Saves PPTX; templates and slide shows convert to PPTX |
| Flows Sheets | XLSX, XLSM, XLTX, XLTM, CSV, TSV, TAB; imports XLS, XLT, XLSB, ODS | Saves supported modern formats; older workbooks and ODS convert to XLSX |
| Flows Vector | SVG, SVGZ, PDF-compatible AI, EPS, EMF, WMF, JPEG 2000, VectorCraft projects and templates | Saves matching formats; imported DrawCraft, Affinity and JPEG 2000 files convert to VectorCraft |
| Flows Photo | PNG, JPEG, TIFF, BMP, PSD, PSB, OpenRaster, PhotoCraft, DDS, DPX, Cineon and IFF; imports RAW, KRA, PDN, XCF, XPM and DICOM | Saves matching raster formats; import-only files and animated formats save as PhotoCraft projects |
| Flows Audio | WAV, AIFF, FLAC, MIDI, SoundCraft; imports MP3, OGG, M4A, AAC and other supported audio | Saves WAV, AIFF, FLAC, MIDI or bundled SoundCraft sessions; compressed audio converts to WAV |
| Flows Video | MP4, MOV, WebM, MKV, AVI and other supported video containers | Saves MP4 |
| Flows CAD | SolidCraft projects; imports STEP, STP, IGES, IGS, STL, OBJ, 3MF, GLB, glTF, FreeCAD and Rhino; DXF through CADCraft | Saves SolidCraft projects; DXF saves DXF |
| Flows Design | IDML, DesignCraft; imports InDesign INDD and INDT | Saves IDML or DesignCraft; InDesign files convert to DesignCraft |

These editors are available in the shared **Storage** configuration tab on web
and desktop. They run in the browser and send saved bytes through the existing
storage connection. File contents are not sent to Storytold. Browser codec
availability can limit video imports and exports.

Flows CAD uses [SolidCraft](https://github.com/tangency-labs/solidcraft) for
3D models and 2D sketches. Saving an imported model creates a `.solidcraft`
project that retains modeling edits and leaves the original file available.
GLB and glTF files must embed their geometry buffers; the viewer cannot load
separate `.bin` files beside the selected file. Textures are not rendered.
Existing DXF drawings continue to open in CADCraft and save as DXF.
FreeCAD `.FCStd` and Rhino `.3dm` imports convert geometry to a STEP base feature;
they do not retain the source application's feature history. Review the import
warnings for omitted content. Saved projects use SolidCraft format 47 and require
SolidCraft 0.30 or later. Keep an original copy when working with older versions.
SolidCraft projects and STEP models load in stages. The editor displays progress
and lets you cancel while it builds the model.

Editing is experimental. Unsupported formatting, layers, animation or document
features may change when saved. Import warnings appear above the editor. Some
RAW and layered image formats open an embedded preview rather than their full
source data. Animated images currently open a single frame, so their Save action
creates a PhotoCraft project. InDesign imports convert the document into the
layout editor's model; review the conversion warnings and resulting pages.
Layout files that reference external graphics need those assets embedded to be
self-contained. Review
important files in their original application after saving.

Word, presentation, spreadsheet, vector, photo and layout editors accept files
up to 50 MiB. Audio and CAD accept 100 MiB; PDF and video accept 200 MiB.
Legacy `.doc` and `.ppt`, OpenDocument presentations, DWG and After Effects
`.aep` files are not supported. Export DWG drawings to DXF before uploading.
TypeScript `.ts` files retain their code preview. LightCraft catalogs and
EffectCraft projects depend on separate
media libraries, so they do not open as standalone storage files.

## Control an open editor with an agent

Open the document's editor. For PDFs, common images, audio and video, choose
**Edit** from the native preview first; **Agent target** appears only after the
editor loads. For other supported files, choose **Edit** if the agent should
change them. Select **Agent target** in the editor toolbar to copy its reference.
Add **Register Editor MCP Tools** before the agent runs and connect the Agent
input. Open its **Editor Element** picker, paste the copied reference into the
search field, and choose **Use reference**. Leave **Tools** empty to expose the
tools provided by that editor, or enter the exact tool names the agent should use.

For an editor on an app page, select its `filePreview` or `fileEditor` component
directly in **Editor Element**, or connect the **Element** output of **Get Element**
to that input. **Get Element** must target the component containing the open
editor. It passes the resolved element reference to the registration node.

The agent works on the open document, including unsaved changes. Choose **Save**
to persist those changes through the editor's existing save path. Read-only
documents allow tools identified as read-only by the editor. Run the flow in
the same window and keep the document open while the agent uses it. Editor tools
can run while the Flow-Like window or browser tab is in the background.
Closing, reloading or replacing the document requires running the registration
node again.

## Models and training history

Trained models are published as files under `models` in the app's storage
folder, which flows reach through the **Storage Dir** node. The **Storage** view
lists the upload folder, so it does not show them. These models sync like any
other app file, ship to devices with a deployment and travel in app archives,
so other runtimes can run them. Forks that do not copy data leave them out.

Models trained on your own tables go to `models` in **User Storage** instead.
They sync for online apps and reach a device only with the data of the user a
deployment selects; app archives leave them out.

Model folders are written once and checked by sha256; a file edited by hand
there stops its model from loading.

The training history behind these models stays on the computer or device that
trained them. Only **Export Training Ledger** and **Import Training Ledger**
move it. See [Training history and published
models](/topics/datascience/ml-auto-training/#training-history-and-published-models).
