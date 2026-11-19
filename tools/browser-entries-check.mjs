import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';

const ext = ['png', 'jpg', 'JPEG', 'GIF', 'webp', 'BMP', 'avif'];
const isImage = (name) => ext.some((suffix) => name.toLowerCase().endsWith('.' + suffix.toLowerCase()));
function fixture(images) {
  const entries = [];
  for (let i = 0; i < images; i++) {
    entries.push({ name: `gallery/photo-${i}.${ext[i % ext.length]}`, isDir: false, size: i });
    if (i % 20 === 0) {
      entries.push({ name: `gallery/folder-${i}.jpg`, isDir: true, size: 0 });
      entries.push({ name: `gallery/notes-${i}.txt`, isDir: false, size: i });
    }
  }
  return entries;
}

// Reference used by BrowserPane before phase G.
const referenceExts = ext.map((suffix) => '.' + suffix.toLowerCase());
function baseline(entries, showGrid) {
  const grid = showGrid ? entries.filter((entry) => {
    if (entry.isDir) return false;
    const name = entry.name.toLowerCase();
    return referenceExts.some((suffix) => name.endsWith(suffix));
  }) : [];
  return { grid, rest: grid.length > 0 ? entries.filter((entry) => !grid.includes(entry)) : entries };
}

const before = process.argv.includes('--baseline');
const classify = before ? baseline : (await import('../src/format.ts')).classifyBrowserEntries;
for (const count of [6000, 24000]) {
  const entries = fixture(count);
  const expected = baseline(entries, true);
  const actual = classify(entries, true);
  assert.equal(actual.grid.length, count);
  assert(actual.grid.every((entry, i) => entry === expected.grid[i] && !entry.isDir && isImage(entry.name)));
  assert.equal(actual.rest.length, expected.rest.length);
  assert(actual.rest.every((entry, i) => entry === expected.rest[i]));
  assert.equal(classify(entries, false).rest, entries);
  assert.deepEqual(classify([], true), { grid: [], rest: [] });

  for (let warm = 0; warm < 5; warm++) classify(entries, true);
  const iterations = before ? 3 : 50;
  const samples = [];
  for (let i = 0; i < 9; i++) {
    const start = performance.now();
    for (let sample = 0; sample < iterations; sample++) classify(entries, true);
    samples.push((performance.now() - start) / iterations);
  }
  samples.sort((a, b) => a - b);
  console.log(`BENCH-G gallery ${before ? 'baseline' : 'current'} images=${count} rows=${entries.length} median_ms=${samples[4].toFixed(3)}`);
}
