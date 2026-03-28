const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
    testDir: '.',
    timeout: 60000,
    retries: 0,
    use: {
        baseURL: 'http://localhost:8091',
        headless: true,
    },
    webServer: {
        command: 'cd examples && python3 server.py 8091',
        port: 8091,
        reuseExistingServer: true,
        cwd: '../..',
    },
    projects: [
        { name: 'chromium', use: { browserName: 'chromium' } },
    ],
});
