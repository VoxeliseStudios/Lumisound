#!/bin/sh
# Ad-hoc signs a built .app (no certificate) so the IPA ships signed and
# carries its entitlements: SideStore / AltStore / Sideloadly / LiveContainer
# copy entitlements from the IPA when they re-sign, so App Groups must be in
# here or extensions can't share data with the app.
#
#   adhoc-sign.sh <App.app> <app.entitlements> [extension.entitlements]
#
# Order: inner code first (frameworks, dylibs), then every extension
# (PlugIns/*.appex, Watch/*.app) with its own entitlements, then the app.
# Then verifies every part and exits non-zero if anything is unsigned or
# lacks an App Group (the release must not publish such an IPA).
set -eu

APP="$1"
APP_ENT="$2"
EXT_ENT="${3:-$2}"

[ -d "$APP" ] || { echo "::error::no app at $APP"; exit 1; }
[ -f "$APP_ENT" ] || { echo "::error::no entitlements at $APP_ENT"; exit 1; }

sign() { codesign --force --sign - --timestamp=none --generate-entitlement-der "$@"; }

# Per-extension ad-hoc entitlements (scripts/adhoc/<Name>.adhoc.entitlements)
# when there is one, else the shared extension file. Only files in
# scripts/adhoc are used: the targets' own .entitlements can carry push,
# HealthKit, CarPlay and other paid entitlements that make a free-Apple-ID
# install fail, so an ad-hoc IPA embeds App Groups only.
ent_for() {
  name=$(basename "$1"); name="${name%.*}"
  own="$(dirname "$0")/adhoc/$name.adhoc.entitlements"
  if [ -f "$own" ]; then echo "$own"; else echo "$EXT_ENT"; fi
}

sign_bundle_inner() {
  b="$1"
  if [ -d "$b/Frameworks" ]; then
    find "$b/Frameworks" -name "*.dylib" -type f | while read -r d; do sign "$d"; done
    find "$b/Frameworks" -maxdepth 1 -name "*.framework" -type d | while read -r f; do sign "$f"; done
  fi
}

sign_bundle_inner "$APP"
for ext in "$APP"/PlugIns/*.appex "$APP"/Extensions/*.appex "$APP"/Watch/*.app; do
  [ -d "$ext" ] || continue
  sign_bundle_inner "$ext"
  for inner in "$ext"/PlugIns/*.appex; do [ -d "$inner" ] && sign --entitlements "$(ent_for "$inner")" "$inner"; done
  sign --entitlements "$(ent_for "$ext")" "$ext"
done
sign --entitlements "$APP_ENT" "$APP"

# ---- verify ------------------------------------------------------------------------------
fail=0
check() {
  part="$1"
  if ! codesign --verify --strict "$part" 2>/dev/null; then
    echo "::error::not signed: $part"; fail=1; return
  fi
  ents=$(codesign -d --entitlements :- "$part" 2>/dev/null || true)
  if ! printf '%s' "$ents" | grep -q "com.apple.security.application-groups"; then
    echo "::error::no App Group entitlement: $part"; fail=1; return
  fi
  echo "ok: $(basename "$part") signed, App Group present"
}
check "$APP"
for ext in "$APP"/PlugIns/*.appex "$APP"/Extensions/*.appex "$APP"/Watch/*.app; do [ -d "$ext" ] && check "$ext"; done
codesign --verify --deep --strict "$APP" || { echo "::error::deep verify failed"; fail=1; }
exit $fail
