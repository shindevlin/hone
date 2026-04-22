"use strict";

/**
 * Inference memory routes — per-account memory CRUD + Obsidian-style graph.
 * Shin Devlin
 *
 * GET  /api/memory                    — list account memories (paginated, filterable)
 * GET  /api/memory/graph              — Obsidian-style graph (nodes + edges)
 * GET  /api/memory/projects           — list projects
 * POST /api/memory/projects           — create a project
 * GET  /api/memory/search             — semantic search by query
 * GET  /api/memory/:id                — get memory metadata + content
 * DELETE /api/memory/:id             — delete a memory
 */

const express = require("express");
const router = express.Router();
const fs = require("fs");
const path = require("path");
const { authenticateToken } = require("../middlewares/auth");
const { sanitizeString } = require("../middlewares/validate");
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");
const memoryService = require("../services/memoryService");

const BLOB_DIR = process.env.BTCPC_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");

function _readBlob(cid) {
  if (!/^[a-f0-9]{64}$/.test(cid)) return null;
  try { return fs.readFileSync(path.join(BLOB_DIR, cid), "utf8"); } catch (_) { return null; }
}

// ── GET /api/memory ───────────────────────────────────────────────────────────
router.get("/", authenticateToken, (req, res) => {
  const account = req.user.username;
  const projectId = req.query.project || null;
  const tag = req.query.tag || null;
  const page = Math.max(1, parseInt(req.query.page) || 1);
  const limit = Math.min(100, Math.max(1, parseInt(req.query.limit) || 20));

  let memories = stateStore.getUserMemories(account);
  if (projectId) memories = memories.filter(m => m.project_id === projectId);
  if (tag) memories = memories.filter(m => m.tags.includes(tag.toLowerCase()));

  const total = memories.length;
  const paged = memories.slice((page - 1) * limit, page * limit);

  res.json({ memories: paged, total, page, limit });
});

// ── GET /api/memory/graph ─────────────────────────────────────────────────────
router.get("/graph", authenticateToken, (req, res) => {
  const account = req.user.username;
  const projectId = req.query.project || null;
  let graph = stateStore.getMemoryGraph(account);
  if (projectId) {
    const keepNodes = new Set();
    keepNodes.add("project:" + projectId);
    graph.nodes.filter(n => n.type === "memory" && n.data.project_id === projectId).forEach(n => keepNodes.add(n.id));
    graph.nodes = graph.nodes.filter(n => keepNodes.has(n.id));
    graph.edges = graph.edges.filter(e => keepNodes.has(e.source) && keepNodes.has(e.target));
  }
  res.json(graph);
});

// ── GET /api/memory/projects ──────────────────────────────────────────────────
router.get("/projects", authenticateToken, (req, res) => {
  const account = req.user.username;
  const projects = stateStore.getUserProjects(account);
  // Annotate with memory count
  const memories = stateStore.getUserMemories(account);
  const countByProject = {};
  memories.forEach(m => {
    if (m.project_id) countByProject[m.project_id] = (countByProject[m.project_id] || 0) + 1;
  });
  const annotated = projects.map(p => ({ ...p, memory_count: countByProject[p.project_id] || 0 }));
  res.json({ projects: annotated });
});

// ── POST /api/memory/projects ─────────────────────────────────────────────────
router.post("/projects", authenticateToken, async (req, res) => {
  const account = req.user.username;
  const name = sanitizeString(req.body.name, 100);
  if (!name) return res.status(400).json({ error: "name required" });

  const existing = stateStore.getUserProjects(account);
  const conflict = existing.find(p => p.name.toLowerCase() === name.toLowerCase());
  if (conflict) return res.status(409).json({ error: "Project with this name already exists", project_id: conflict.project_id });

  const projectId = name.toLowerCase().replace(/[^a-z0-9_-]/g, "-").replace(/-+/g, "-").slice(0, 64) + "-" + Date.now().toString(36);
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordMemoryProjectCreate(account, projectId, name, epoch);

  res.status(201).json({ project_id: projectId, name });
});

// ── GET /api/memory/search ────────────────────────────────────────────────────
router.get("/search", authenticateToken, async (req, res) => {
  const account = req.user.username;
  const query = sanitizeString(req.query.q, 500);
  if (!query) return res.status(400).json({ error: "q required" });
  const limit = Math.min(50, Math.max(1, parseInt(req.query.limit) || 10));
  const results = await memoryService.searchMemories(account, query, limit);
  res.json({ results, query });
});

// ── GET /api/memory/:id ───────────────────────────────────────────────────────
router.get("/:id", authenticateToken, (req, res) => {
  const account = req.user.username;
  const mem = stateStore.getMemory(account, req.params.id);
  if (!mem) return res.status(404).json({ error: "Memory not found" });

  const raw = _readBlob(mem.cid);
  let content = null;
  if (raw) {
    try { content = JSON.parse(raw); } catch (_) { content = { raw }; }
  }

  res.json({ ...mem, content });
});

// ── DELETE /api/memory/:id ────────────────────────────────────────────────────
router.delete("/:id", authenticateToken, async (req, res) => {
  const account = req.user.username;
  const mem = stateStore.getMemory(account, req.params.id);
  if (!mem) return res.status(404).json({ error: "Memory not found" });

  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordMemoryDelete(account, mem.memory_id, mem.project_id, epoch);

  // Delete local blob copy (best-effort; storage nodes may still hold it)
  try { fs.unlinkSync(path.join(BLOB_DIR, mem.cid)); } catch (_) {}

  res.json({ deleted: true, memory_id: mem.memory_id });
});

module.exports = router;
