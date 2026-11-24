import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { resolveAsset, verifyDraftRelease } from '../.github/scripts/release-assets.mjs';

export function releaseInventory(product, version) {
  const base = `${product}_${version}`;
  const installers = [];
  const platforms = {};
  for (const [arch, {windows, mac, deb, rpm, appImage}] of Object.entries({
    x86_64: {windows: 'x64', mac: 'x64', deb: 'amd64', rpm: 'x86_64', appImage: 'amd64'},
    aarch64: {windows: 'arm64', mac: 'aarch64', deb: 'arm64', rpm: 'aarch64', appImage: 'aarch64'},
  })) {
    const nsis = `${base}_${windows}-setup.exe`;
    const app = `${base}_${mac}.app.tar.gz`;
    const image = `${base}_${appImage}.AppImage`;
    const debian = `${base}_${deb}.deb`;
    const redhat = `${product}-${version}-1.${rpm}.rpm`;
    installers.push(`${base}_${mac}.dmg`);
    Object.assign(platforms, {
      [`windows-${arch}`]: nsis,
      [`windows-${arch}-nsis`]: nsis,
      [`darwin-${arch}`]: app,
      [`darwin-${arch}-app`]: app,
      [`linux-${arch}`]: image,
      [`linux-${arch}-appimage`]: image,
      [`linux-${arch}-deb`]: debian,
      [`linux-${arch}-rpm`]: redhat,
    });
  }
  const signed = [...new Set(Object.values(platforms))];
  const names = ['latest.json', ...installers, ...signed.flatMap(name => [name, name + '.sig'])];
  return { names, signed, platforms };
}

export async function verifyAssets({directory, release, repo, tag, config}) {
  const version = tag.slice(1);
  const {names, signed, platforms} = releaseInventory(config.productName, version);
  const checksums = await verifyDraftRelease({
    directory, release, repo, tag, version, requiredAssets: names, platforms: Object.keys(platforms),
  });
  const expected = new Set([...names, 'SHA256SUMS']);
  for (const name of [...fs.readdirSync(directory), ...release.assets.map(asset => asset.name)]) {
    if (!expected.has(name)) throw new Error(`Unexpected release asset: ${name}`);
  }

  const feed = JSON.parse(fs.readFileSync(path.join(directory, 'latest.json'), 'utf8'));
  for (const [key, entry] of Object.entries(feed.platforms)) {
    const name = resolveAsset(entry.url, {repo, tag, assets: release.assets});
    if (name !== platforms[key]) throw new Error(`${key} must point to ${platforms[key] ?? 'a supported target'}`);
  }

  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'rclone-updater-'));
  try {
    const pubkey = path.join(temp, 'updater.pub');
    const signature = path.join(temp, 'updater.minisig');
    fs.writeFileSync(pubkey, Buffer.from(config.plugins.updater.pubkey, 'base64'));
    for (const name of signed) {
      const asset = path.join(directory, name);
      fs.writeFileSync(signature, Buffer.from(fs.readFileSync(asset + '.sig', 'utf8'), 'base64'));
      const result = spawnSync('minisign', ['-V', '-H', '-q', '-m', asset, '-p', pubkey, '-x', signature], {encoding: 'utf8'});
      if (result.error || result.status !== 0) {
        throw new Error(`Updater signature verification failed: ${name}. ${result.error ?? result.stderr.trim()}`);
      }
    }
  } finally {
    fs.rmSync(temp, {recursive: true, force: true});
  }
  return checksums;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [, , directory, releaseJson, repo, tag, configJson] = process.argv;
  const json = file => JSON.parse(fs.readFileSync(file, 'utf8'));
  try {
    const sums = await verifyAssets({directory, release: json(releaseJson), repo, tag, config: json(configJson)});
    fs.writeFileSync(path.join(directory, 'SHA256SUMS'), sums);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
