#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const repoRoot = path.resolve(__dirname, '..');
const publicVault = path.join(repoRoot, 'obsidian-public');
const outPath = path.join(repoRoot, 'website', 'public', 'gource-data.json');

const textExtensions = new Set([
  '.md', '.html', '.js', '.mjs', '.cjs', '.json', '.yaml', '.yml',
  '.toml', '.sh', '.css', '.xml', '.txt', '.java', '.kt'
]);

const excludeNames = new Set(['.git', '.obsidian', 'node_modules', 'dist', 'build', 'coverage', '.DS_Store']);

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function sanitizeText(contents) {
  return String(contents)
    .replaceAll('/home/ubuntclaw/repos/hone/', '')
    .replaceAll('/home/ubuntclaw/', '')
    .replace(/CLAUDE_HANDOFF[^\n]*/gi, '')
    .replace(/SECURITY_AUDIT[^\n]*/gi, '')
    .replace(/PRIVATE_AUTH_(IMPLEMENTATION_PLAN|FUTURE|STACK)[^\n]*/gi, '')
    .replace(/SELF_HEAL_PRD[^\n]*/gi, '');
}

function buildGitTimestampMap() {
  const map = new Map();
  let currentTs = null;
  const stdout = execFileSync('git', ['log', '--format=%ct', '--name-only', '--', '.'], {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
  });
  for (const line of stdout.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    if (/^\d+$/.test(trimmed)) {
      currentTs = Number(trimmed);
      continue;
    }
    if (currentTs && !map.has(trimmed)) {
      map.set(trimmed, currentTs);
    }
  }
  return map;
}

function readText(filePath) {
  return sanitizeText(fs.readFileSync(filePath, 'utf8'));
}

function extractSummary(filePath, text) {
  const lines = text.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (filePath.endsWith('.md')) {
    for (const line of lines) {
      if (!line.startsWith('#')) return line;
    }
  }
  return lines[0] || '';
}

function guessSourcePath(rel, text) {
  if (rel.startsWith('Source/')) return path.join('src', rel.slice('Source/'.length)).replace(/\\/g, '/');
  if (rel.startsWith('Tests/')) return path.join('tests', rel.slice('Tests/'.length)).replace(/\\/g, '/');
  if (rel.startsWith('Website/')) return path.join('website', rel.slice('Website/'.length)).replace(/\\/g, '/');
  if (rel.startsWith('Android/')) return path.join('android/app/src/main/java/network/hone/app', rel.slice('Android/'.length)).replace(/\\/g, '/');
  if (rel.startsWith('Docs/code-wiki/')) return path.join('docs/code-wiki', rel.slice('Docs/code-wiki/'.length)).replace(/\\/g, '/');
  if (rel.startsWith('Docs/')) return path.join('docs', rel.slice('Docs/'.length)).replace(/\\/g, '/');
  if (rel.startsWith('Code Wiki/')) {
    const match = text.match(/File-based community:\s*(.+)/i);
    if (match) return match[1].trim().replace(/\\/g, '/').replace(/^\/+/, '');
  }
  return null;
}

function walk(dir, relParts = []) {
  const entries = [];
  const items = fs.readdirSync(dir, { withFileTypes: true })
    .filter((entry) => !excludeNames.has(entry.name) && entry.name !== 'index.html')
    .sort((a, b) => a.name.localeCompare(b.name));

  for (const entry of items) {
    const abs = path.join(dir, entry.name);
    const rel = path.posix.join(...relParts, entry.name);
    if (entry.isDirectory()) {
      entries.push({
        kind: 'folder',
        rel,
        name: entry.name,
        children: walk(abs, [...relParts, entry.name]),
      });
      continue;
    }
    const ext = path.extname(entry.name).toLowerCase();
    if (!textExtensions.has(ext)) continue;
    const raw = fs.readFileSync(abs);
    const text = textExtensions.has(ext) ? sanitizeText(raw.toString('utf8')) : '';
    entries.push({
      kind: 'file',
      rel,
      name: entry.name.replace(new RegExp(`${ext.replace('.', '\\.')}$`, 'i'), ''),
      ext,
      size: raw.length,
      summary: extractSummary(rel, text),
      sourcePath: guessSourcePath(rel, text),
      mtime: fs.statSync(abs).mtimeMs / 1000,
      links: Array.from(text.matchAll(/\[\[([^\]]+)\]\]/g)).map((m) => m[1].split('|')[0].split('#')[0].trim()).filter(Boolean),
    });
  }
  return entries;
}

function flattenTree(nodes, out = []) {
  for (const node of nodes) {
    out.push(node);
    if (node.children) flattenTree(node.children, out);
  }
  return out;
}

function resolveTimestamps(nodes, gitMap) {
  const flat = flattenTree(nodes);
  for (const node of flat) {
    if (node.kind === 'file') {
      const gitTs = node.sourcePath ? gitMap.get(node.sourcePath) : null;
      node.ts = gitTs || node.mtime || Math.floor(Date.now() / 1000);
    }
  }
  function bubble(node) {
    if (node.kind === 'file') return node.ts;
    const childTs = node.children.map(bubble).filter(Boolean);
    node.ts = childTs.length ? Math.max(...childTs) : Math.floor(Date.now() / 1000);
    return node.ts;
  }
  for (const node of nodes) bubble(node);
}

function assignIds(nodes, parentId = '') {
  for (const node of nodes) {
    node.id = parentId ? `${parentId}/${node.rel}` : node.rel;
    if (node.children) assignIds(node.children, node.id);
  }
}

function buildStats(nodes) {
  const flat = flattenTree(nodes);
  return {
    folders: flat.filter((n) => n.kind === 'folder').length,
    files: flat.filter((n) => n.kind === 'file').length,
    minTs: Math.min(...flat.map((n) => n.ts)),
    maxTs: Math.max(...flat.map((n) => n.ts)),
  };
}

function main() {
  const gitMap = buildGitTimestampMap();
  const tree = walk(publicVault, []);
  assignIds(tree);
  resolveTimestamps(tree, gitMap);
  const stats = buildStats(tree);
  ensureDir(path.dirname(outPath));
  fs.writeFileSync(outPath, JSON.stringify({ title: 'HONE Vault Atlas', root: tree, stats }, null, 2) + '\n', 'utf8');
  console.log(`Wrote ${outPath}`);
}

main();
