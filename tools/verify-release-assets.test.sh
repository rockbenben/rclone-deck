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
  if [[ "${1:-} ${2:-}" == "api repos/$repo/releases/assets/123" ]]; then
    printf '%s\n' "$fixture_asset_url"
    return 0
  fi
  if [[ "${1:-} ${2:-} ${3:-}" == "api --paginate repos/$repo/releases" ]]; then
    printf '%s\n' "${fixture_release_assets:-123}"
    return 0
  fi
  return 1
}

export -f gh
export repo fixture_asset_url="$direct_url" fixture_release_assets

write_feed "$direct_url" "$signature"
verify
write_feed "$api_url" "$signature"
verify

# 草稿 release：GitHub 在 draft=false 之前只给 untagged-<hash> 地址，tag 形态要发布之后才有
untagged_url="${direct_url/"$tag"/untagged-436cb4e569d161a1116d}"
fixture_asset_url="$untagged_url"
write_feed "$api_url" "$signature"
verify

fixture_release_assets=999
expect_rejected '不属于该标签的 release'
fixture_release_assets=

fixture_asset_url="${untagged_url/$name/rclone-deck_0.0.9_x64-setup.exe}"
write_feed "$api_url" "$signature"
expect_rejected '未指向该标签'
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
