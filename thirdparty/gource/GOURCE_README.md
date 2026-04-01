# Gource Visualization Scripts for flow-like

Create beautiful development visualization videos for your YouTube channel!

## Features

✨ **GitHub Avatars**: Automatically fetches contributor avatars from GitHub
📅 **Release Captions**: Shows release milestones with emojis
🎨 **File Type Legend**: Color-coded key showing file extensions (--key flag)
🎥 **Multiple Resolutions**: 720p, 1080p, and 4K options
🌟 **Optimized Settings**: Perfect balance of speed and visual appeal

## Quick Start

### 1. Fetch Contributor Avatars
```bash
./fetch_github_avatars.sh
```
This will download avatars from GitHub for all contributors.

### 2. Generate Your Video

Choose your preferred resolution:

**720p (Fast, good for testing):**
```bash
./create_gource_video_720p.sh
```

**1080p (Recommended for YouTube):**
```bash
./create_gource_video.sh
```

**4K (Best quality, slowest):**
```bash
./create_gource_video_4k.sh
```

## Output

Videos will be saved in the `output/` directory:
- `flow-like-gource-720p.mp4` - 720p version
- `flow-like-gource.mp4` - 1080p version
- `flow-like-gource-4k.mp4` - 4K version

## Customization

### Release Captions

Edit [gource_captions.txt](gource_captions.txt) to add or modify release events:
```
timestamp|🚀 Your Custom Caption
```

Get timestamps with:
```bash
git log -1 --format="%at" <commit-or-tag>
```

### Visual Settings

All scripts include these optimized settings:
- `--seconds-per-day 3` - Show 3 seconds per day of development
- `--auto-skip-seconds 1` - Skip idle periods after 1 second
- `--key` - Show file extension legend
- `--camera-mode track` - Follow active contributors
- `--multi-sampling` - Anti-aliasing for smooth visuals
- Custom bloom, elasticity, and font settings

Modify any script to adjust these parameters!

## Technical Details

### Video Encoding (FFmpeg)
- **720p**: ultrafast preset, CRF 18 (quick render)
- **1080p**: slow preset, CRF 18 (balanced)
- **4K**: veryslow preset, CRF 15 (best quality)

All use:
- 60 FPS for smooth motion
- H.264 codec for broad compatibility
- `yuv420p` pixel format (YouTube compatible)
- `+faststart` flag for web streaming

## Tips for YouTube

1. **Use 1080p** for the best balance of quality and file size
2. **Add music** in post-production (use YouTube Audio Library)
3. **Consider adding** an intro/outro card
4. **Optimize thumbnail** from an interesting moment in the video

## Examples of Great Gource Videos

Check out the [Gource Videos Wiki](https://github.com/acaudwell/Gource/wiki/Videos) for inspiration!

## Requirements

- [Gource](http://gource.io/) - `brew install gource`
- [FFmpeg](https://ffmpeg.org/) - `brew install ffmpeg`
- curl (pre-installed on macOS)

---

**Happy visualizing! 🎬**
