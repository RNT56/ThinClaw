const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const os = require('os');

async function main() {
    console.log(`[openclaw-wrapper] Starting wrapper...`);
    console.log(`[openclaw-wrapper] Current directory: ${process.cwd()}`);
    console.log(`[openclaw-wrapper] Script directory: ${__dirname}`);
    console.log(`[openclaw-wrapper] OS Homedir: ${os.homedir()}`);

    // Forward arguments
    const args = process.argv.slice(2);

    // Attempt to locate openclaw
    // We expect to be in a directory that has node_modules/openclaw
    // Or node_modules is in a parent directory

    let openclawDir = '';
    const possibleDirs = [
        path.join(__dirname, 'node_modules', 'openclaw'),
        path.join(__dirname, '..', 'node_modules', 'openclaw'),
        path.resolve(__dirname, 'node_modules', 'openclaw')
    ];

    for (const dir of possibleDirs) {
        if (fs.existsSync(dir)) {
            openclawDir = dir;
            break;
        }
    }

    if (!openclawDir) {
        // Fallback to require.resolve
        try {
            const pkgPath = require.resolve('openclaw/package.json');
            openclawDir = path.dirname(pkgPath);
        } catch (e) {
            console.error('[openclaw-wrapper] Could not find openclaw via file search or require.resolve');
            // Try moltbot as legacy fallback?
            try {
                const pkgPath = require.resolve('moltbot/package.json');
                openclawDir = path.dirname(pkgPath);
                console.log('[openclaw-wrapper] Falling back to moltbot package');
            } catch (e2) {
                process.exit(1);
            }
        }
    }

    console.log(`[openclaw-wrapper] Found openclaw directory: ${openclawDir}`);

    // Try to peek at identity
    try {
        // Path might differ in openclaw vs moltbot
        const identityLoader = require(path.join(openclawDir, 'dist', 'infra', 'device-identity.js'));
        if (identityLoader && identityLoader.loadOrCreateDeviceIdentity) {
            const identity = identityLoader.loadOrCreateDeviceIdentity();
            console.log(`[openclaw-wrapper] Device ID: ${identity.deviceId}`);
        }
    } catch (e) {
        console.log(`[openclaw-wrapper] Could not peek at identity: ${e.message}`);
    }

    const pkg = JSON.parse(fs.readFileSync(path.join(openclawDir, 'package.json'), 'utf8'));

    let binRelPath = '';
    if (pkg.bin) {
        if (typeof pkg.bin === 'string') {
            binRelPath = pkg.bin;
        } else if (pkg.bin.openclaw) {
            binRelPath = pkg.bin.openclaw;
        } else if (pkg.bin.moltbot) {
            binRelPath = pkg.bin.moltbot;
        }
    }

    if (!binRelPath) {
        // Fallback to common names
        const fallbacks = ['openclaw.mjs', 'openclaw.js', 'bin/openclaw.mjs', 'dist/index.js'];
        for (const f of fallbacks) {
            if (fs.existsSync(path.join(openclawDir, f))) {
                binRelPath = f;
                break;
            }
        }
    }

    if (!binRelPath) {
        console.error('[openclaw-wrapper] Could not determine binary path for openclaw');
        process.exit(1);
    }

    const binPath = path.join(openclawDir, binRelPath);
    console.log(`[openclaw-wrapper] Starting openclaw from: ${binPath}`);

    // We use process.execPath (the bundled node) to run the .mjs or .js file
    const child = spawn(process.execPath, [binPath, ...args], {
        stdio: 'inherit',
        env: {
            ...process.env
        }
    });

    child.on('exit', (code) => {
        console.log(`[openclaw-wrapper] openclaw exited with code ${code}`);
        process.exit(code || 0);
    });

    child.on('error', (err) => {
        console.error('[openclaw-wrapper] Failed to spawn openclaw:', err);
        process.exit(1);
    });
}

main();
