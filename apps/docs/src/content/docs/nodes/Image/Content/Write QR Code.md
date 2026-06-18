---
title: Write Barcode
description: Encodes text into a barcode image and outputs it as a NodeImage.
---

## Purpose of the Node
This node generates a barcode image from input text and outputs it as a NodeImage object. Supported formats include QR Code, Data Matrix, Aztec, PDF417, Code 128, Code 39, Code 93, Codabar, ITF, EAN-8, EAN-13, UPC-A, UPC-E, and Telepen.

## Pins
| Pin Name | Pin Description | Pin Type | Value Type |
|:----------:|:-------------:|:------:|:------:|
| Start | Initiate Execution | Execution | N/A |
| data | Text to encode | String | |
| format | Barcode Format | String | QR_CODE |
| scale | Pixels per barcode module | Integer | 8 |
| margin | Quiet zone in modules | Integer | 4 |
| End | Done with the Execution | Execution | N/A |
| image_out | Barcode image | Struct | NodeImage |
