const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const os = require('os');

async function main() {
    console.log(`[moltbot-wrapper] Starting wrapper...`);
    console.log(`[moltbot-wrapper] Current directory: ${process.cwd()}`);
    console.log(`[moltbot-wrapper] Script directory: ${__dirname}`);
    console.log(`[moltbot-wrapper] OS Homedir: ${os.homedir()}`);

    // Forward arguments
    const args = process.argv.slice(2);

    // Attempt to locate moltbot
    // We expect to be in a directory that has node_modules/moltbot
    // Or node_modules is in a parent directory

    let moltbotDir = '';
    const possibleDirs = [
        path.join(__dirname, 'node_modules', 'moltbot'),
        path.join(__dirname, '..', 'node_modules', 'moltbot'),
        path.resolve(__dirname, 'node_modules', 'moltbot')
    ];

    for (const dir of possibleDirs) {
        if (fs.existsSync(dir)) {
            moltbotDir = dir;
            break;
        }
    }

    if (!moltbotDir) {
        // Fallback to require.resolve
        try {
            const pkgPath = require.resolve('moltbot/package.json');
            moltbotDir = path.dirname(pkgPath);
        } catch (e) {
            console.error('[moltbot-wrapper] Could not find moltbot via file search or require.resolve');
            process.exit(1);
        }
    }

    console.log(`[moltbot-wrapper] Found moltbot directory: ${moltbotDir}`);

    // Try to peek at identity
    try {
        const identityLoader = require(path.join(moltbotDir, 'dist', 'infra', 'device-identity.js'));
        if (identityLoader && identityLoader.loadOrCreateDeviceIdentity) {
            const identity = identityLoader.loadOrCreateDeviceIdentity();
            console.log(`[moltbot-wrapper] Moltbot Device ID: ${identity.deviceId}`);
        }
    } catch (e) {
        console.log(`[moltbot-wrapper] Could not peek at identity: ${e.message}`);
    }

    const pkg = JSON.parse(fs.readFileSync(path.join(moltbotDir, 'package.json'), 'utf8'));

    let binRelPath = '';
    if (pkg.bin) {
        if (typeof pkg.bin === 'string') {
            binRelPath = pkg.bin;
        } else if (pkg.bin.moltbot) {
            binRelPath = pkg.bin.moltbot;
        }
    }

    if (!binRelPath) {
        // Fallback to common names
        const fallbacks = ['moltbot.mjs', 'moltbot.js', 'dist/index.js'];
        for (const f of fallbacks) {
            if (fs.existsSync(path.join(moltbotDir, f))) {
                binRelPath = f;
                break;
            }
        }
    }

    if (!binRelPath) {
        console.error('[moltbot-wrapper] Could not determine binary path for moltbot');
        process.exit(1);
    }

    const binPath = path.join(moltbotDir, binRelPath);
    console.log(`[moltbot-wrapper] Starting moltbot from: ${binPath}`);

    // We use process.execPath (the bundled node) to run the .mjs or .js file
    const child = spawn(process.execPath, [binPath, ...args], {
        stdio: 'inherit',
        env: {
            ...process.env
        }
    });

    child.on('exit', (code) => {
        console.log(`[moltbot-wrapper] moltbot exited with code ${code}`);
        process.exit(code || 0);
    });

    child.on('error', (err) => {
        console.error('[moltbot-wrapper] Failed to spawn moltbot:', err);
        process.exit(1);
    });
}

main();
