import wasmInit, { Demuxer } from './pkg/videoplayer.js';
import { WebGLRenderer } from './webgl-renderer.js';

const dropZone = document.getElementById('drop-zone');
const fileInput = document.getElementById('file-input');
const status = document.getElementById('status');
const playerContainer = document.getElementById('player-container');
const canvas = document.getElementById('canvas');
const playBtn = document.getElementById('play-btn');
const pauseBtn = document.getElementById('pause-btn');
const restartBtn = document.getElementById('restart-btn');
const timeDisplay = document.getElementById('time');
const fpsDisplay = document.getElementById('fps');
const speedSelect = document.getElementById('speed');

let player = null;

class HEVCPlayer {
    constructor(canvas) {
        this.canvas = canvas;
        this.renderer = new WebGLRenderer(canvas);
        this.demuxer = null;
        this.worker = null;
        this.decodedFrames = []; // { pts, imageData }
        this.playing = false;
        this.startTime = 0;
        this.pauseOffset = 0;
        this.nextSample = 0;
        this.totalSamples = 0;
        this.durationMs = 0;
        this.nalLengthSize = 4;
        this.rafId = null;
        this.width = 0;
        this.height = 0;
        this.playbackSpeed = 1.0;
        this.pendingDecodes = 0;
        this.flushed = false;
        this.maxBuffer = 30;
    }

    async load(arrayBuffer) {
        status.textContent = 'Initializing...';
        await wasmInit();

        status.textContent = 'Parsing MP4...';
        const data = new Uint8Array(arrayBuffer);
        this.demuxer = new Demuxer(data);

        this.width = this.demuxer.width();
        this.height = this.demuxer.height();
        this.canvas.width = this.width;
        this.canvas.height = this.height;
        this.totalSamples = this.demuxer.sample_count();
        this.durationMs = this.demuxer.duration_ms();
        this.nalLengthSize = this.demuxer.nal_length_size();

        status.textContent = `Video: ${this.width}x${this.height}, ${this.totalSamples} frames, ${(this.durationMs / 1000).toFixed(1)}s`;

        // Start decoder worker
        status.textContent += ' — Loading decoder...';
        this.worker = new Worker('./decode-worker.js', { type: 'module' });

        await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error('Decoder init timed out')), 10000);
            this.worker.onmessage = (e) => {
                if (e.data.type === 'ready') {
                    clearTimeout(timeout);
                    if (e.data.sharedMemory) this.sharedMemory = e.data.sharedMemory;
                    resolve();
                }
                if (e.data.type === 'error') {
                    console.error('[decoder]', e.data.msg);
                    // Don't reject on error, still wait for ready
                }
                if (e.data.type === 'log') console.log('[decoder]', e.data.msg);
            };
            this.worker.onerror = (e) => { clearTimeout(timeout); reject(e); };
            const hvcc = this.demuxer.codec_description();
            this.worker.postMessage({ type: 'init', hvcc });
        });

        // Set up frame handler
        this.worker.onmessage = (e) => this.onWorkerMessage(e.data);

        // Decode first frame as thumbnail
        status.textContent = status.textContent.replace(/ — .*/, '') + ' — Decoding thumbnail...';
        await this.decodeFirstFrame();

        // Reset decoder after thumbnail so playback starts clean from sample 0
        this.nextSample = 0;
        this.pendingDecodes = 0;
        this.decodedFrames = [];
        const hvcc2 = this.demuxer.codec_description();
        this.worker.postMessage({ type: 'reset', hvcc: hvcc2 });

        status.textContent = status.textContent.replace(/ — .*/, '') + ' — Ready';
    }

    async decodeFirstFrame() {
        return new Promise((resolve) => {
            const prevHandler = this.worker.onmessage;
            let sent = 0;
            const maxToSend = Math.min(30, this.totalSamples);

            const trySend = () => {
                while (sent < maxToSend && this.nextSample < this.totalSamples) {
                    const sample = this.demuxer.read_sample(this.nextSample);
                    this.nextSample++;
                    if (!sample) continue;
                    this.worker.postMessage({
                        type: 'samples',
                        samples: [{ data: sample.data, pts: 0 }],
                        nalLengthSize: this.nalLengthSize,
                    });
                    sent++;
                    break;
                }
            };

            const timeout = setTimeout(() => {
                console.warn('[thumbnail] timed out after sending', sent, 'samples');
                this.worker.onmessage = prevHandler;
                resolve();
            }, 10000);

            this.worker.onmessage = (e) => {
                const msg = e.data;
                if (msg.type === 'frame') {
                    this.renderer.render(msg.yData, msg.uData, msg.vData, msg.width, msg.height, 8);
                    clearTimeout(timeout);
                    this.worker.onmessage = prevHandler;
                    resolve();
                } else if (msg.type === 'decoded') {
                    this.pendingDecodes--;
                    if (msg.frames === 0) trySend();
                } else if (msg.type === 'log') {
                    console.log('[decoder]', msg.msg);
                } else if (msg.type === 'error') {
                    console.error('[decoder]', msg.msg);
                }
            };

            this.pendingDecodes = 0;
            trySend();
        });
    }

    onWorkerMessage(msg) {
        switch (msg.type) {
            case 'frame': {
                // Drop if buffer is full (back-pressure didn't catch it)
                if (this.decodedFrames.length >= 50) break;

                const frame = {
                    pts: msg.pts,
                    yData: msg.yData, uData: msg.uData, vData: msg.vData,
                    width: msg.width, height: msg.height,
                };
                // Binary insert sorted by PTS (decoder doesn't fully reorder)
                let lo = 0, hi = this.decodedFrames.length;
                while (lo < hi) {
                    const mid = (lo + hi) >> 1;
                    if (this.decodedFrames[mid].pts < frame.pts) lo = mid + 1;
                    else hi = mid;
                }
                this.decodedFrames.splice(lo, 0, frame);
                break;
            }
            case 'decoded':
                this.pendingDecodes -= msg.count;
                if (msg.avgMs) console.log(`[perf] ${msg.count} samples, ${msg.frames} frames, avg ${msg.avgMs}ms/sample`);
                if (this.playing) this.feedWorker();
                break;
            case 'flushed':
                this.flushed = true;
                break;
            case 'log':
                console.log('[decoder]', msg.msg);
                break;
            case 'error':
                console.error('[decoder]', msg.msg);
                status.textContent = `Decoder: ${msg.msg}`;
                break;
        }
    }

    feedWorker() {
        // Send batches of samples to the worker to keep it busy
        // but not overwhelm it
        if (this.pendingDecodes > 10) return;
        if (this.decodedFrames.length > 30) return; // back-pressure: don't decode too far ahead
        if (this.nextSample >= this.totalSamples) {
            if (!this.flushed && this.pendingDecodes === 0) {
                this.worker.postMessage({ type: 'flush' });
            }
            return;
        }

        const batchSize = Math.min(10, this.totalSamples - this.nextSample);
        const samples = [];

        for (let i = 0; i < batchSize; i++) {
            const sample = this.demuxer.read_sample(this.nextSample);
            this.nextSample++;
            if (!sample) continue;
            samples.push({
                data: sample.data,
                pts: Math.round(sample.timestamp_us),
            });
        }

        if (samples.length > 0) {
            this.pendingDecodes += samples.length;
            this.worker.postMessage(
                { type: 'samples', samples, nalLengthSize: this.nalLengthSize }
            );
        }
    }

    play() {
        if (this.playing) return;
        this.playing = true;
        this.startTime = performance.now() - this.pauseOffset;
        this.feedWorker();
        this.renderLoop();
    }

    pause() {
        this.playing = false;
        this.pauseOffset = performance.now() - this.startTime;
        if (this.rafId) {
            cancelAnimationFrame(this.rafId);
            this.rafId = null;
        }
    }

    restart() {
        this.pause();
        this.decodedFrames = [];
        this.nextSample = 0;
        this.pauseOffset = 0;
        this.pendingDecodes = 0;
        this.flushed = false;
        this._lastShownPts = 0;

        const hvcc = this.demuxer.codec_description();
        this.worker.postMessage({ type: 'reset', hvcc });

        const gl = this.renderer?.gl;
        if (gl) { gl.clearColor(0,0,0,1); gl.clear(gl.COLOR_BUFFER_BIT); }
        this.updateTime(0);
    }

    renderLoop() {
        if (!this.playing) return;

        this.feedWorker();

        const elapsedUs = (performance.now() - this.startTime) * 1000 * this.playbackSpeed;
        const MIN_REORDER = 3;

        // Consume frames up to current time, keeping reorder margin
        let frameToShow = null;
        let skipped = 0;
        while (this.decodedFrames.length > MIN_REORDER || this.flushed) {
            const f = this.decodedFrames[0];
            if (!f || f.pts > elapsedUs) break;
            const candidate = this.decodedFrames.shift();
            // Never go backward
            if (candidate.pts >= (this._lastShownPts || 0)) {
                if (frameToShow) skipped++;
                frameToShow = candidate;
                this._lastShownPts = candidate.pts;
            } else {
                skipped++;
            }
        }

        if (frameToShow) {
            this.renderFrame(frameToShow);
            // FPS tracking
            const now = performance.now();
            this._fpsFrames = (this._fpsFrames || 0) + 1;
            if (!this._fpsTime) this._fpsTime = now;
            if (now - this._fpsTime >= 1000) {
                fpsDisplay.textContent = `${this._fpsFrames} fps`;
                this._fpsFrames = 0;
                this._fpsTime = now;
            }
        }

        this.updateTime(elapsedUs / 1000);

        const done = this.flushed && this.decodedFrames.length === 0;

        if (done) {
            this.playing = false;
            status.textContent = status.textContent.replace(/ — .*/, '') + ' — Finished';
        } else {
            this.rafId = requestAnimationFrame(() => this.renderLoop());
        }
    }

    updateTime(elapsedMs) {
        const cur = Math.min(elapsedMs / 1000, this.durationMs / 1000);
        const tot = this.durationMs / 1000;
        timeDisplay.textContent = `${fmtTime(cur)} / ${fmtTime(tot)}`;
    }

    setSpeed(speed) {
        if (this.playing) {
            // Re-anchor so position doesn't jump
            const now = performance.now();
            const elapsedUs = (now - this.startTime) * 1000 * this.playbackSpeed;
            this.playbackSpeed = speed;
            this.startTime = now - elapsedUs / (speed * 1000);
        } else {
            const elapsedUs = this.pauseOffset * 1000 * this.playbackSpeed;
            this.playbackSpeed = speed;
            this.pauseOffset = elapsedUs / (speed * 1000);
        }
    }

    renderFrame(f) {
        this.renderer.render(f.yData, f.uData, f.vData, f.width, f.height, f.bpp);
    }

    destroy() {
        this.pause();
        this.decodedFrames = [];
        if (this.worker) {
            this.worker.terminate();
            this.worker = null;
        }
        if (this.renderer) {
            this.renderer.destroy();
            this.renderer = null;
        }
    }
}

function fmtTime(sec) {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
}

async function loadFile(file) {
    if (player) player.destroy();

    dropZone.classList.add('loading');
    status.textContent = 'Loading file...';
    playerContainer.classList.add('visible');

    try {
        const buf = await file.arrayBuffer();
        player = new HEVCPlayer(canvas);
        await player.load(buf);
        dropZone.classList.remove('loading');
        dropZone.classList.add('hidden');
    } catch (e) {
        dropZone.classList.remove('loading');
        status.textContent = `Error: ${e.message || e}`;
        console.error(e);
    }
}

// Load sample file
async function loadUrl(url) {
    if (player) player.destroy();

    dropZone.classList.add('loading');
    status.textContent = 'Fetching file...';
    playerContainer.classList.add('visible');

    try {
        const resp = await fetch(url);
        if (!resp.ok) throw new Error(`Fetch failed: ${resp.status}`);
        const buf = await resp.arrayBuffer();
        player = new HEVCPlayer(canvas);
        await player.load(buf);
        dropZone.classList.remove('loading');
        dropZone.classList.add('hidden');
        document.getElementById('load-sample')?.classList.add('hidden');
    } catch (e) {
        dropZone.classList.remove('loading');
        status.textContent = `Error: ${e.message || e}`;
        console.error(e);
    }
}

document.getElementById('load-sample')?.addEventListener('click', () => {
    loadUrl('./data/hellmode12_2m.mp4');
});

// File input (label[for] handles the click natively)
fileInput.addEventListener('change', (e) => {
    if (e.target.files[0]) loadFile(e.target.files[0]);
});

// Drag and drop on the whole document (label elements don't handle drop well)
document.addEventListener('dragover', (e) => {
    e.preventDefault();
    if (dropZone) dropZone.classList.add('drag-over');
});
document.addEventListener('dragleave', (e) => {
    if (!e.relatedTarget || e.relatedTarget === document.documentElement) {
        dropZone.classList.remove('drag-over');
    }
});
document.addEventListener('drop', (e) => {
    e.preventDefault();
    dropZone.classList.remove('drag-over');
    const file = e.dataTransfer?.files[0];
    if (file) loadFile(file);
});

// Controls
playBtn.addEventListener('click', () => player?.play());
pauseBtn.addEventListener('click', () => player?.pause());
restartBtn.addEventListener('click', () => player?.restart());

// Speed control
speedSelect.addEventListener('change', () => {
    player?.setSpeed(parseFloat(speedSelect.value));
});

// Keyboard
document.addEventListener('keydown', (e) => {
    if (e.code === 'Space') {
        e.preventDefault();
        if (player?.playing) player.pause();
        else player?.play();
    }
});
