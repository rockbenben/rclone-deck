#!/usr/bin/env bash
set -euo pipefail

dist="$1"
repo="$2"
tag="$3"
config="$4"
version="${tag#v}"
feed="$dist/latest.json"

fail() {
  echo "::error::$*" >&2
  exit 1
}

jq -e --arg version "$version" '.version == $version' "$feed" >/dev/null ||
  fail 'latest.json 的版本与标签不一致'
product="$(jq -er '.productName | strings | select(length > 0)' "$config")"
name="${product}_${version}_x64-setup.exe"
canonical_url="https://github.com/$repo/releases/download/$tag/$name"
api_prefix="https://api.github.com/repos/$repo/releases/assets/"
url="$(jq -er '.platforms["windows-x86_64"].url | strings | select(length > 0)' "$feed")" ||
  fail 'latest.json 缺少 Windows x64 下载地址'
signature="$(jq -er '.platforms["windows-x86_64"].signature | strings | select(length > 0)' "$feed")" ||
  fail 'latest.json 缺少 Windows x64 更新签名'

if [[ "$url" == "$api_prefix"* ]]; then
  asset_id="${url#"$api_prefix"}"
  [[ "$asset_id" =~ ^[0-9]+$ ]] || fail 'latest.json 的 GitHub 资产 ID 无效'
  url="$(gh api "repos/$repo/releases/assets/$asset_id" --jq .browser_download_url)"
else
  asset_id=''
fi

# 草稿 release 的资产地址拿 `untagged-<hash>` 当路径段：GitHub 要到 draft=false 之后才把它
# 改回 `releases/download/<tag>/`。release job 建的就是草稿，所以这一步本来只会看到 untagged
# 形态；只认 tag 形态等于让第一次发布永远红在这一行。草稿那种改判「这个资产 id 属于该标签的
# release」——地址换了个路径段，指的是同一份产物。
if [[ "$url" != "$canonical_url" ]]; then
  [[ "$url" == "https://github.com/$repo/releases/download/untagged-"*"/$name" ]] ||
    fail 'latest.json 未指向该标签的 Windows x64 安装包'
  [[ -n "$asset_id" ]] || fail 'latest.json 指向的安装包没有资产 ID，核不了归属'
  release_assets="$(gh api --paginate "repos/$repo/releases" \
    --jq ".[] | select(.tag_name == \"$tag\") | .assets[].id")" ||
    fail '读不到该标签 release 的资产列表'
  grep -qx "$asset_id" <<<"$release_assets" ||
    fail 'latest.json 指向的资产不属于该标签的 release'
fi
[[ -s "$dist/$name" && -s "$dist/$name.sig" ]] || fail '缺少 Windows x64 安装包或对应的 .sig'
[[ "$signature" == "$(<"$dist/$name.sig")" ]] || fail 'latest.json 的签名与安装包 .sig 不一致'

scratch="$(mktemp -d)"
trap 'rm -f "$scratch/updater.pub" "$scratch/installer.minisig"; rmdir "$scratch"' EXIT
# Tauri 在原始 Minisign 公钥和签名外各包了一层 Base64。
jq -ej '.plugins.updater.pubkey | strings | select(length > 0)' "$config" |
  base64 --decode >"$scratch/updater.pub" || fail '无法解码 Tauri 更新公钥'
printf '%s' "$signature" |
  base64 --decode >"$scratch/installer.minisig" || fail '无法解码 Tauri 更新签名'
minisign -V -H -q -m "$dist/$name" -p "$scratch/updater.pub" -x "$scratch/installer.minisig" ||
  fail '安装包未通过 Tauri 更新公钥的签名验证'

echo "verified updater feed version=$version asset=$name"
