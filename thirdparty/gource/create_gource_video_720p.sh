#!/bin/bash
# Create a 720p Gource visualization video for flow-like (faster rendering)

echo "Creating flow-like development visualization (720p)..."
echo "This will take some time depending on your git history size."
echo ""

# Create output directory if it doesn't exist
mkdir -p output

gource \
  -1280x720 \
  --seconds-per-day 1 \
  --auto-skip-seconds 1 \
  --multi-sampling \
  --title "365+ days of Flow-Like" \
  --caption-file gource_captions.txt \
  --caption-size 20 \
  --caption-duration 5 \
  --date-format "%B %d, %Y" \
  --hide mouse,progress \
  --file-idle-time 0 \
  --max-file-lag 0.5 \
  --bloom-multiplier 1.2 \
  --bloom-intensity 0.5 \
  --camera-mode track \
  --user-image-dir ../../.git/avatar/ \
  --user-scale 1 \
  --dir-name-depth 3 \
  --font-size 18 \
  --elasticity 0.1 \
  -o - \
  | ffmpeg -y -r 60 -f image2pipe -vcodec ppm -i - \
    -vcodec libx264 \
    -preset ultrafast \
    -pix_fmt yuv420p \
    -crf 18 \
    -threads 0 \
    -bf 2 \
    -movflags +faststart \
    output/flow-like-gource-720p.mp4

echo ""
echo "Video created successfully!"
echo "Output: output/flow-like-gource-720p.mp4"
