const fs = require('fs');
const path = require('path');
const https = require('https');
const { execSync } = require('child_process');

const binDir = path.join(__dirname, '..', 'bin');
const dryRun = process.argv.includes('--dry-run');
const help = process.argv.includes('--help') || process.argv.includes('-h');

if (help) {
    console.log(`Usage: node backend/scripts/download_ai_binaries.cjs [--dry-run]

Downloads AI sidecar binaries into backend/bin for the current desktop app.
Currently this helper targets macOS ARM64 release artifacts.`);
    process.exit(0);
}

if (!dryRun && !fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });

// Configuration for binaries
const LLAMA_VERSION = 'b4618';
const WHISPER_VERSION = 'v1.7.4';
const SD_VERSION = 'master-7010bb4'; // Using a stable commit for SD

const binaries = [
    {
        name: 'llama-server-aarch64-apple-darwin',
        url: `https://github.com/ggerganov/llama.cpp/releases/download/${LLAMA_VERSION}/llama-${LLAMA_VERSION}-bin-macos-arm64.zip`,
        type: 'zip',
        targetBin: 'llama-server',
        patterns: ['*.dylib', '*.metal']
    },
    {
        name: 'whisper-server-aarch64-apple-darwin',
        url: `https://github.com/ggerganov/whisper.cpp/releases/download/${WHISPER_VERSION}/whisper-${WHISPER_VERSION}-bin-macos-arm64.zip`,
        type: 'zip',
        targetBin: 'whisper-server',
        patterns: ['*.dylib', 'whisper-cli']
    },
    {
        name: 'sd-aarch64-apple-darwin',
        url: `https://github.com/leejet/stable-diffusion.cpp/releases/download/${SD_VERSION.split('-')[1]}/sd-${SD_VERSION}-bin-macos-arm64.zip`,
        // Note: SD release names can vary. Falls back to manual if URL fails.
        type: 'zip',
        targetBin: 'sd',
        patterns: ['*.dylib', '*.metal']
    }
];

async function downloadFile(url, dest) {
    return new Promise((resolve, reject) => {
        const options = {
            headers: { 'User-Agent': 'node.js' }
        };
        https.get(url, options, (response) => {
            if (response.statusCode === 301 || response.statusCode === 302) {
                downloadFile(response.headers.location, dest).then(resolve).catch(reject);
                return;
            }
            if (response.statusCode !== 200) {
                reject(new Error(`Failed to download ${url}: ${response.statusCode}`));
                return;
            }
            const file = fs.createWriteStream(dest);
            response.pipe(file);
            file.on('finish', () => {
                file.close();
                resolve();
            });
        }).on('error', (err) => {
            fs.unlink(dest, () => reject(err));
        });
    });
}

function extractZip(file, dest) {
    console.log(`Extracting ${file}...`);
    try {
        execSync(`unzip -o "${file}" -d "${dest}"`);
    } catch (e) {
        console.error(`Failed to extract ${file}: ${e.message}`);
    }
}

async function setup() {
    console.log("Setting up AI sidecar binaries for macOS ARM64...");

    if (dryRun) {
        for (const bin of binaries) {
            console.log(`[dry-run] would download ${bin.name} from ${bin.url}`);
        }
        return;
    }

    for (const bin of binaries) {
        const targetPath = path.join(binDir, bin.name);
        if (fs.existsSync(targetPath)) {
            console.log(`[skip] ${bin.name} already exists.`);
            continue;
        }

        console.log(`[setting up] ${bin.name}...`);
        const tempFile = path.join(binDir, `temp-${bin.name}.zip`);

        try {
            await downloadFile(bin.url, tempFile);
            const extractDir = path.join(binDir, `extract-${bin.name}`);
            if (!fs.existsSync(extractDir)) fs.mkdirSync(extractDir);

            extractZip(tempFile, extractDir);

            // 1. Copy main binary
            const binFiles = execSync(`find "${extractDir}" -type f -name "${bin.targetBin}"`).toString().trim().split('\n');
            if (binFiles.length > 0 && binFiles[0]) {
                fs.copyFileSync(binFiles[0], targetPath);
                fs.chmodSync(targetPath, 0o755);
                console.log(`[done binary] ${bin.name}`);
            }

            // 2. Copy dependencies (*.dylib, *.metal)
            if (bin.patterns) {
                for (const pattern of bin.patterns) {
                    try {
                        const deps = execSync(`find "${extractDir}" -type f -name "${pattern}"`).toString().trim().split('\n');
                        for (const dep of deps) {
                            if (dep) {
                                const depName = path.basename(dep);
                                // Special case for whisper-cli etc to match tauri.conf.json
                                let finalName = depName;
                                if (depName === 'whisper-cli' || depName === 'main' || depName === 'whisper') {
                                    finalName = 'whisper-aarch64-apple-darwin';
                                } else if (depName.includes('cli')) {
                                    finalName = depName + '-aarch64-apple-darwin';
                                }

                                const destPath = path.join(binDir, finalName);
                                fs.copyFileSync(dep, destPath);
                                if (!finalName.endsWith('.dylib') && !finalName.endsWith('.metal')) {
                                    fs.chmodSync(destPath, 0o755);
                                }
                                console.log(`[done asset] ${finalName}`);
                            }
                        }
                    } catch (e) { /* pattern not found */ }
                }
            }

            // Cleanup
            fs.unlinkSync(tempFile);
            fs.rmSync(extractDir, { recursive: true, force: true });
        } catch (err) {
            console.warn(`[warning] Failed to setup ${bin.name} automatically:`, err.message);
            console.log(`Please manually download from: ${bin.url}`);
        }
    }
}

setup();
