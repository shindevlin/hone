"use strict";

const path = require("path");
const { spawn } = require("child_process");
const { app, BrowserWindow, shell } = require("electron");

const REPO_ROOT = path.resolve(__dirname, "..");
const WEBSITE_PORT = process.env.HONE_DESKTOP_PORT || "4243";
const START_WEBSITE = process.env.HONE_DESKTOP_WEBSITE !== "0";

let websiteProcess = null;

function startWebsite() {
  if (!START_WEBSITE) return;

  websiteProcess = spawn(process.execPath, [path.join(REPO_ROOT, "website", "serve.js")], {
    cwd: REPO_ROOT,
    env: Object.assign({}, process.env, {
      PORT: WEBSITE_PORT,
      HONE_API_PORT: process.env.HONE_API_PORT || "3000",
    }),
    stdio: process.env.HONE_DESKTOP_DEV ? "inherit" : "ignore",
  });

  websiteProcess.on("exit", () => {
    websiteProcess = null;
  });
}

function createWindow() {
  const win = new BrowserWindow({
    width: 420,
    height: 860,
    minWidth: 360,
    minHeight: 640,
    backgroundColor: "#0a0e17",
    title: "HONE",
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });

  win.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url);
    return { action: "deny" };
  });

  win.loadURL(`http://127.0.0.1:${WEBSITE_PORT}/app.html`);
}

app.whenReady().then(() => {
  startWebsite();
  setTimeout(createWindow, START_WEBSITE ? 500 : 0);

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

app.on("before-quit", () => {
  if (websiteProcess) websiteProcess.kill("SIGTERM");
});
