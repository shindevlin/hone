/**
 * PM2 Ecosystem Configuration for BTCPC
 *
 * Start everything:   pm2 start ecosystem.config.js
 * Stop everything:    pm2 stop all
 * View logs:          pm2 logs
 * Monitor:            pm2 monit
 *
 * The miner runs continuously and auto-restarts on crash.
 * The updater runs every 15 minutes via cron, checks GitHub
 * for new commits, pulls, and restarts the miner if needed.
 */

module.exports = {
  apps: [
    {
      name: 'btcpc-mine',
      script: 'bin/btcpc-mine',
      cwd: __dirname,
      restart_delay: 10000,       // 10s between restarts
      max_restarts: 50,           // don't loop forever on fatal errors
      autorestart: true,          // always restart on exit
      watch: false,               // don't file-watch (updater handles restarts)
      env: {
        NODE_ENV: 'production'
      }
    },
    {
      name: 'btcpc-update',
      script: 'bin/btcpc-update',
      cwd: __dirname,
      cron_restart: '*/15 * * * *',  // run every 15 minutes
      autorestart: false,             // cron handles scheduling, don't loop
      watch: false,
      env: {
        NODE_ENV: 'production'
      }
    }
  ]
};
