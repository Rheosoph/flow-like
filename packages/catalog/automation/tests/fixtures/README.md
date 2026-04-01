# Template Matching Test Fixtures

This directory contains test images for template matching accuracy and performance tests.

## Image Sources

### RustAutoGUI Test Images (`algorithm_tests/`)

**Source:** [DavorMar/rustautogui](https://github.com/DavorMar/rustautogui)
**License:** MIT License
**Paper:** [Segmented Normalized Cross-Correlation (arXiv:2502.01286)](https://arxiv.org/abs/2502.01286)

Images from `tests/testing_images/algorithm_tests/`:
- `Darts_main.png` + `Darts_template{1,2,3}.png`
- `Socket_main.png` + `Socket_template{1,2,3}.png`
- `Split_main.png` + `Split_template{1,2,3,4,5}.png`

These images are used for template matching algorithm validation with known ground truth positions.

### PyAutoGUI-Inspired Synthetic Images (`synthetic/`)

**Inspired by:** [asweigart/pyautogui](https://github.com/asweigart/pyautogui)
**PyAutoGUI License:** BSD-3-Clause

Simple colored test squares generated programmatically:
- `100x100_blue.png` - Solid blue square
- `100x100_red.png` - Solid red square
- `25x25_blue.png` - Small blue square (used for negative tests)
- `gradient_100x100.png` - Gradient pattern for edge detection tests
- `checkerboard_100x100.png` - High-frequency pattern for robustness tests

These are self-generated and not copied from PyAutoGUI, but follow the same simple-image testing philosophy.

## Ground Truth Positions

From `rustautogui` tests (top-left corner coordinates):

| Main Image | Template | Position (x, y) |
|------------|----------|-----------------|
| Darts_main | template1 | (206, 1) |
| Darts_main | template2 | (60, 270) |
| Darts_main | template3 | (454, 31) |
| Socket_main | template1 | (197, 345) |
| Socket_main | template2 | (81, 825) |
| Socket_main | template3 | (359, 666) |
| Split_main | template1 | (969, 688) |
| Split_main | template2 | (713, 1389) |
| Split_main | template4 | (1273, 1667) |
| Split_main | template5 | (41, 53) |

## Usage

These fixtures are used by `integration_test.rs` when the `execute` feature is enabled, which includes `rustautogui` for actual template matching validation.

```rust
#[cfg(feature = "execute")]
mod template_matching_accuracy_tests {
    // Tests that use actual rustautogui matching
}
```

## Downloading Test Images

The test images need to be downloaded from the original repositories. Run:

```bash
# From packages/catalog/automation directory
./tests/fixtures/download_fixtures.sh
```

Or manually download from:
- https://github.com/DavorMar/rustautogui/tree/main/tests/testing_images/algorithm_tests
