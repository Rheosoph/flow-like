---
title: Read QR-/Barcode
description: Detects and decodes QR codes and barcodes from images.
---

## Purpose of the Node
This node reads and decodes QR codes and barcodes from an input image. It can detect multiple codes of various types, including inverted light-on-dark codes, and can apply built-in preprocessing for difficult industrial scans.

## Pins
| Pin Name | Pin Description | Pin Type | Value Type |
|:----------:|:-------------:|:------:|:------:|
| Start | Initiate Execution | Execution | N/A |
| image_in | Image object | Struct | NodeImage |
| options | Barcode decoding options | Struct | ReadBarcodeOptions |
| Results | Detected/Decoded Codes | Array of Struct | Barcode |
| End | Done with the Execution | Execution | N/A |

`ReadBarcodeOptions` supports `filter`, `format`, `expected_formats`, `try_harder`, `also_inverted`, `pure_barcode`, `preprocess`, `validation`, and `preprocessing`.

`preprocess` accepts `None`, `Fallback`, `Aggressive`, or `Industrial`. `preprocessing` configures the node's internal image recovery pipeline with `polarity`, `roi`, `rotations`, `contrast_stretch`, `local_contrast`, `denoise`, `sharpen`, `otsu_threshold`, `adaptive_threshold`, `morphology`, `local_contrast_tile_size`, `adaptive_threshold_window`, `adaptive_threshold_bias`, `upscale_factor`, `max_upscale_pixels`, `max_variants`, and `max_decode_attempts`.

For industrial setups, use `expected_formats` to constrain decode formats, `roi` to crop a fixed camera region, `rotations` to retry known orientations, `polarity` as `Auto`, `DarkOnLight`, or `LightOnDark`, and `validation` to reject false positives by text length, prefix, suffix, contains, or explicit allowed values.
