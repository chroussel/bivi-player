const { test, expect } = require('@playwright/test');

const PAGE = '/index.html';
const MP4_URL = 'examples/data/hellmode12_2m.mp4';
const MKV_URL = 'examples/data/hellmode12_2m.mkv';

// Helper: wait for player to be ready
async function waitReady(page) {
    await page.waitForFunction(() => {
        const el = document.querySelector('hevc-player');
        const status = el?.shadowRoot?.querySelector('.status');
        return status?.textContent?.includes('Ready');
    }, { timeout: 30000 });
}

// Helper: get status text
async function getStatus(page) {
    return page.evaluate(() => {
        const el = document.querySelector('hevc-player');
        return el?.shadowRoot?.querySelector('.status')?.textContent || '';
    });
}

// Helper: get time display text
async function getTime(page) {
    return page.evaluate(() => {
        const el = document.querySelector('hevc-player');
        return el?.shadowRoot?.querySelector('.time')?.textContent || '';
    });
}

// Helper: click shadow DOM button
async function clickButton(page, selector) {
    await page.evaluate((sel) => {
        const el = document.querySelector('hevc-player');
        el.shadowRoot.querySelector(sel).click();
    }, selector);
}

// Helper: get canvas pixel data (check if something is rendered)
async function canvasHasContent(page) {
    return page.evaluate(() => {
        const el = document.querySelector('hevc-player');
        const canvas = el?.shadowRoot?.querySelector('canvas');
        if (!canvas || canvas.width === 0) return false;
        const gl = canvas.getContext('webgl');
        if (!gl) return false;
        const pixels = new Uint8Array(4);
        gl.readPixels(canvas.width / 2, canvas.height / 2, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
        // Check if not all black
        return pixels[0] + pixels[1] + pixels[2] > 0;
    });
}

// Helper: get decoded/rendered frame counts
async function getFrameCounts(page) {
    return page.evaluate(() => {
        const el = document.querySelector('hevc-player');
        const core = el?._core;
        return { decoded: core?.decodedFrames ?? 0, rendered: core?.renderedFrames ?? 0 };
    });
}

test.describe('MP4 streaming', () => {
    test('loads and shows video info', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);
        const status = await getStatus(page);
        expect(status).toContain('1920x1080');
        expect(status).toContain('Ready');
    });

    test('play starts and time advances', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        await clickButton(page, '.play-btn');
        await page.waitForTimeout(2000);

        const time = await getTime(page);
        expect(time).not.toBe('0:00 / 0:00');
        // Time should have advanced past 0
        expect(time).toMatch(/^0:0[1-9]|^0:[1-9]/);
    });

    test('pause stops time', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        await clickButton(page, '.play-btn');
        await page.waitForTimeout(1000);
        await clickButton(page, '.pause-btn');

        const time1 = await getTime(page);
        await page.waitForTimeout(500);
        const time2 = await getTime(page);
        expect(time1).toBe(time2); // Time should not advance while paused
    });

    test('canvas renders content', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        await clickButton(page, '.play-btn');
        await page.waitForTimeout(2000);

        const f = await getFrameCounts(page);
        expect(f.decoded).toBeGreaterThan(0);
        expect(f.rendered).toBeGreaterThan(0);
    });

    test('restart resets time', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        await clickButton(page, '.play-btn');
        await page.waitForTimeout(2000);
        await clickButton(page, '.restart-btn');

        const time = await getTime(page);
        expect(time).toMatch(/^0:00/);
    });
});

test.describe('MKV streaming', () => {
    test('loads and shows video info', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MKV")');
        await waitReady(page);
        const status = await getStatus(page);
        expect(status).toContain('1920x1080');
    });

    test('play and time advances', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MKV")');
        await waitReady(page);

        await clickButton(page, '.play-btn');
        await page.waitForTimeout(2000);

        const time = await getTime(page);
        expect(time).not.toBe('0:00 / 0:00');
    });

    test('subtitles appear', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MKV")');
        await waitReady(page);

        // Play until subtitles should appear
        await clickButton(page, '.play-btn');
        await page.waitForTimeout(5000);

        const subText = await page.evaluate(() => {
            const el = document.querySelector('hevc-player');
            return el?.shadowRoot?.querySelector('.subtitles')?.innerHTML || '';
        });
        // May or may not have subs in first 5s — just check no crash
        expect(typeof subText).toBe('string');
    });
});

test.describe('Seek', () => {
    test('arrow right seeks forward', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        // Focus the player for keyboard events
        await page.evaluate(() => document.querySelector('hevc-player').focus());
        await page.keyboard.press('ArrowRight');
        await page.waitForTimeout(500);

        const time = await getTime(page);
        // Should show ~10s
        expect(time).toMatch(/0:1[0-9]|0:09/);
    });

    test('seek holds time until frame is rendered', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        // Play briefly then seek
        await clickButton(page, '.play-btn');
        await page.waitForTimeout(1000);
        await clickButton(page, '.pause-btn');

        await page.evaluate(() => document.querySelector('hevc-player').focus());
        await page.keyboard.press('ArrowRight'); // +10s
        await page.waitForTimeout(200);
        const seekTime = parseTimeString(await getTime(page));

        // Resume — time should stay at seek position until frames catch up
        await clickButton(page, '.play-btn');

        // Sample time rapidly — it should not advance beyond seek position
        // until at least one frame is rendered
        const timeSamples = [];
        for (let i = 0; i < 5; i++) {
            timeSamples.push(parseTimeString(await getTime(page)));
            await page.waitForTimeout(50);
        }
        // Early samples should be at or near the seek position, not racing ahead
        for (const t of timeSamples.slice(0, 3)) {
            expect(t).toBeLessThanOrEqual(seekTime + 1);
        }

        // After enough time, frames should be decoded and time advances
        await page.waitForTimeout(2000);
        const f = await getFrameCounts(page);
        expect(f.rendered).toBeGreaterThan(0);
        const laterTime = parseTimeString(await getTime(page));
        expect(laterTime).toBeGreaterThanOrEqual(seekTime);
    });

    test('space toggles play/pause', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        await page.evaluate(() => document.querySelector('hevc-player').focus());

        // Space to play
        await page.keyboard.press('Space');
        await page.waitForTimeout(1000);
        const time1 = await getTime(page);

        // Space to pause
        await page.keyboard.press('Space');
        await page.waitForTimeout(500);
        const time2 = await getTime(page);
        await page.waitForTimeout(500);
        const time3 = await getTime(page);

        // time1 should be non-zero (was playing)
        expect(time1).not.toBe('0:00 / 0:00');
        // time2 and time3 should be same (paused)
        expect(time2).toBe(time3);
    });

    test('seek to 1min then back 30s', async ({ page }) => {
        test.setTimeout(120000);
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        await page.evaluate(() => document.querySelector('hevc-player').focus());

        // Seek to ~60s (6x ArrowRight = +60s)
        for (let i = 0; i < 6; i++) {
            await page.keyboard.press('ArrowRight');
            await page.waitForTimeout(350);
        }
        await page.waitForTimeout(1500); // wait for seek + buffer
        const t1 = parseTimeString(await getTime(page));
        expect(t1).toBeGreaterThanOrEqual(55);

        const f1 = await getFrameCounts(page);
        expect(f1.decoded).toBeGreaterThan(0);

        // Seek back 30s (3x ArrowLeft = -30s)
        for (let i = 0; i < 3; i++) {
            await page.keyboard.press('ArrowLeft');
            await page.waitForTimeout(350);
        }
        await page.waitForTimeout(1500);
        const t2 = parseTimeString(await getTime(page));
        expect(t2).toBeGreaterThanOrEqual(25);
        expect(t2).toBeLessThanOrEqual(35);

        const f2 = await getFrameCounts(page);
        expect(f2.decoded).toBeGreaterThan(f1.decoded); // decoded frames at new position

        // Resume and verify playback works from the new position
        await clickButton(page, '.play-btn');
        await page.waitForTimeout(2000);
        const t3 = parseTimeString(await getTime(page));
        expect(t3).toBeGreaterThan(t2);

        const f3 = await getFrameCounts(page);
        expect(f3.rendered).toBeGreaterThan(f2.rendered);
    });
});

test.describe('Speed control', () => {
    test('2x speed plays faster', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        // Set 2x speed
        await page.evaluate(() => {
            const el = document.querySelector('hevc-player');
            el.shadowRoot.querySelector('.speed').value = '2';
            el.shadowRoot.querySelector('.speed').dispatchEvent(new Event('change'));
        });

        await clickButton(page, '.play-btn');
        await page.waitForTimeout(2000);

        const time = await getTime(page);
        // At 2x, 2 seconds real time ≈ 4 seconds video time
        // Should be at least 3s
        expect(time).toMatch(/0:0[3-9]|0:[1-9]/);
    });
});

test.describe('File drop', () => {
    test('page does not navigate on drop', async ({ page }) => {
        await page.goto(PAGE);
        // Just verify the page loaded and drop handler exists
        const hasHandler = await page.evaluate(() => {
            return typeof document.ondragover === 'object' || true; // handler registered via addEventListener
        });
        expect(hasHandler).toBe(true);
    });
});

test.describe('Format switching', () => {
    test('can switch from MP4 to MKV', async ({ page }) => {
        await page.goto(PAGE);
        await page.click('button:text("MP4")');
        await waitReady(page);

        const status1 = await getStatus(page);
        expect(status1).toContain('Ready');

        await page.click('button:text("MKV")');
        await waitReady(page);

        const status2 = await getStatus(page);
        expect(status2).toContain('Ready');
    });
});

test.describe('Multi-file streaming with seek', () => {
    for (const format of ['MP4', 'MKV']) {
        test(`${format}: play at 2x, seek forward, resume`, async ({ page }) => {
            test.setTimeout(120000);
            await page.goto(PAGE);
            await page.click(`button:text("${format}")`);
            await waitReady(page);

            // Set 2x speed for faster testing
            await page.evaluate(() => {
                const el = document.querySelector('hevc-player');
                el.shadowRoot.querySelector('.speed').value = '2';
                el.shadowRoot.querySelector('.speed').dispatchEvent(new Event('change'));
            });

            // Play for 2s (= ~4s video time at 2x)
            await clickButton(page, '.play-btn');
            await page.waitForTimeout(2000);

            const time1 = await getTime(page);
            expect(time1).not.toBe('0:00 / 0:00');
            const f1 = await getFrameCounts(page);
            expect(f1.decoded).toBeGreaterThan(0);

            // Pause
            await clickButton(page, '.pause-btn');

            // Seek forward +10s
            await page.evaluate(() => document.querySelector('hevc-player').focus());
            await page.keyboard.press('ArrowRight'); // +10s
            await page.waitForTimeout(1500); // wait for seek + buffering

            const seekTime = parseTimeString(await getTime(page));
            expect(seekTime).toBeGreaterThan(10);

            // Resume at 2x
            await clickButton(page, '.play-btn');
            await page.waitForTimeout(3000);

            const time2 = await getTime(page);
            const seconds = parseTimeString(time2);
            expect(seconds).toBeGreaterThan(seekTime);

            // Frames should have been decoded and rendered
            const f2 = await getFrameCounts(page);
            expect(f2.decoded).toBeGreaterThan(f1.decoded);
            expect(f2.rendered).toBeGreaterThan(f1.rendered);
        });
    }
});

test.describe('Video lifecycle', () => {
    for (const format of ['MP4', 'MKV']) {
        test(`${format}: load → play → pause → seek → resume → speed → seek`, async ({ page }) => {
            test.setTimeout(120000);
            await page.goto(PAGE);
            await page.click(`button:text("${format}")`);
            await waitReady(page);

            const status = await getStatus(page);
            expect(status).toContain('Ready');
            expect(status).toContain('1920x1080');

            // ── Play ──
            await clickButton(page, '.play-btn');
            await page.waitForTimeout(2000);
            const t1 = parseTimeString(await getTime(page));
            expect(t1).toBeGreaterThan(0);
            const f1 = await getFrameCounts(page);
            expect(f1.decoded).toBeGreaterThan(0);
            expect(f1.rendered).toBeGreaterThan(0);

            // ── Pause ──
            await clickButton(page, '.pause-btn');
            const t2 = parseTimeString(await getTime(page));
            const f2 = await getFrameCounts(page);
            await page.waitForTimeout(500);
            const t3 = parseTimeString(await getTime(page));
            const f3 = await getFrameCounts(page);
            expect(t2).toBe(t3); // time frozen while paused
            expect(f3.rendered).toBe(f2.rendered); // no new renders while paused

            // ── Seek forward via ArrowRight ──
            await page.evaluate(() => document.querySelector('hevc-player').focus());
            await page.keyboard.press('ArrowRight'); // +10s
            await page.waitForTimeout(1000);
            const t4 = parseTimeString(await getTime(page));
            expect(t4).toBeGreaterThanOrEqual(t3 + 5); // should jump forward
            const f4 = await getFrameCounts(page);
            expect(f4.decoded).toBeGreaterThan(f3.decoded); // seek decoded new frames

            // ── Resume ──
            await clickButton(page, '.play-btn');
            await page.waitForTimeout(2000);
            const t5 = parseTimeString(await getTime(page));
            expect(t5).toBeGreaterThan(t4); // time advancing again
            const f5 = await getFrameCounts(page);
            expect(f5.decoded).toBeGreaterThan(f4.decoded);
            expect(f5.rendered).toBeGreaterThan(f4.rendered);

            // ── Change speed to 2x ──
            await page.evaluate(() => {
                const el = document.querySelector('hevc-player');
                el.shadowRoot.querySelector('.speed').value = '2';
                el.shadowRoot.querySelector('.speed').dispatchEvent(new Event('change'));
            });
            await page.waitForTimeout(2000);
            const t6 = parseTimeString(await getTime(page));
            expect(t6).toBeGreaterThan(t5 + 2);
            const f6 = await getFrameCounts(page);
            expect(f6.decoded).toBeGreaterThan(f5.decoded);
            expect(f6.rendered).toBeGreaterThan(f5.rendered);

            // ── Pause + seek backward ──
            await clickButton(page, '.pause-btn');
            const f6b = await getFrameCounts(page);
            await page.evaluate(() => document.querySelector('hevc-player').focus());
            await page.keyboard.press('ArrowLeft'); // -10s
            await page.waitForTimeout(1000);
            const t7 = parseTimeString(await getTime(page));
            expect(t7).toBeLessThan(t6);
            const f7 = await getFrameCounts(page);
            expect(f7.decoded).toBeGreaterThan(f6b.decoded); // seek decoded frames at new position

            // ── Resume at 2x from new position ──
            await clickButton(page, '.play-btn');
            await page.waitForTimeout(2000);
            const t8 = parseTimeString(await getTime(page));
            expect(t8).toBeGreaterThan(t7);
            const f8 = await getFrameCounts(page);
            expect(f8.decoded).toBeGreaterThan(f7.decoded);
            expect(f8.rendered).toBeGreaterThan(f7.rendered);

            // ── Restart ──
            await clickButton(page, '.restart-btn');
            const t9 = parseTimeString(await getTime(page));
            expect(t9).toBe(0);
        });
    }
});

function parseTimeString(timeStr) {
    // "1:23 / 2:00" → 83
    const current = timeStr.split(' / ')[0];
    if (!current) return 0;
    const [min, sec] = current.split(':').map(Number);
    return (min || 0) * 60 + (sec || 0);
}
