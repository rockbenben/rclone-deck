// Shared by both GitHub release workflows. Only publish a verified draft by ID.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { versionFromTag } from './release-version.mjs';

const { TAG, GH_TOKEN, GITHUB_REPOSITORY, SOURCE_COMMIT, RELEASE_ID } = process.env;
const endpoint = `${process.env.GITHUB_API_URL}/repos/${GITHUB_REPOSITORY}/releases`;

async function api(resource, { optional = false, body } = {}) {
  const response = await fetch(endpoint + resource, {
    method: body ? 'PATCH' : 'GET',
    headers: { Authorization: `Bearer ${GH_TOKEN}`, 'Content-Type': 'application/json' },
    body: body && JSON.stringify(body),
  });
  if (optional && response.status === 404) return null;
  if (!response.ok) throw new Error(`Release API ${resource}: HTTP ${response.status}`);
  return response.json();
}

function verifyRemoteTag() {
  const ref = 'refs/tags/' + TAG;
  const output = execFileSync('git', ['ls-remote', '--exit-code', 'origin', ref, ref + '^{}'], { encoding: 'utf8' });
  const refs = new Map(output.trim().split(/\r?\n/).map(line => line.split('\t').reverse()));
  if ((refs.get(ref + '^{}') ?? refs.get(ref)) !== SOURCE_COMMIT) throw new Error('Tag moved after validation');
}

function shouldAdvanceLatest(currentTag) {
  if (TAG.includes('-')) return false;
  if (!currentTag) return true;
  const current = versionFromTag(currentTag, 'electron').split('.').map(BigInt);
  const candidate = versionFromTag(TAG, 'electron').split('.').map(BigInt);
  const difference = candidate.findIndex((part, i) => part !== current[i]);
  return difference !== -1 && candidate[difference] > current[difference];
}

async function verifyUpload(directory, assets) {
  const files = fs.readdirSync(directory);
  if (files.length !== assets.length) throw new Error('Draft asset inventory differs from this build');
  for (const name of files) {
    const hash = createHash('sha256');
    for await (const chunk of fs.createReadStream(path.join(directory, name))) hash.update(chunk);
    if (assets.find(a => a.name === name)?.digest !== 'sha256:' + hash.digest('hex')) {
      throw new Error(`Draft asset differs from this build: ${name}`);
    }
  }
}

async function main() {
  versionFromTag(TAG, 'tauri');
  verifyRemoteTag();
  const [command, directory] = process.argv.slice(2);
  if (command === 'guard') {
    const release = await api('/tags/' + TAG, { optional: true });
    if (release && !release.draft) throw new Error('Release already public; use a new version');
    return;
  }
  if (command !== 'publish' || !/^\d+$/.test(RELEASE_ID ?? '')) throw new Error('Expected a draft release ID');
  const draft = await api('/' + RELEASE_ID);
  if (!draft.draft || draft.tag_name !== TAG || draft.prerelease !== TAG.includes('-')) {
    throw new Error('Expected the unpublished draft for this tag and channel');
  }
  if (directory) await verifyUpload(directory, draft.assets);
  const previous = await api('/latest', { optional: true });
  const advance = shouldAdvanceLatest(previous?.tag_name);
  verifyRemoteTag();
  await api('/' + RELEASE_ID, { body: { draft: false, make_latest: String(advance) } });
  const latest = await api('/latest', { optional: true });
  if ((latest?.tag_name ?? null) !== (advance ? TAG : previous?.tag_name ?? null)) {
    throw new Error('Unexpected Latest release after publication');
  }
}

main().catch(error => { console.error(error.message); process.exitCode = 1; });
