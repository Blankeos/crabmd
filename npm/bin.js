#!/usr/bin/env node

const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");
const { install } = require("./install");

const binaryName = "crabmd";
const binaryPath = path.join(__dirname, "bin", binaryName);

async function ensureBinary() {
  if (fs.existsSync(binaryPath)) {
    return;
  }

  console.error("crabmd binary not found. Attempting download...");

  try {
    await install();
  } catch (error) {
    process.exit(1);
  }

  if (!fs.existsSync(binaryPath)) {
    console.error("❌ crabmd binary still missing after download.");
    process.exit(1);
  }
}

async function run() {
  await ensureBinary();

  const args = process.argv.slice(2);
  const wait = args.includes("-w") || args.includes("--wait");

  if (!wait) {
    const child = spawn(binaryPath, args, {
      stdio: "ignore",
      detached: true,
    });
    child.on("error", (err) => {
      console.error("❌ Failed to start crabmd:", err.message);
      process.exit(1);
    });
    child.unref();
    return;
  }

  const child = spawn(binaryPath, args, { stdio: "inherit" });

  child.on("error", (err) => {
    console.error("❌ Failed to start crabmd:", err.message);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    process.exit(signal ? 1 : code || 0);
  });
}

run();
