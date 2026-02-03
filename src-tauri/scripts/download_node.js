const fs = require('fs');
const path = require('path');
const https = require('https');
const { execSync } = require('child_process');

const binDir = path.join(__dirname, '..', 'bin');
if (!fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });

const NODE_VERSION = 'v24.13.0';

const targets = [
    {
        name: 'node-aarch64-apple-darwin',
        url: `https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-darwin-arm64.tar.gz`,
        type: 'tar.gz',
        binPath: `node-${NODE_VERSION}-darwin-arm64/bin/node`
    },
    {
        name: 'node-x86_64-apple-darwin',
        url: `https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-darwin-x64.tar.gz`,
        type: 'tar.gz',
        binPath: `node-${NODE_VERSION}-darwin-x64/bin/node`
    },
    {
        name: 'node-x86_64-pc-windows-msvc.exe',
        url: `https://nodejs.org/dist/${NODE_VERSION}/win-x64/node.exe`,
        type: 'binary'
    },
    {
        name: 'node-x86_64-unknown-linux-gnu',
        url: `https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-linux-x64.tar.xz`,
        type: 'tar.xz',
        binPath: `node-${NODE_VERSION}-linux-x64/bin/node`
    }
];

async function downloadFile(url, dest) {
    return new Promise((resolve, reject) => {
        const file = fs.createWriteStream(dest);
        https.get(url, (response) => {
            if (response.statusCode !== 200) {
                reject(new Error(`Failed to download ${url}: ${response.statusCode}`));
                return;
            }
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

function extract(file, dest, type) {
    console.log(`Extracting ${file}...`);
    if (type === 'tar.gz') {
        execSync(`tar -xzf "${file}" -C "${dest}"`);
    } else if (type === 'tar.xz') {
        execSync(`tar -xJf "${file}" -C "${dest}"`);
    }
}

async function setup() {
    for (const target of targets) {
        const targetPath = path.join(binDir, target.name);
        if (fs.existsSync(targetPath)) {
            console.log(`[skip] ${target.name} already exists.`);
            continue;
        }

        console.log(`[setting up] ${target.name}...`);
        const tempFile = path.join(binDir, `temp-${target.name}${target.type === 'binary' ? '' : '.' + target.type}`);

        try {
            await downloadFile(target.url, tempFile);

            if (target.type === 'binary') {
                fs.renameSync(tempFile, targetPath);
            } else {
                extract(tempFile, binDir, target.type);
                const extractedBin = path.join(binDir, target.binPath);
                fs.renameSync(extractedBin, targetPath);
                // Cleanup
                const rootDir = target.binPath.split('/')[0];
                fs.rmSync(path.join(binDir, rootDir), { recursive: true, force: true });
                fs.unlinkSync(tempFile);
            }

            if (!targetPath.endsWith('.exe')) {
                fs.chmodSync(targetPath, 0o755);
            }
            console.log(`[done] ${target.name}`);
        } catch (err) {
            console.error(`[error] Failed to setup ${target.name}:`, err.message);
        }
    }
}

setup();
