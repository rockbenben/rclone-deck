import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';
import { hashFile } from '../.github/scripts/release-assets.mjs';
import { releaseInventory, verifyAssets } from './verify-release-assets.mjs';

const product = 'rclone-deck';
const version = '9.8.7';
const tag = 'v' + version;
const repo = 'example/rclone-deck';
const {names, signed, platforms} = releaseInventory(product, version);

async function fixture(t, edit = () => {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'rclone-release-test-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const directory = path.join(root, 'dist');
  fs.mkdirSync(directory);
  const pubkey = path.join(root, 'public');
  const key = path.join(root, 'private');
  const rawSignature = path.join(root, 'signature');
  execFileSync('minisign', ['-G', '-W', '-p', pubkey, '-s', key]);
  for (const name of names) {
    if (name !== 'latest.json' && !name.endsWith('.sig')) {
      fs.writeFileSync(path.join(directory, name), name);
    }
  }
  for (const name of signed) {
    const file = path.join(directory, name);
    execFileSync('minisign', ['-S', '-s', key, '-x', rawSignature, '-m', file]);
    fs.writeFileSync(file + '.sig', fs.readFileSync(rawSignature).toString('base64'));
  }
  const release = {tag_name: tag, draft: true, prerelease: false,
    assets: names.map((name, index) => ({name, id: index + 1}))};
  const feed = {version, platforms: Object.fromEntries(Object.entries(platforms).map(([platform, name], i) => {
    const asset = release.assets.find(row => row.name === name);
    const url = i % 2 ? `https://api.github.com/repos/${repo}/releases/assets/${asset.id}`
      : `https://github.com/${repo}/releases/download/${tag}/${name}`;
    return [platform, {url, signature: fs.readFileSync(path.join(directory, name + '.sig'), 'utf8')}];
  }))};
  const config = {productName: product, plugins: {updater: {pubkey: fs.readFileSync(pubkey).toString('base64')}}};
  edit({directory, release, feed, config});
  fs.writeFileSync(path.join(directory, 'latest.json'), JSON.stringify(feed));
  for (const asset of release.assets) {
    asset.digest ??= 'sha256:' + await hashFile(path.join(directory, asset.name));
  }
  return {directory, release, repo, tag, config};
}

test('checks every native installer, all six bases and the format-specific downloader entries', async t => {
  assert.equal(names.length, 23);
  assert.equal(Object.keys(platforms).length, 16);
  assert.equal(platforms['windows-x86_64'], 'rclone-deck_9.8.7_x64-setup.exe');
  assert.equal(platforms['linux-aarch64-deb'], 'rclone-deck_9.8.7_arm64.deb');
  assert.equal(platforms['darwin-x86_64'], 'rclone-deck_9.8.7_x64.app.tar.gz');
  const input = await fixture(t, ({directory, release}) => {
    fs.writeFileSync(path.join(directory, 'SHA256SUMS'), 'previous run');
    release.assets.push({id: 100, name: 'SHA256SUMS'});
  });
  const checksums = await verifyAssets(input);
  assert.equal(checksums.trim().split('\n').length, names.length);
  assert.doesNotMatch(checksums, /SHA256SUMS/);
});

test('rejects a draft missing the Intel DMG', async t => {
  const input = await fixture(t, ({release}) => {
    release.assets = release.assets.filter(a => a.name !== 'rclone-deck_9.8.7_x64.dmg');
  });
  await assert.rejects(verifyAssets(input), /Missing required asset.*_x64\.dmg/);
});

test('rejects a feed missing an ARM64 Linux carrier', async t => {
  const input = await fixture(t, ({feed}) => { delete feed.platforms['linux-aarch64-appimage']; });
  await assert.rejects(verifyAssets(input), /Missing updater target: linux-aarch64-appimage/);
});

test('rejects wrong architectures and GitHub asset IDs from other drafts', async t => {
  const input = await fixture(t, ({feed}) => {
    feed.platforms['windows-aarch64'] = feed.platforms['windows-x86_64'];
  });
  await assert.rejects(verifyAssets(input), /windows-aarch64 architecture mismatch/);
  const foreign = await fixture(t, ({feed}) => {
    feed.platforms['darwin-x86_64'].url = `https://api.github.com/repos/${repo}/releases/assets/7000`;
  });
  await assert.rejects(verifyAssets(foreign), /Asset ID does not belong to this draft/);
});

test('rejects mismatched transfer digests and cryptographically invalid payloads', async t => {
  const input = await fixture(t, ({release}) => {
    release.assets[0].digest = 'sha256:' + '0'.repeat(64);
  });
  await assert.rejects(verifyAssets(input), /Release digest mismatch/);
  const tampered = await fixture(t, ({directory}) => {
    fs.appendFileSync(path.join(directory, platforms['linux-aarch64-rpm']), 'modified');
  });
  await assert.rejects(verifyAssets(tampered), /Updater signature verification failed.*aarch64\.rpm/);
});

test('rejects a feed with a wrong tag or signature text', async t => {
  const wrongTag = await fixture(t, ({feed}) => {
    feed.platforms['windows-x86_64'].url = feed.platforms['windows-x86_64'].url.replace(tag, 'v9.8.6');
  });
  await assert.rejects(verifyAssets(wrongTag), /Unexpected asset repository or tag/);
  const wrongSig = await fixture(t, ({feed}) => { feed.platforms['darwin-aarch64'].signature = 'wrong'; });
  await assert.rejects(verifyAssets(wrongSig), /darwin-aarch64 signature mismatch/);
});
