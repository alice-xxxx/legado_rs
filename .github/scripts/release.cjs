const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

async function tagCommit(github, repo, tag) {
  let object;
  try { ({ data: { object } } = await github.rest.git.getRef({ ...repo, ref: `tags/${tag}` })); }
  catch (error) { if (error.status === 404) return null; throw error; }
  while (object.type === 'tag') {
    ({ data: { object } } = await github.rest.git.getTag({ ...repo, tag_sha: object.sha }));
  }
  if (object.type !== 'commit') throw new Error(`${tag} does not point to a source commit`);
  return object.sha;
}

async function validate({ github, context }, env = process.env) {
  const tag = env.RELEASE_TAG;
  const commit = await tagCommit(github, context.repo, tag);
  if (commit && commit !== env.RELEASE_COMMIT) {
    throw new Error(`${tag} points to ${commit}, but the selected source is ${env.RELEASE_COMMIT}. Tags are never moved; select that source or bump the app version.`);
  }
  const releases = await github.paginate(github.rest.repos.listReleases, { ...context.repo, per_page: 100 });
  const release = releases.find(item => item.tag_name === tag);
  if (release && !release.draft) throw new Error(`${tag} is already published. Bump the app version before making another release.`);
  return release;
}

async function stage({ github, context, core }, env = process.env) {
  let release = await validate({ github, context }, env);
  const tag = env.RELEASE_TAG;
  if (!await tagCommit(github, context.repo, tag)) {
    try {
      await github.rest.git.createRef({ ...context.repo, ref: `refs/tags/${tag}`, sha: env.RELEASE_COMMIT });
    } catch (error) {
      // A tag created concurrently is only reusable if it selects this exact source.
      if (error.status !== 422 || await tagCommit(github, context.repo, tag) !== env.RELEASE_COMMIT) throw error;
    }
  }
  if (!release) {
    ({ data: release } = await github.rest.repos.createRelease({
      ...context.repo,
      tag_name: tag,
      target_commitish: env.RELEASE_COMMIT,
      name: `Foliver ${tag}`,
      draft: true,
      prerelease: env.RELEASE_PRERELEASE === 'true',
      make_latest: 'false',
      generate_release_notes: true,
      body: '安装包按平台和架构分别提供。Android APK 已签名；Windows/macOS 安装包未配置系统证书签名，iOS IPA 需自行签名后安装。\n\n',
    }));
  }
  const names = new Set(fs.readdirSync('release-assets'));
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...context.repo, release_id: release.id, per_page: 100 });
  for (const asset of assets) {
    if (!names.has(asset.name)) await github.rest.repos.deleteReleaseAsset({ ...context.repo, asset_id: asset.id });
  }
  fs.writeFileSync(path.join(env.RUNNER_TEMP, 'foliver-release-notes.json'), JSON.stringify({
    notes: release.body || '', pub_date: new Date().toISOString(),
  }));
  core.setOutput('id', release.id);
}

function makeLatest(tag, prerelease, releases) {
  if (prerelease) return false;
  const stable = value => /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(value)?.slice(1).map(BigInt);
  const candidate = stable(tag);
  if (!candidate) return false;
  return releases.filter(item => !item.draft && !item.prerelease).every(item => {
    const version = stable(item.tag_name);
    if (!version) return true;
    for (let i = 0; i < 3; i++) if (candidate[i] !== version[i]) return candidate[i] > version[i];
    return false;
  });
}

async function publish({ github, context, core }, env = process.env) {
  const release_id = Number(env.RELEASE_ID);
  const { data: release } = await github.rest.repos.getRelease({ ...context.repo, release_id });
  if (!release.draft || release.tag_name !== env.RELEASE_TAG) throw new Error('Release state changed during upload; refusing to publish');
  if (await tagCommit(github, context.repo, env.RELEASE_TAG) !== env.RELEASE_COMMIT) throw new Error('Release tag moved during build');
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...context.repo, release_id, per_page: 100 });
  const expected = fs.readdirSync('release-assets');
  if (assets.length !== expected.length) throw new Error('Release asset count does not match the complete package set');
  for (const name of expected) {
    const asset = assets.find(item => item.name === name);
    const file = path.join('release-assets', name);
    if (!asset || asset.state !== 'uploaded' || asset.size !== fs.statSync(file).size) {
      throw new Error(`Missing or incomplete release asset: ${name}`);
    }
    if (asset.digest) {
      const hash = crypto.createHash('sha256');
      for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
      if (asset.digest !== `sha256:${hash.digest('hex')}`) throw new Error(`Uploaded asset checksum mismatch: ${name}`);
    }
  }
  const draft = env.RELEASE_DRAFT === 'true';
  const prerelease = env.RELEASE_PRERELEASE === 'true';
  if (!draft) {
    const releases = await github.paginate(github.rest.repos.listReleases, { ...context.repo, per_page: 100 });
    await github.rest.repos.updateRelease({ ...context.repo, release_id, draft: false, prerelease,
      make_latest: makeLatest(env.RELEASE_TAG, prerelease, releases) ? 'true' : 'false' });
  }
  await core.summary.addHeading(`Foliver ${env.RELEASE_TAG}`)
    .addRaw(`\n${draft ? '草稿已就绪' : prerelease ? '预发布已发布' : '版本已发布'}；${expected.length} 个附件已校验。\n\n源码：\`${env.RELEASE_COMMIT}\`\n\n`)
    .addLink('查看 Release', release.html_url).write();
}

module.exports = { validate, stage, publish, makeLatest };
