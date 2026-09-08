#!/usr/bin/env node

const { spawn } = require("child_process");
const fs = require("fs");
const { install, binaryPath, ensureDesktopApp } = require("./install");

async function ensureBinary() {
  const bin = binaryPath();
  if (fs.existsSync(bin)) {
    return bin;
  }

  // Bun skips postinstall unless the package is trusted, so first `crabmd`
  // downloads here. npm still prefers postinstall when it runs.
  console.error("crabmd binary not found. Attempting download...");

  try {
    await install();
  } catch (error) {
    process.exit(1);
  }

  if (!fs.existsSync(bin)) {
    console.error("❌ crabmd binary still missing after download.");
    process.exit(1);
  }
  return bin;
}

async function run() {
  const bin = await ensureBinary();
  ensureDesktopApp(bin);

  // Rust detaches the GUI unless `-w`. Always inherit stdio so `--help`
  // and `--install-desktop` actually print.
  const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });

  child.on("error", (err) => {
    console.error("❌ Failed to start crabmd:", err.message);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    process.exit(signal ? 1 : code || 0);
  });
}

run();
