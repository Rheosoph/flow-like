#!/bin/bash
# Fetch avatars from GitHub for Gource visualization

size=90
output_dir=".git/avatar"

if [ ! -d ".git" ]; then
    echo "Error: no .git/ directory found in current path"
    exit 1
fi

mkdir -p "$output_dir"

echo "Fetching GitHub avatars for authors..."

# Get unique authors
git log --all --pretty=format:"%an" | sort -u | while read -r author; do
    author_image_file="$output_dir/$author.png"

    # Skip if image already exists
    if [ -f "$author_image_file" ]; then
        echo "Skipping '$author' (image already exists)"
        continue
    fi

    echo "Fetching avatar for '$author'..."

    # Try to fetch from GitHub
    # First try as username directly
    github_url="https://github.com/${author}.png?size=${size}"

    http_code=$(curl -L -s -w "%{http_code}" -o "$author_image_file" "$github_url")

    if [ "$http_code" != "200" ]; then
        echo "  No GitHub avatar found for '$author', removing file"
        rm -f "$author_image_file"
    else
        echo "  Successfully fetched GitHub avatar for '$author'"
    fi

    # Be nice to GitHub servers
    sleep 0.3
done

echo ""
echo "GitHub avatar fetch complete!"
echo "Images stored in: $output_dir"
