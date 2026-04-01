#!/bin/bash
# Download test fixtures from source repositories
# Run this script from the packages/catalog/automation directory

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURES_DIR="$SCRIPT_DIR"
ALGORITHM_TESTS_DIR="$FIXTURES_DIR/algorithm_tests"
SYNTHETIC_DIR="$FIXTURES_DIR/synthetic"

echo "=== Downloading Template Matching Test Fixtures ==="
echo ""

# Create directories
mkdir -p "$ALGORITHM_TESTS_DIR"
mkdir -p "$SYNTHETIC_DIR"

# RustAutoGUI test images (MIT License)
# Source: https://github.com/DavorMar/rustautogui
echo "Downloading RustAutoGUI test images (MIT License)..."
RUSTAUTOGUI_BASE="https://raw.githubusercontent.com/DavorMar/rustautogui/main/tests/testing_images/algorithm_tests"

# Darts images
curl -sL "$RUSTAUTOGUI_BASE/Darts_main.png" -o "$ALGORITHM_TESTS_DIR/Darts_main.png"
curl -sL "$RUSTAUTOGUI_BASE/Darts_template1.png" -o "$ALGORITHM_TESTS_DIR/Darts_template1.png"
curl -sL "$RUSTAUTOGUI_BASE/Darts_template2.png" -o "$ALGORITHM_TESTS_DIR/Darts_template2.png"
curl -sL "$RUSTAUTOGUI_BASE/Darts_template3.png" -o "$ALGORITHM_TESTS_DIR/Darts_template3.png"
echo "  ✓ Darts images downloaded"

# Socket images
curl -sL "$RUSTAUTOGUI_BASE/Socket_main.png" -o "$ALGORITHM_TESTS_DIR/Socket_main.png"
curl -sL "$RUSTAUTOGUI_BASE/Socket_template1.png" -o "$ALGORITHM_TESTS_DIR/Socket_template1.png"
curl -sL "$RUSTAUTOGUI_BASE/Socket_template2.png" -o "$ALGORITHM_TESTS_DIR/Socket_template2.png"
curl -sL "$RUSTAUTOGUI_BASE/Socket_template3.png" -o "$ALGORITHM_TESTS_DIR/Socket_template3.png"
echo "  ✓ Socket images downloaded"

# Split images
curl -sL "$RUSTAUTOGUI_BASE/Split_main.png" -o "$ALGORITHM_TESTS_DIR/Split_main.png"
curl -sL "$RUSTAUTOGUI_BASE/Split_template1.png" -o "$ALGORITHM_TESTS_DIR/Split_template1.png"
curl -sL "$RUSTAUTOGUI_BASE/Split_template2.png" -o "$ALGORITHM_TESTS_DIR/Split_template2.png"
curl -sL "$RUSTAUTOGUI_BASE/Split_template4.png" -o "$ALGORITHM_TESTS_DIR/Split_template4.png"
curl -sL "$RUSTAUTOGUI_BASE/Split_template5.png" -o "$ALGORITHM_TESTS_DIR/Split_template5.png"
echo "  ✓ Split images downloaded"

echo ""
echo "=== Creating Synthetic Test Images ==="

# Generate synthetic images using ImageMagick (if available) or create placeholder
if command -v convert &> /dev/null; then
    # 100x100 solid blue
    convert -size 100x100 xc:"#0000FF" "$SYNTHETIC_DIR/100x100_blue.png"
    # 100x100 solid red
    convert -size 100x100 xc:"#FF0000" "$SYNTHETIC_DIR/100x100_red.png"
    # 25x25 small blue
    convert -size 25x25 xc:"#0000FF" "$SYNTHETIC_DIR/25x25_blue.png"
    # Gradient pattern
    convert -size 100x100 gradient:white-black "$SYNTHETIC_DIR/gradient_100x100.png"
    # Checkerboard pattern
    convert -size 100x100 pattern:checkerboard "$SYNTHETIC_DIR/checkerboard_100x100.png"
    echo "  ✓ Synthetic images created with ImageMagick"
else
    echo "  ⚠ ImageMagick not found. Synthetic images will be generated at test runtime."
    echo "  Install with: brew install imagemagick (macOS) or apt install imagemagick (Linux)"
fi

echo ""
echo "=== Download Complete ==="
echo "Test fixtures are ready in: $FIXTURES_DIR"
echo ""
echo "Image sources:"
echo "  - RustAutoGUI: MIT License - https://github.com/DavorMar/rustautogui"
echo "  - Synthetic: Generated (inspired by PyAutoGUI testing approach)"
