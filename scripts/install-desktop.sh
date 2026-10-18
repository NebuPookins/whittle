#!/usr/bin/env bash
# Install a .desktop launcher and icon into the current user's XDG
# directories. The launcher runs scripts/launch.sh from this checkout, so it
# always starts the current code; re-run this only if the checkout moves or
# the launcher template changes. Pass --uninstall to remove them.
set -euo pipefail

cd "$(dirname "$0")/.."

app_id=dev.whittle.app
root="$PWD"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
apps_dir="$data_home/applications"
icon_theme_dir="$data_home/icons/hicolor"

desktop_path="$apps_dir/$app_id.desktop"
icon_path="$icon_theme_dir/512x512/apps/$app_id.png"

refresh_caches() {
    update-desktop-database "$apps_dir" 2>/dev/null || true
    gtk-update-icon-cache -q -t "$icon_theme_dir" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -fv "$desktop_path" "$icon_path"
    refresh_caches
    exit 0
fi

install -Dm644 src-tauri/icons/icon.png "$icon_path"
mkdir -p "$apps_dir"
sed "s|@ROOT@|$root|g" "packaging/$app_id.desktop.in" > "$desktop_path"
chmod 644 "$desktop_path"

if command -v desktop-file-validate >/dev/null; then
    desktop-file-validate "$desktop_path"
fi
refresh_caches

echo "Installed:"
echo "  $icon_path"
echo "  $desktop_path -> $root/scripts/launch.sh"
