"use strict";

/**
 * HONE System Notifications — Native OS notifications
 * Shin Devlin
 *
 * Uses whatever the OS provides. No Electron, no dependencies.
 * Linux: notify-send (libnotify)
 * Mac: osascript (AppleScript)
 * Windows: powershell toast
 */

var { execSync } = require("child_process");
var platform = process.platform;

/**
 * Send a desktop notification.
 * @param {string} title
 * @param {string} body
 * @param {string} [urgency] — "low", "normal", "critical"
 */
function notify(title, body, urgency) {
  urgency = urgency || "normal";

  try {
    if (platform === "linux") {
      execSync('notify-send ' +
        '--urgency=' + urgency + ' ' +
        '--app-name=HONE ' +
        JSON.stringify(title) + ' ' +
        JSON.stringify(body),
        { timeout: 5000, stdio: 'ignore' }
      );
    } else if (platform === "darwin") {
      execSync('osascript -e ' +
        JSON.stringify('display notification ' + JSON.stringify(body) +
        ' with title ' + JSON.stringify(title) +
        ' sound name "default"'),
        { timeout: 5000, stdio: 'ignore' }
      );
    } else if (platform === "win32") {
      var ps = '[Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime] | Out-Null; ' +
        '$t=[Windows.UI.Notifications.ToastNotification]::New([Windows.Data.Xml.Dom.XmlDocument]::new()); ' +
        '$xml="<toast><visual><binding template=\\"ToastText02\\"><text id=\\"1\\">' + title.replace(/"/g, '') + '</text>' +
        '<text id=\\"2\\">' + body.replace(/"/g, '') + '</text></binding></visual></toast>"; ' +
        '$t.Content.LoadXml($xml); ' +
        '[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier("HONE").Show($t)';
      execSync('powershell -command "' + ps.replace(/"/g, '\\"') + '"',
        { timeout: 10000, stdio: 'ignore', windowsHide: true }
      );
    }
  } catch (e) {
    // Notification failed — not fatal, just log
    // Common on headless servers, SSH sessions, etc.
  }
}

/**
 * Notify about an available update.
 */
function notifyUpdate(currentVersion, newVersion, commitCount) {
  notify(
    "HONE Update Available",
    "v" + currentVersion + " → v" + newVersion + " (" + commitCount + " commits). Update to keep earning.",
    "normal"
  );
}

/**
 * Notify about mining status.
 */
function notifyMining(epoch, reward) {
  notify(
    "HONE Mining",
    "Epoch " + epoch + " — earned " + reward.toFixed(4) + " HONE",
    "low"
  );
}

/**
 * Notify about throttle state change.
 */
function notifyThrottle(mode) {
  notify(
    "HONE Miner",
    mode === "full" ? "Full speed — PC is idle" : "Reduced mode — PC in use",
    "low"
  );
}

module.exports = {
  notify: notify,
  notifyUpdate: notifyUpdate,
  notifyMining: notifyMining,
  notifyThrottle: notifyThrottle
};
