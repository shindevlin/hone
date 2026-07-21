"use strict";
const Project = require('../models/Project');

/**
 * API key authentication middleware for project integrations.
 * Accepts Bearer tokens prefixed with hone_ and resolves to a project.
 * Falls through to next auth method if token is not a hone_ key.
 */
async function authenticateApiKey(req, res, next) {
  const authHeader = req.headers.authorization;
  if (!authHeader || !authHeader.startsWith('Bearer ')) {
    return res.status(401).json({
      error: { message: 'Missing API key. Provide a Bearer token.', type: 'authentication_error', code: 'missing_key' }
    });
  }

  const token = authHeader.slice(7).trim();

  // Internal relay key — used by bot endpoints to call inference API
  const RELAY_KEY = process.env.HONE_RELAY_API_KEY;
  if (RELAY_KEY && token === RELAY_KEY) {
    req.isRelay = true;
    return next();
  }

  // If it's a hone_ project key, resolve the project
  if (token.startsWith('hone_')) {
    const project = await Project.findOne({ apiKey: token, isActive: true });
    if (!project) {
      return res.status(401).json({
        error: { message: 'Invalid or deactivated API key.', type: 'authentication_error', code: 'invalid_api_key' }
      });
    }
    if (!project.verified) {
      return res.status(403).json({
        error: { message: 'Project not verified. Add a .hone file to your repo and call POST /api/projects/verify.', type: 'authorization_error', code: 'unverified_project' }
      });
    }
    req.project = project;
    req.apiKey = token;
    return next();
  }

  // Not a project key — pass the token through for other auth methods
  req.apiKey = token;
  next();
}

module.exports = { authenticateApiKey };
