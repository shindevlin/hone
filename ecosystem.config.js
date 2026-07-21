/**
 * PM2 is no longer used for HONE node management.
 *
 * The node is managed by systemd:
 *   sudo systemctl start hone-node
 *   sudo systemctl stop hone-node
 *   sudo systemctl status hone-node
 *   journalctl -u hone-node -f
 *
 * Service file: /etc/systemd/system/hone-node.service
 * Deploy template: deploy/systemd/hone-node.service
 */

// This file is intentionally empty — kept only for historical reference.
module.exports = { apps: [] };
