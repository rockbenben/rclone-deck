#!/usr/bin/env bash
set -euo pipefail

verifier="$(dirname "${BASH_SOURCE[0]}")/verify-release-assets.sh"
scratch="$(mktemp -d)"
trap 'rm -f "$scratch"/{updater.pub,updater.key,tauri.conf.json,installer.minisig,latest.json,verify.log} "$installer" "$installer.sig"; rmdir "$scratch"' EXIT
repo=example/rclone-deck
tag=v0.1.1
name=rclone-deck_0.1.1_x64-setup.exe
installer="$scratch/$name"
direct_url="https://github.com/$repo/releases/download/$tag/$name"
api_url="https://api.github.com/repos/$repo/releases/assets/123"

printf 'installer fixture\n' >"$installer"
minisign -G -W -p "$scratch/updater.pub" -s "$scratch/updater.key" >/dev/null
minisign -S -s "$scratch/updater.key" -x "$scratch/installer.minisig" -m "$installer"
base64 -w0 "$scratch/installer.minisig" >"$installer.sig"
signature="$(<"$installer.sig")"
jq -n --arg pubkey "$(base64 -w0 "$scratch/updater.pub")" \
  '{productName: "rclone-deck", plugins: {updater: {pubkey: $pubkey}}}' >"$scratch/tauri.conf.json"

write_feed() {
  jq -n --arg url "$1" --arg signature "$2" \
    '{version: "0.1.1", platforms: {"windows-x86_64": {$url, $signature}}}' >"$scratch/latest.json"
}

verify() {
  bash "$verifier" "$scratch" "$repo" "$tag" "$scratch/tauri.conf.json"
}

expect_rejected() {
  if verify >"$scratch/verify.log" 2>&1 || ! grep -Fq "$1" "$scratch/verify.log"; then
    cat "$scratch/verify.log" >&2
    echo "Expected rejection: $1" >&2
    exit 1
  fi
}

gh() {
  [[ "$*" == "api repos/$repo/releases/assets/123 --jq .browser_download_url" ]] || return 1
  printf '%s\n' "$fixture_asset_url"
}

export -f gh
export repo fixture_asset_url="$direct_url"

write_feed "$direct_url" "$signature"
verify
write_feed "$api_url" "$signature"
verify
fixture_asset_url="${direct_url/"$tag"/v0.1.0}"
expect_rejected '未指向该标签'
write_feed "https://example.com/$repo/releases/download/$tag/$name" "$signature"
expect_rejected '未指向该标签'
write_feed "$direct_url" 'mismatched-fixture-signature'
expect_rejected '签名与安装包 .sig 不一致'
write_feed "$direct_url" "$signature"
printf 'mismatched sidecar' >"$installer.sig"
expect_rejected '签名与安装包 .sig 不一致'
printf '%s' "$signature" >"$installer.sig"
printf 'tampered bytes\n' >>"$installer"
expect_rejected '未通过 Tauri 更新公钥的签名验证'

echo 'Release asset verifier regression tests passed'
