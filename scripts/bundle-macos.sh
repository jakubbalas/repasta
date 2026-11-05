#!/bin/sh
# Builds target/Repasta.app from the desktop preview.
# Pass --install to copy it into ~/Applications.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
app="$root/target/Repasta.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)

cargo build --manifest-path "$root/Cargo.toml" --locked --offline --release \
  --features desktop --bin repasta-desktop

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$root/target/release/repasta-desktop" "$app/Contents/MacOS/Repasta"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
iconset="$work/Repasta.iconset"
mkdir "$iconset"
swift "$root/scripts/macos-icon.swift" "$work/icon.png" 1024
for px in 16 32 128 256 512; do
  sips -z "$px" "$px" "$work/icon.png" --out "$iconset/icon_${px}x${px}.png" >/dev/null
  double=$((px * 2))
  sips -z "$double" "$double" "$work/icon.png" --out "$iconset/icon_${px}x${px}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/Repasta.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Repasta</string>
  <key>CFBundleDisplayName</key><string>Repasta</string>
  <key>CFBundleIdentifier</key><string>io.repasta.desktop</string>
  <key>CFBundleExecutable</key><string>Repasta</string>
  <key>CFBundleIconFile</key><string>Repasta</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- Menu bar app: no Dock icon; the menu bar item shows and quits it. -->
  <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature for local use only; this is not a distributable build.
codesign --force --sign - "$app" >/dev/null 2>&1 || true
echo "Built $app"

if [ "${1:-}" = "--install" ]; then
  mkdir -p "$HOME/Applications"
  rm -rf "$HOME/Applications/Repasta.app"
  cp -R "$app" "$HOME/Applications/Repasta.app"
  echo "Installed $HOME/Applications/Repasta.app"
fi
