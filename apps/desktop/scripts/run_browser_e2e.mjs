#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { constants, createWriteStream } from "node:fs";
import {
  access,
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  rename,
  rm,
} from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { Browser, BrowserPlatform, install } from "@puppeteer/browsers";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const wdioCli = join(desktopRoot, "node_modules", "@wdio", "cli", "bin", "wdio.js");
const maxChromeDriverArchiveBytes = 32 * 1024 * 1024;

async function isExecutable(path) {
  try {
    await access(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function installedChromeVersion() {
  const candidates = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
    "/Applications/Google Chrome Dev.app/Contents/MacOS/Google Chrome Dev",
  ];
  for (const chrome of candidates) {
    try {
      const output = execFileSync(chrome, ["--version"], { encoding: "utf8" });
      const version = output.match(/\d+\.\d+\.\d+\.\d+/)?.[0];
      if (version) return version;
    } catch {
      // Try the next installed Chrome channel.
    }
  }
  throw new Error("Google Chrome is required for the desktop browser E2E suite");
}

async function prepareMacChromedriver() {
  if (process.env.CHROMEDRIVER_PATH) return process.env.CHROMEDRIVER_PATH;

  const version = installedChromeVersion();
  const arm64 = process.arch === "arm64";
  const platform = arm64 ? BrowserPlatform.MAC_ARM : BrowserPlatform.MAC;
  const platformLabel = arm64 ? "mac-arm64" : "mac-x64";
  const installRoot = join(
    homedir(),
    ".cache",
    "thinclaw-webdriver",
    "manual",
    version,
  );
  const driver = join(installRoot, `chromedriver-${platformLabel}`, "chromedriver");
  if (await isExecutable(driver)) return driver;

  const archive = await install({
    browser: Browser.CHROMEDRIVER,
    buildId: version,
    cacheDir: join(homedir(), ".cache", "thinclaw-webdriver", "downloads"),
    platform,
    unpack: false,
  });
  const scratch = await mkdtemp(join(tmpdir(), "thinclaw-chromedriver-"));
  let archiveFile;
  try {
    archiveFile = await open(archive, constants.O_RDONLY | constants.O_NOFOLLOW);
    const archiveStat = await archiveFile.stat();
    if (!archiveStat.isFile() || archiveStat.size === 0 || archiveStat.size > maxChromeDriverArchiveBytes) {
      throw new Error(`ChromeDriver archive has an invalid size (${archiveStat.size} bytes)`);
    }
    const signature = Buffer.alloc(2);
    await archiveFile.read(signature, 0, signature.length, 0);
    if (signature[0] !== 0x50 || signature[1] !== 0x4b) {
      throw new Error("ChromeDriver download is not a ZIP archive");
    }
    const validatedArchive = join(scratch, "chromedriver.zip");
    await pipeline(
      archiveFile.createReadStream({ autoClose: false, start: 0 }),
      createWriteStream(validatedArchive, { flags: "wx", mode: 0o600 }),
    );
    await archiveFile.close();
    archiveFile = undefined;

    const expectedEntry = `chromedriver-${platformLabel}/chromedriver`;
    const unzip = spawnSync("unzip", ["-q", validatedArchive, expectedEntry, "-d", scratch], {
      stdio: "inherit",
    });
    if (unzip.status !== 0) {
      throw new Error(`unzip failed with exit code ${unzip.status ?? "unknown"}`);
    }
    const extractedDriver = join(scratch, expectedEntry);
    const extractedStat = await lstat(extractedDriver);
    if (!extractedStat.isFile() || extractedStat.isSymbolicLink()) {
      throw new Error("ChromeDriver archive did not contain the expected regular file");
    }
    await mkdir(dirname(driver), { recursive: true });
    const stagedDriver = `${driver}.tmp-${process.pid}`;
    await copyFile(extractedDriver, stagedDriver);
    await chmod(stagedDriver, 0o755);
    await rename(stagedDriver, driver);
  } finally {
    await archiveFile?.close();
    await rm(scratch, { recursive: true, force: true });
  }
  if (!(await isExecutable(driver))) {
    throw new Error(`ChromeDriver was not installed at ${driver}`);
  }
  return driver;
}

const env = { ...process.env };
if (process.platform === "darwin") {
  env.CHROMEDRIVER_PATH = await prepareMacChromedriver();
}

const result = spawnSync(process.execPath, [wdioCli, "run", "./wdio.browser.conf.ts"], {
  cwd: desktopRoot,
  env,
  stdio: "inherit",
});
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
