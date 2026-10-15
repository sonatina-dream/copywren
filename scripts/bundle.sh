#!/usr/bin/env bash
# Builds dist/Copywren.app: a release build wrapped in a menu-bar-agent bundle
# (no Dock icon) and signed ad hoc, so macOS can remember its Accessibility
# permission and "Open at login" works. Run scripts/make-icons.sh first if
# you changed assets/*.svg.
set -euo pipefail

cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=dist/Copywren.app

cargo build --release -p copywren

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/copywren "$APP/Contents/MacOS/Copywren"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>         <string>io.github.sonatina-dream.copywren</string>
    <key>CFBundleName</key>               <string>Copywren</string>
    <key>CFBundleDisplayName</key>        <string>Copywren</string>
    <key>CFBundleExecutable</key>         <string>Copywren</string>
    <key>CFBundlePackageType</key>        <string>APPL</string>
    <key>CFBundleIconFile</key>           <string>AppIcon</string>
    <key>CFBundleShortVersionString</key> <string>${VERSION}</string>
    <key>CFBundleVersion</key>            <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>     <string>14.0</string>
    <key>LSUIElement</key>                <true/>
    <key>NSHighResolutionCapable</key>    <true/>
    <key>LSApplicationCategoryType</key>  <string>public.app-category.productivity</string>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP"
echo "Built $APP ($VERSION)"
