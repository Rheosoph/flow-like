#!/bin/bash
# Fetch GitHub avatars for Gource visualization

size=90
output_dir=".git/avatar"

if [ ! -d ".git" ]; then
    echo "Error: no .git/ directory found in current path"
    exit 1
fi

mkdir -p "$output_dir"

echo "Fetching GitHub avatars for authors..."

# Extract GitHub usernames from GitHub email addresses
git log --all --pretty=format:"%ae|%an" | sort -u | while IFS='|' read -r email author; do
    author_image_file="$output_dir/$author.png"

    # Skip if image already exists
    if [ -f "$author_image_file" ]; then
        echo "Skipping '$author' (image already exists)"
        continue
    fi

    # Check if it's a GitHub noreply email and extract username
    if [[ $email =~ ^([0-9]+)\+([^@]+)@users\.noreply\.github\.com$ ]]; then
        github_username="${BASH_REMATCH[2]}"
        avatar_url="https://avatars.githubusercontent.com/u/${BASH_REMATCH[1]}?s=${size}&v=4"

        echo "Fetching GitHub avatar for '$author' (@$github_username)..."

        http_code=$(curl -s -w "%{http_code}" -L -o "$author_image_file" "$avatar_url")

        if [ "$http_code" = "200" ]; then
            echo "  ✓ Successfully fetched avatar for '$author'"
        else
            echo "  ✗ Failed to fetch avatar for '$author', removing file"
            rm -f "$author_image_file"
        fi
    elif [[ $email =~ \[bot\]@users\.noreply\.github\.com$ ]]; then
        # Handle bot accounts
        echo "Skipping bot account: '$author'"
    else
        # For regular email addresses, try to fetch from Gravatar as fallback
        email_lower=$(echo "$email" | tr '[:upper:]' '[:lower:]')
        email_hash=$(echo -n "$email_lower" | md5)
        grav_url="https://www.gravatar.com/avatar/${email_hash}?d=404&size=${size}"

        echo "Fetching Gravatar for '$author' <$email>..."

        http_code=$(curl -s -w "%{http_code}" -o "$author_image_file" "$grav_url")

        if [ "$http_code" = "200" ]; then
            echo "  ✓ Successfully fetched gravatar for '$author'"
        else
            echo "  ✗ No avatar found for '$author'"
            rm -f "$author_image_file"
        fi
    fi

    # Be nice to servers
    sleep 0.3
done

echo ""
echo "Avatar fetch complete!"
echo "Images stored in: $output_dir"
ls -lh "$output_dir" 2>/dev/null || echo "No avatars were fetched"
