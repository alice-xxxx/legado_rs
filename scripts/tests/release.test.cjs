const { test, beforeEach, afterEach } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { validate, stage, publish, makeLatest } = require('../../.github/scripts/release.cjs');

const sourceDirectory = process.cwd();
let temporary, env, state, api, outputs;
const fail = status => Object.assign(new Error(`HTTP ${status}`), { status });
const summary = { addHeading() { return this; }, addRaw() { return this; }, addLink() { return this; }, async write() {} };

beforeEach(() => {
  temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'foliver-release-test-'));
  assert.equal(path.dirname(fs.realpathSync(temporary)), fs.realpathSync(os.tmpdir()));
  process.chdir(temporary);
  fs.mkdirSync('release-assets');
  fs.writeFileSync('release-assets/package.exe', 'installer');
  fs.writeFileSync('release-assets/latest.json', JSON.stringify({ version: '0.1.1', platforms: {} }));
  env = { RELEASE_TAG: 'v0.1.1', RELEASE_COMMIT: 'a'.repeat(40), RELEASE_PRERELEASE: 'false', RELEASE_DRAFT: 'false', RELEASE_ID: '7', RUNNER_TEMP: temporary };
  state = { tag: null, releases: [], deleted: [], created: [], published: [], assets:
    ['package.exe', 'latest.json'].map((name, index) => ({ id: index + 1, name,
      size: fs.statSync(path.join('release-assets', name)).size, state: 'uploaded',
      digest: `sha256:${crypto.createHash('sha256').update(fs.readFileSync(path.join('release-assets', name))).digest('hex')}`,
    })),
  };
  outputs = {};
  const repos = {
    listReleases() {}, listReleaseAssets() {},
    async createRelease(args) {
      state.created.push(args);
      const release = { ...args, id: 7, body: 'Generated notes', html_url: 'https://example.invalid/release' };
      state.releases.push(release);
      return { data: release };
    },
    async getRelease() { return { data: state.releases.find(item => item.id === 7) }; },
    async deleteReleaseAsset({ asset_id }) { state.deleted.push(asset_id); state.assets = state.assets.filter(item => item.id !== asset_id); },
    async updateRelease(args) { state.published.push(args); return { data: args }; },
  };
  api = {
    context: { repo: { owner: 'example', repo: 'reader' } },
    core: { setOutput(name, value) { outputs[name] = value; }, summary },
    github: {
      rest: { repos, git: {
        async getRef() { if (!state.tag) throw fail(404); return { data: { object: { type: state.annotated ? 'tag' : 'commit', sha: state.tag } } }; },
        async getTag() { return { data: { object: { type: 'commit', sha: env.RELEASE_COMMIT } } }; },
        async createRef(args) { state.tag = args.sha; },
      } },
      async paginate(method) { return method === repos.listReleases ? state.releases : state.assets; },
    },
  };
});

afterEach(() => {
  process.chdir(sourceDirectory);
  fs.rmSync(temporary, { recursive: true });
});

test('new source is valid without an existing Git tag; validation never writes', async () => {
  assert.equal(await validate(api, env), undefined);
  assert.equal(state.tag, null);
  assert.equal(state.created.length, 0);
});

test('moved tags and published versions are rejected before builds', async () => {
  state.tag = 'b'.repeat(40);
  await assert.rejects(validate(api, env), /Tags are never moved/);
  state.tag = env.RELEASE_COMMIT;
  state.releases = [{ tag_name: env.RELEASE_TAG, draft: false }];
  await assert.rejects(validate(api, env), /already published/);
});

test('annotated tags are resolved to their source commit', async () => {
  state.tag = 'tag-object'; state.annotated = true;
  await validate(api, env);
});

test('missing tag is created at the pinned commit and release starts as a draft', async () => {
  await stage(api, env);
  assert.equal(state.tag, env.RELEASE_COMMIT);
  assert.equal(state.created[0].target_commitish, env.RELEASE_COMMIT);
  assert.equal(state.created[0].draft, true);
  assert.equal(outputs.id, 7);
  assert.equal(JSON.parse(fs.readFileSync(path.join(temporary, 'foliver-release-notes.json'))).notes, 'Generated notes');
  assert.equal(state.published.length, 0);
});

test('draft retries preserve edited release notes and remove obsolete attachments', async () => {
  state.tag = env.RELEASE_COMMIT;
  state.releases = [{ id: 7, tag_name: env.RELEASE_TAG, draft: true, body: 'Edited notes' }];
  state.assets.push({ id: 99, name: 'obsolete.exe' });
  await stage(api, env);
  assert.equal(state.created.length, 0);
  assert.deepEqual(state.deleted, [99]);
  assert.equal(JSON.parse(fs.readFileSync(path.join(temporary, 'foliver-release-notes.json'))).notes, 'Edited notes');
});

test('a concurrent tag creation is only accepted at the same source', async () => {
  api.github.rest.git.createRef = async () => { state.tag = env.RELEASE_COMMIT; throw fail(422); };
  await stage(api, env);
  assert.equal(state.created.length, 1);
});

test('a concurrent different tag never produces a release', async () => {
  api.github.rest.git.createRef = async () => { state.tag = 'b'.repeat(40); throw fail(422); };
  await assert.rejects(stage(api, env));
  assert.equal(state.created.length, 0);
});

test('complete stable release is published and becomes latest', async () => {
  await stage(api, env);
  await publish(api, env);
  assert.equal(state.published[0].draft, false);
  assert.equal(state.published[0].make_latest, 'true');
});

test('optional draft mode verifies artifacts without publishing', async () => {
  env.RELEASE_DRAFT = 'true';
  await stage(api, env);
  await publish(api, env);
  assert.equal(state.published.length, 0);
});

test('prereleases and historical stable releases never replace latest', async () => {
  env.RELEASE_PRERELEASE = 'true'; env.RELEASE_TAG = 'v1.0.0-rc.1';
  await stage(api, env);
  await publish(api, env);
  assert.equal(state.published[0].prerelease, true);
  assert.equal(state.published[0].make_latest, 'false');
  const releases = [{ tag_name: 'v2.0.0', draft: false }];
  assert.equal(makeLatest('v1.9.9', false, releases), false);
  assert.equal(makeLatest('v2.0.1', false, releases), true);
  assert.equal(makeLatest('v2.0.0', false, releases), false);
  assert.equal(makeLatest('v10.0.0', false, releases), true);
});

test('missing and corrupt uploaded packages never publish', async () => {
  await stage(api, env);
  state.assets[0].size++;
  await assert.rejects(publish(api, env), /incomplete/);
  state.assets[0].size--;
  state.assets[0].digest = 'sha256:wrong';
  await assert.rejects(publish(api, env), /checksum mismatch/);
  state.assets.pop();
  await assert.rejects(publish(api, env), /asset count/);
  assert.equal(state.published.length, 0);
});

test('uploaded update manifest digest is verified without an extra checksum file', async () => {
  await stage(api, env);
  state.assets[1].digest = 'sha256:wrong';
  await assert.rejects(publish(api, env), /checksum mismatch: latest.json/);
  assert.equal(state.published.length, 0);
});

test('release or tag changes during upload never publish', async () => {
  await stage(api, env);
  state.tag = 'b'.repeat(40);
  await assert.rejects(publish(api, env), /tag moved/);
  state.tag = env.RELEASE_COMMIT;
  state.releases[0].draft = false;
  await assert.rejects(publish(api, env), /state changed/);
  assert.equal(state.published.length, 0);
});
