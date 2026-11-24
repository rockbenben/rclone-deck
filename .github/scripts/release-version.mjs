// Copy to .github/scripts. Runs before npm ci, so use only Node built-ins.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const number = '(?:0|[1-9][0-9]*)';
const identifier = `(?:${number}|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)`;
const releaseTag = new RegExp(`^v(${number}\\.${number}\\.${number})(?:-(${identifier}(?:\\.${identifier})*))?$`);

export function versionFromTag(tag, stack) {
  const match = releaseTag.exec(tag);
  if (!['tauri', 'electron'].includes(stack) || !match || match[0] !== tag ||
      (stack === 'electron' && match[2])) {
    throw new Error(`Invalid ${stack} release tag: ${tag} (expected vX.Y.Z; only Tauri supports prereleases)`);
  }
  return tag.slice(1);
}

function checkWindowsInstaller(version, base, windows) {
  // Platform configuration uses JSON Merge Patch. A null removes an inherited property.
  const merge = (original, patch) => {
    if (!patch || typeof patch !== 'object' || Array.isArray(patch)) return patch;
    const result = { ...original };
    for (const [key, value] of Object.entries(patch)) {
      if (value === null) delete result[key];
      else result[key] = merge(result[key], value);
    }
    return result;
  };
  const { bundle = {} } = merge(base, windows);
  const targets = [bundle.targets ?? 'all'].flat();
  if (!targets.some(target => ['all', 'msi'].includes(target))) return;
  const wixVersion = bundle.windows?.wix?.version ?? version.replace('-', '.');
  if (!/^\d+\.\d+\.\d+(?:\.\d+)?$/.test(wixVersion) ||
      wixVersion.split('.').some((n, i) => Number(n) > [255, 255, 65535, 65535][i])) {
    throw new Error(`Invalid MSI version ${wixVersion}. Configure a numeric WiX version or select NSIS for Windows.`);
  }
}

function prepareTauri(version) {
  const dir = 'src-tauri';
  const configurations = new Map();
  for (const platform of ['', '.windows', '.macos', '.linux']) {
    const file = path.join(dir, `tauri${platform}.conf.json`);
    if (fs.existsSync(file.replace(/json$/, 'json5')) || fs.existsSync(path.join(dir, `Tauri${platform}.toml`))) {
      throw new Error(`Adapt release-version.mjs to the project's JSON5/TOML configuration in ${dir}`);
    }
    if (!platform || fs.existsSync(file)) configurations.set(platform, JSON.parse(fs.readFileSync(file, 'utf8')));
  }
  checkWindowsInstaller(version, configurations.get(''), configurations.get('.windows') ?? {});
  for (const [platform, config] of configurations) {
    if (platform && !Object.hasOwn(config, 'version')) continue;
    if (config.version === '../package.json' || config.version === version) continue;
    config.version = version;
    fs.writeFileSync(path.join(dir, `tauri${platform}.conf.json`), JSON.stringify(config, null, 2) + '\n');
  }
}

function main() {
  const [command, stack] = process.argv.slice(2);
  const { TAG, APP_VERSION, GITHUB_OUTPUT, GITHUB_SHA, GITHUB_REF_TYPE } = process.env;
  if (command === 'resolve') {
    if (GITHUB_REF_TYPE !== 'tag') throw new Error('Dispatch on the target tag ref');
    const version = versionFromTag(TAG, stack);
    const sha = ref => execFileSync('git', ['rev-parse', '--verify', ref], { encoding: 'utf8' }).trim();
    const commit = sha('HEAD');
    if (commit !== sha(`${GITHUB_SHA}^{commit}`)) throw new Error('Checkout differs from event commit');
    fs.appendFileSync(GITHUB_OUTPUT, `version=${version}\ncommit=${commit}\n`);
  } else if (command === 'apply') {
    versionFromTag(`v${APP_VERSION}`, stack);
    if (stack === 'tauri') prepareTauri(APP_VERSION);
    const args = ['version', APP_VERSION, '--no-git-tag-version', '--ignore-scripts', '--allow-same-version'];
    if (process.platform === 'win32') execFileSync('cmd.exe', ['/d', '/c', 'npm.cmd', ...args], { stdio: 'inherit' });
    else execFileSync('npm', args, { stdio: 'inherit' });
  } else throw new Error('Expected resolve or apply and the application stack');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
