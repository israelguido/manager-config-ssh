#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="Gerenciador SSH"
EXECUTABLE_NAME="manager-config-file"
BUNDLE_ID="${BUNDLE_ID:-br.com.israelguido.manager-config-file}"
VERSION="$(awk -F '"' '/^version = / { print $2; exit }' "$ROOT_DIR/Cargo.toml")"
ARCH="$(uname -m)"
DIST_DIR="$ROOT_DIR/dist"
STAGING_DIR="$DIST_DIR/pkg-root"
APP_DIR="$STAGING_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
COMPONENT_PLIST="$DIST_DIR/components.plist"
PKG_PATH="$DIST_DIR/$EXECUTABLE_NAME-$VERSION-$ARCH.pkg"

echo "Compilando a versão release..."
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" --release \
    --jobs "${CARGO_BUILD_JOBS:-1}"

rm -rf "$STAGING_DIR" "$PKG_PATH" "$COMPONENT_PLIST"
mkdir -p "$CONTENTS_DIR/MacOS"

install -m 755 \
    "$ROOT_DIR/target/release/$EXECUTABLE_NAME" \
    "$CONTENTS_DIR/MacOS/$EXECUTABLE_NAME"

cat > "$CONTENTS_DIR/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>pt_BR</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$EXECUTABLE_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

cat > "$COMPONENT_PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
    <dict>
        <key>BundleHasStrictIdentifier</key>
        <true/>
        <key>BundleIsRelocatable</key>
        <false/>
        <key>BundleIsVersionChecked</key>
        <false/>
        <key>BundleOverwriteAction</key>
        <string>upgrade</string>
        <key>RootRelativeBundlePath</key>
        <string>$APP_NAME.app</string>
    </dict>
</array>
</plist>
EOF

if [[ -n "${DEVELOPER_ID_APPLICATION:-}" ]]; then
    echo "Assinando o aplicativo com Developer ID..."
    codesign --force --options runtime --timestamp \
        --sign "$DEVELOPER_ID_APPLICATION" "$APP_DIR"
else
    echo "Aplicando assinatura ad hoc para uso local..."
    codesign --force --deep --sign - "$APP_DIR"
fi

PKG_ARGUMENTS=(
    --root "$STAGING_DIR"
    --component-plist "$COMPONENT_PLIST"
    --install-location /Applications
    --identifier "$BUNDLE_ID"
    --version "$VERSION"
)

if [[ -n "${DEVELOPER_ID_INSTALLER:-}" ]]; then
    PKG_ARGUMENTS+=(--sign "$DEVELOPER_ID_INSTALLER")
fi

echo "Criando o instalador..."
pkgbuild "${PKG_ARGUMENTS[@]}" "$PKG_PATH"
rm -rf "$STAGING_DIR" "$COMPONENT_PLIST"

echo
echo "Pacote criado em: $PKG_PATH"
