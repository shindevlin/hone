"use strict";
const Project = require('../models/Project');

/**
 * API key authentication middleware for project integrations.
 * Accepts Bearer tokens prefixed with btcpc_ and resolves to a project.
 * Falls through to next auth method if token is not a btcpc_ key.
 */
async function authenticateApiKey(req, res, next) {
  const authHeader = req.headers.authorization;
  if (!authHeader || !authHeader.startsWith('Bearer ')) {
    return res.status(401).json({
      error: { message: 'Missing API key. Provide a Bearer token.', type: 'authentication_error', code: 'missing_key' }
    });
  }

  const token = authHeader.slice(7).trim();

  // If it's a btcpc_ project key, resolve the project
  if (token.startsWith('btcpc_')) {
    const project = await Project.findOne({ apiKey: token, isActive: true });
    if (!project) {
      return res.status(401).json({
        error: { message: 'Invalid or deactivated API key.', type: 'authentication_error', code: 'invalid_api_key' }
      });
    }
    if (!project.verified) {
      return res.status(403).json({
        error: { message: 'Project not verified. Add a .btcpc file to your repo and call POST /api/projects/verify.', type: 'authorization_error', code: 'unverified_project' }
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
