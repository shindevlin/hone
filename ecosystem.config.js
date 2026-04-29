/**
 * PM2 is no longer used for BTCPC node management.
 *
 * The node is managed by systemd:
 *   sudo systemctl start btcpc-node
 *   sudo systemctl stop btcpc-node
 *   sudo systemctl status btcpc-node
 *   journalctl -u btcpc-node -f
 *
 * Service file: /etc/systemd/system/btcpc-node.service
 * Deploy template: deploy/systemd/btcpc-node.service
 */

// This file is intentionally empty — kept only for historical reference.
module.exports = { apps: [] };
