#!/bin/bash
# Create a 4K Gource visualization video for flow-like (highest quality)

echo "Creating flow-like development visualization (4K Ultra HD)..."
echo "WARNING: This will take a LONG time and produce a large file!"
echo ""

# Create output directory if it doesn't exist
mkdir -p output

gource \
  -3840x2160 \
  --seconds-per-day 3 \
  --auto-skip-seconds 1 \
  --multi-sampling \
  --title "365+ days of Flow-Like" \
  --caption-file gource_captions.txt \
  --caption-size 32 \
  --caption-duration 5 \
  --date-format "%B %d, %Y" \
  --hide mouse,progress \
  --file-idle-time 0 \
  --max-file-lag 0.5 \
  --bloom-multiplier 1.5 \
  --bloom-intensity 0.75 \
  --camera-mode track \
  --dir-name-depth 3 \
  --font-size 36 \
  --elasticity 0.1 \
  -o - \
  | ffmpeg -y -r 60 -f image2pipe -vcodec ppm -i - \
    -vcodec libx264 \
    -preset veryslow \
    -pix_fmt yuv420p \
    -crf 15 \
    -threads 0 \
    -bf 2 \
    -movflags +faststart \
    output/flow-like-gource-4k.mp4

echo ""
echo "Video created successfully!"
echo "Output: output/flow-like-gource-4k.mp4"
